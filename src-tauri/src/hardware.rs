//! 固定的本机占用率读取；不提供任意硬件查询、进程列表或命令执行。
use crate::{
    store::{err, now_ms, AppResult, Store},
    widgets::{self, Sessions},
    Core,
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

#[derive(Clone, Copy)]
struct CpuTimes {
    idle: u64,
    kernel: u64,
    user: u64,
}

fn cpu_percent(previous: CpuTimes, current: CpuTimes) -> Option<f64> {
    let idle = current.idle.checked_sub(previous.idle)?;
    let total = current
        .kernel
        .checked_sub(previous.kernel)?
        .checked_add(current.user.checked_sub(previous.user)?)?;
    if total == 0 || idle > total {
        return None;
    }
    Some((total - idle) as f64 / total as f64 * 100.0)
}
#[cfg(test)]
fn gpu_percent(rows: impl IntoIterator<Item = (String, f64)>) -> Option<f64> {
    gpu_reading(rows).map(|(percent, _)| percent)
}
fn gpu_reading(
    rows: impl IntoIterator<Item = (String, f64)>,
) -> Option<(f64, Option<crate::hardware_details::AdapterKey>)> {
    let mut engines = BTreeMap::<String, f64>::new();
    for (name, value) in rows {
        if !value.is_finite() || value < 0.0 || !name.starts_with("pid_") {
            continue;
        }
        let Some((_, key)) = name.split_once("_luid_") else {
            continue;
        };
        if !key.contains("_phys_") || !key.contains("_eng_") || !key.contains("_engtype_") {
            continue;
        }
        *engines.entry(key.to_owned()).or_default() += value;
    }
    engines
        .into_iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(key, value)| {
            (
                value.min(100.0),
                crate::hardware_details::AdapterKey::parse(&key),
            )
        })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Metric {
    percent: Option<f64>,
    error: Option<String>,
}
impl Metric {
    fn value(percent: Option<f64>, error: &str) -> Self {
        Self {
            percent,
            error: percent.is_none().then(|| error.to_owned()),
        }
    }
    fn failed(error: impl Into<String>) -> Self {
        Self {
            percent: None,
            error: Some(error.into()),
        }
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    sampled_at: u64,
    cpu: Metric,
    memory: Metric,
    gpu: Metric,
    details: Value,
}

#[derive(Default)]
pub struct Service(Mutex<Option<(Instant, platform::Sampler, Snapshot)>>);
impl Service {
    fn read(&self) -> AppResult<Value> {
        let mut state = self.0.lock().map_err(err)?;
        let now = Instant::now();
        if let Some((at, _, snapshot)) = state.as_ref() {
            if now.duration_since(*at) < Duration::from_secs(1) {
                return serde_json::to_value(snapshot).map_err(err);
            }
        }
        // 视图长时间离开后重新预热，避免将跨休眠/长间隔平均值当成当前占用率。
        let mut sampler = state
            .take()
            .filter(|(at, _, _)| now.duration_since(*at) < Duration::from_secs(10))
            .map(|(_, sampler, _)| sampler)
            .unwrap_or_default();
        let snapshot = sampler.sample();
        let value = serde_json::to_value(&snapshot).map_err(err)?;
        // 初始化驱动计数器可能较慢；缓存期限从完整采样完成时计算。
        *state = Some((Instant::now(), sampler, snapshot));
        Ok(value)
    }
    fn authorized_read(
        &self,
        store: &Store,
        sessions: &Sessions,
        window: &str,
        token: &str,
    ) -> AppResult<Value> {
        widgets::require_capability(store, sessions, window, token, "system.metrics")?;
        let value = self.read()?;
        widgets::require_capability(store, sessions, window, token, "system.metrics")?;
        Ok(value)
    }
}
pub async fn gateway(app: &AppHandle, window: &str, token: &str, args: Value) -> AppResult<Value> {
    if !args.is_null() && args.as_object().is_none_or(|map| !map.is_empty()) {
        return Err("CAPABILITY_DENIED: 本机占用率读取不接受路径、进程或其他参数".into());
    }
    let handle = app.clone();
    let label = window.to_owned();
    let token = token.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let core = handle.state::<Core>();
        core.hardware
            .authorized_read(&core.store, &core.sessions, &label, &token)
    })
    .await
    .map_err(err)?
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        mem::{align_of, size_of},
        ptr::{null, null_mut},
    };
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::{
            Performance::*,
            SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
            Threading::{GetActiveProcessorGroupCount, GetSystemTimes},
        },
    };

    // 句柄只在外层互斥锁中使用；保存整数避免向其他线程暴露裸指针。
    struct Gpu {
        query: usize,
        counter: usize,
        warm: bool,
        adapter: Option<crate::hardware_details::AdapterKey>,
    }
    impl Drop for Gpu {
        fn drop(&mut self) {
            unsafe {
                PdhCloseQuery(self.query as PDH_HQUERY);
            }
        }
    }
    impl Gpu {
        fn open() -> AppResult<Self> {
            let mut query = null_mut();
            let status = unsafe { PdhOpenQueryW(null(), 0, &mut query) };
            if status != 0 {
                return Err(format!("GPU性能查询不可用（{status:#x}）"));
            }
            let mut gpu = Self {
                query: query as usize,
                counter: 0,
                warm: false,
                adapter: None,
            };
            let path = "\\GPU Engine(*)\\Utilization Percentage"
                .encode_utf16()
                .chain([0])
                .collect::<Vec<_>>();
            let mut counter = null_mut();
            let status = unsafe { PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) };
            if status != 0 {
                return Err(format!("GPU驱动未提供占用率计数器（{status:#x}）"));
            }
            gpu.counter = counter as usize;
            Ok(gpu)
        }
        fn sample(&mut self) -> Metric {
            self.adapter = None;
            let status = unsafe { PdhCollectQueryData(self.query as PDH_HQUERY) };
            if status != 0 {
                self.warm = false;
                return Metric::failed(format!("GPU采样失败（{status:#x}）"));
            }
            if !self.warm {
                self.warm = true;
                return Metric::failed("GPU正在采样，请稍候");
            }
            match self.rows() {
                Ok(rows) => {
                    let reading = gpu_reading(rows);
                    self.adapter = reading.and_then(|(_, adapter)| adapter);
                    Metric::value(reading.map(|(percent, _)| percent), "GPU计数器没有有效读数")
                }
                Err(error) => Metric::failed(error),
            }
        }
        fn rows(&self) -> AppResult<Vec<(String, f64)>> {
            for _ in 0..3 {
                // 实例增减导致MORE_DATA时重新查询大小，不采用不足缓冲区返回的大小。
                let mut bytes = 0;
                let mut count = 0;
                let status = unsafe {
                    PdhGetFormattedCounterArrayW(
                        self.counter as PDH_HCOUNTER,
                        PDH_FMT_DOUBLE,
                        &mut bytes,
                        &mut count,
                        null_mut(),
                    )
                };
                if status != PDH_MORE_DATA {
                    return Err(format!("GPU读数不可用（{status:#x}）"));
                }
                if bytes == 0 || bytes > 4 * 1024 * 1024 {
                    return Err("GPU计数器数据超限".into());
                }
                // 系统结构含指针与double，使用u64分配保证Windows x64的对齐。
                assert!(align_of::<PDH_FMT_COUNTERVALUE_ITEM_W>() <= align_of::<u64>());
                let mut buffer = vec![0u64; (bytes as usize).div_ceil(size_of::<u64>())];
                let start = buffer.as_ptr() as usize;
                let end = start + buffer.len() * size_of::<u64>();
                let status = unsafe {
                    PdhGetFormattedCounterArrayW(
                        self.counter as PDH_HCOUNTER,
                        PDH_FMT_DOUBLE,
                        &mut bytes,
                        &mut count,
                        buffer.as_mut_ptr().cast(),
                    )
                };
                if status == PDH_MORE_DATA {
                    continue;
                }
                if status != 0 {
                    return Err(format!("GPU读数失败（{status:#x}）"));
                }
                if count as usize
                    > buffer.len() * size_of::<u64>() / size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>()
                {
                    return Err("GPU计数器项目超限".into());
                }
                let items = unsafe {
                    std::slice::from_raw_parts(
                        buffer.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(),
                        count as usize,
                    )
                };
                let mut rows = Vec::with_capacity(items.len());
                for item in items {
                    if ![PDH_CSTATUS_VALID_DATA, PDH_CSTATUS_NEW_DATA]
                        .contains(&item.FmtValue.CStatus)
                    {
                        continue;
                    }
                    let pointer = item.szName as usize;
                    if pointer < start || pointer >= end || pointer % align_of::<u16>() != 0 {
                        continue;
                    }
                    let units = unsafe {
                        std::slice::from_raw_parts(item.szName, ((end - pointer) / 2).min(512))
                    };
                    let Some(length) = units.iter().position(|unit| *unit == 0) else {
                        continue;
                    };
                    let Ok(name) = String::from_utf16(&units[..length]) else {
                        continue;
                    };
                    rows.push((name, unsafe { item.FmtValue.Anonymous.doubleValue }));
                }
                return Ok(rows);
            }
            Err("GPU计数器变化过快，请稍后重试".into())
        }
    }
    pub struct Sampler {
        previous: Option<CpuTimes>,
        gpu: AppResult<Gpu>,
        details: crate::hardware_details::Sampler,
    }
    impl Default for Sampler {
        fn default() -> Self {
            Self {
                previous: None,
                gpu: Gpu::open(),
                details: Default::default(),
            }
        }
    }
    fn ticks(time: FILETIME) -> u64 {
        (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
    }
    impl Sampler {
        pub fn sample(&mut self) -> Snapshot {
            let mut idle = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            let cpu = if unsafe { GetActiveProcessorGroupCount() } > 1 {
                Metric::failed("当前CPU读取器不支持多个处理器组")
            } else if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
                self.previous = None;
                Metric::failed("CPU系统时间读取失败")
            } else {
                let current = CpuTimes {
                    idle: ticks(idle),
                    kernel: ticks(kernel),
                    user: ticks(user),
                };
                let percent = self
                    .previous
                    .and_then(|previous| cpu_percent(previous, current));
                self.previous = Some(current);
                Metric::value(percent, "CPU正在采样或计数器已重置")
            };
            let mut memory = MEMORYSTATUSEX {
                dwLength: size_of::<MEMORYSTATUSEX>() as u32,
                ..Default::default()
            };
            let memory_ok = unsafe { GlobalMemoryStatusEx(&mut memory) } != 0;
            let memory_total = memory_ok.then_some(memory.ullTotalPhys);
            let memory = if memory_ok && memory.dwMemoryLoad <= 100 {
                Metric::value(Some(f64::from(memory.dwMemoryLoad)), "")
            } else {
                Metric::failed("物理内存占用率读取失败")
            };
            let gpu = match &mut self.gpu {
                Ok(gpu) => gpu.sample(),
                Err(error) => Metric::failed(error.clone()),
            };
            let details = self.details.sample(
                memory_total,
                self.gpu.as_ref().ok().and_then(|gpu| gpu.adapter),
            );
            Snapshot {
                sampled_at: now_ms(),
                cpu,
                memory,
                gpu,
                details,
            }
        }
    }
}
#[cfg(not(windows))]
mod platform {
    use super::*;
    #[derive(Default)]
    pub struct Sampler;
    impl Sampler {
        pub fn sample(&mut self) -> Snapshot {
            Snapshot {
                sampled_at: now_ms(),
                cpu: Metric::failed("当前平台不支持"),
                memory: Metric::failed("当前平台不支持"),
                gpu: Metric::failed("当前平台不支持"),
                details: crate::hardware_details::Sampler::default().sample(None, None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formal_capability_is_required_before_any_system_sampling_and_settings_are_separate() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-metrics-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let staged = widgets::stage_bytes(
            &store,
            crate::presets::find("local.souldesk.widgetjs.base-panel")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        let defaults = widgets::initial_settings(&staged.manifest);
        widgets::activate(&store, &staged).unwrap();
        let first = widgets::create_instance(&store, "local.souldesk.widgetjs.base-panel").unwrap();
        let second =
            widgets::create_instance(&store, "local.souldesk.widgetjs.base-panel").unwrap();
        let sessions = Sessions::default();
        let service = Service::default();
        let label = format!("float-{first}");
        let session =
            widgets::open_display_session(&store, &sessions, &label, &first, None).unwrap();
        let token = session["token"].as_str().unwrap();
        assert!(service
            .authorized_read(&store, &sessions, "other-window", token)
            .is_err());
        assert!(service.0.lock().unwrap().is_none());
        sessions.0.lock().unwrap().insert(
            "preview".into(),
            widgets::Session {
                instance: first.clone(),
                package_digest: None,
                window: "trial-metrics".into(),
                shared_cursor: None,
                preview: Some(std::sync::Arc::new(Mutex::new(widgets::Preview {
                    package: staged,
                    settings: defaults.clone(),
                    storage: json!({}),
                    ready_since: None,
                    failure: None,
                }))),
            },
        );
        assert!(service
            .authorized_read(&store, &sessions, "trial-metrics", "preview")
            .is_err());
        assert!(service.0.lock().unwrap().is_none());
        let denied = widgets::stage_bytes(
            &store,
            crate::presets::find("local.fixture.surface")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        widgets::activate(&store, &denied).unwrap();
        let no_cap = widgets::create_instance(&store, "local.fixture.surface").unwrap();
        let no_label = format!("float-{no_cap}");
        let no_session =
            widgets::open_display_session(&store, &sessions, &no_label, &no_cap, None).unwrap();
        assert!(service
            .authorized_read(
                &store,
                &sessions,
                &no_label,
                no_session["token"].as_str().unwrap()
            )
            .is_err());
        assert!(service.0.lock().unwrap().is_none());
        assert!(service
            .authorized_read(&store, &sessions, &label, token)
            .unwrap()["sampledAt"]
            .as_u64()
            .is_some());
        let mut next = defaults.clone();
        next["fontSize"] = json!(18);
        widgets::gateway(
            &store,
            &sessions,
            &label,
            token,
            "settings.update",
            next.clone(),
        )
        .unwrap();
        assert_eq!(
            widgets::gateway(
                &store,
                &sessions,
                &label,
                token,
                "settings.get",
                Value::Null
            )
            .unwrap(),
            next
        );
        let other_label = format!("float-{second}");
        let other_session =
            widgets::open_display_session(&store, &sessions, &other_label, &second, None).unwrap();
        assert_eq!(
            widgets::gateway(
                &store,
                &sessions,
                &other_label,
                other_session["token"].as_str().unwrap(),
                "settings.get",
                Value::Null
            )
            .unwrap(),
            defaults
        );
    }
    #[cfg(windows)]
    #[test]
    fn native_cpu_memory_and_gpu_have_real_values_or_explicit_unavailability() {
        let service = Service::default();
        let first = service.read().unwrap();
        assert!(first["cpu"]["percent"].is_null());
        assert!(first["gpu"]["percent"].is_null());
        assert_eq!(first, service.read().unwrap());
        std::thread::sleep(Duration::from_millis(1200));
        let second = service.read().unwrap();
        let details = &second["details"];
        assert!(details["cpuLogicalCount"]
            .as_u64()
            .is_some_and(|count| count > 0));
        assert!(details["memoryTotalBytes"]
            .as_str()
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|bytes| bytes > 0));
        if let Some(celsius) = details["gpuTemperature"]["celsius"].as_f64() {
            assert!(celsius > 0.0 && celsius <= 200.0);
            assert!(details["gpuAdapter"].as_str().is_some());
        } else {
            assert!(details["gpuTemperature"]["error"].as_str().is_some());
        }
        let network = &details["network"];
        for direction in ["downloadBytesPerSecond", "uploadBytesPerSecond"] {
            if let Some(speed) = network[direction].as_f64() {
                assert!(speed.is_finite() && speed >= 0.0);
            } else {
                assert!(network["error"].as_str().is_some());
            }
        }
        assert!(second["sampledAt"].as_u64().unwrap() > first["sampledAt"].as_u64().unwrap());
        assert!(second["memory"]["percent"]
            .as_f64()
            .is_some_and(|value| (0.0..=100.0).contains(&value)));
        for kind in ["cpu", "gpu"] {
            if let Some(value) = second[kind]["percent"].as_f64() {
                assert!((0.0..=100.0).contains(&value));
            } else {
                assert!(second[kind]["error"]
                    .as_str()
                    .is_some_and(|error| !error.is_empty()));
            }
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../artifacts/widgetjs-native-metrics-probe.json");
        std::fs::write(path,serde_json::to_vec_pretty(&json!({"scope":"Windows原生占用率采样；非WebView/桌面验收","first":first,"second":second})).unwrap()).unwrap();
    }
    #[test]
    fn energy_label_uses_formal_metrics_and_keeps_settings_independent_after_reopening_store() {
        let root = std::env::temp_dir().join(format!("souldesk-energy-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root.clone()).unwrap();
        let staged = widgets::stage_bytes(
            &store,
            crate::presets::find("local.souldesk.widgetjs.energy-label")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        let defaults = widgets::initial_settings(&staged.manifest);
        widgets::activate(&store, &staged).unwrap();
        let first =
            widgets::create_instance(&store, "local.souldesk.widgetjs.energy-label").unwrap();
        let second =
            widgets::create_instance(&store, "local.souldesk.widgetjs.energy-label").unwrap();
        let sessions = Sessions::default();
        let label = format!("float-{first}");
        let session =
            widgets::open_display_session(&store, &sessions, &label, &first, None).unwrap();
        let token = session["token"].as_str().unwrap();
        let mut next = defaults.clone();
        next["productor"] = json!("本机自用");
        next["spec"] = json!("日常-01");
        next["fontSize"] = json!(20);
        widgets::gateway(
            &store,
            &sessions,
            &label,
            token,
            "settings.update",
            next.clone(),
        )
        .unwrap();
        assert_eq!(
            widgets::gateway(
                &store,
                &sessions,
                &label,
                token,
                "settings.get",
                Value::Null
            )
            .unwrap(),
            next
        );
        let service = Service::default();
        let value = service
            .authorized_read(&store, &sessions, &label, token)
            .unwrap();
        assert!(value["sampledAt"].as_u64().is_some());
        let reopened = Store::open(root).unwrap().snapshot().unwrap();
        let instances = reopened["instances"].as_array().unwrap();
        assert_eq!(
            instances
                .iter()
                .find(|instance| instance["id"] == first)
                .unwrap()["settings"],
            next
        );
        assert_eq!(
            instances
                .iter()
                .find(|instance| instance["id"] == second)
                .unwrap()["settings"],
            defaults
        );
    }
    #[test]
    fn base_panel_has_formal_metrics_and_its_theme_is_independent() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-base-panel-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let staged = widgets::stage_bytes(
            &store,
            crate::presets::find("local.souldesk.widgetjs.base-panel")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        let defaults = widgets::initial_settings(&staged.manifest);
        widgets::activate(&store, &staged).unwrap();
        let first = widgets::create_instance(&store, "local.souldesk.widgetjs.base-panel").unwrap();
        let second =
            widgets::create_instance(&store, "local.souldesk.widgetjs.base-panel").unwrap();
        let sessions = Sessions::default();
        let label = format!("float-{first}");
        let session =
            widgets::open_display_session(&store, &sessions, &label, &first, None).unwrap();
        let token = session["token"].as_str().unwrap();
        assert_eq!(
            widgets::require_capability(&store, &sessions, &label, token, "system.metrics")
                .unwrap(),
            first
        );
        let mut next = defaults.clone();
        next["background"] = json!("#112233");
        next["fontSize"] = json!(18);
        widgets::gateway(
            &store,
            &sessions,
            &label,
            token,
            "settings.update",
            next.clone(),
        )
        .unwrap();
        assert_eq!(
            widgets::gateway(
                &store,
                &sessions,
                &label,
                token,
                "settings.get",
                Value::Null
            )
            .unwrap(),
            next
        );
        assert_eq!(
            store.snapshot().unwrap()["instances"]
                .as_array()
                .unwrap()
                .iter()
                .find(|instance| instance["id"] == second)
                .unwrap()["settings"],
            defaults
        );
    }
    #[test]
    fn cpu_uses_kernel_including_idle_and_rejects_counter_reset() {
        let before = CpuTimes {
            idle: 100,
            kernel: 200,
            user: 100,
        };
        assert_eq!(
            cpu_percent(
                before,
                CpuTimes {
                    idle: 130,
                    kernel: 240,
                    user: 120
                }
            ),
            Some(50.0)
        );
        assert_eq!(cpu_percent(before, before), None);
        assert_eq!(
            cpu_percent(
                before,
                CpuTimes {
                    idle: 90,
                    kernel: 240,
                    user: 120
                }
            ),
            None
        );
        assert_eq!(
            cpu_percent(
                before,
                CpuTimes {
                    idle: 200,
                    kernel: 240,
                    user: 120
                }
            ),
            None
        );
    }
    #[test]
    fn gpu_sums_processes_of_same_engine_then_uses_busiest_engine() {
        assert_eq!(
            gpu_percent([
                ("pid_1_luid_0x0_0x1_phys_0_eng_0_engtype_3D".into(), 25.0),
                ("pid_2_luid_0x0_0x1_phys_0_eng_0_engtype_3D".into(), 30.0),
                ("pid_3_luid_0x0_0x1_phys_0_eng_1_engtype_Copy".into(), 40.0),
                ("pid_4_luid_0x0_0x2_phys_0_eng_0_engtype_3D".into(), 10.0),
            ]),
            Some(55.0)
        );
        assert_eq!(gpu_percent([("_Total".into(), 99.0)]), None);
        assert_eq!(
            gpu_percent([("pid_1_luid_x_phys_0_eng_0_engtype_3D".into(), f64::NAN)]),
            None
        );
        assert_eq!(
            gpu_percent([("pid_1_luid_x_phys_0_eng_0_engtype_3D".into(), 0.0)]),
            Some(0.0)
        );
        assert_eq!(
            gpu_percent([
                ("pid_1_luid_x_phys_0_eng_0_engtype_3D".into(), 70.0),
                ("pid_2_luid_x_phys_0_eng_0_engtype_3D".into(), 70.0),
            ]),
            Some(100.0)
        );
    }
    #[test]
    fn gpu_temperature_identity_is_from_the_same_busiest_engine_as_utilization() {
        let (percent, adapter) = gpu_reading([
            (
                "pid_1_luid_0x00000000_0x00000001_phys_0_eng_0_engtype_3D".into(),
                20.0,
            ),
            (
                "pid_2_luid_0x00000000_0x00000002_phys_1_eng_0_engtype_3D".into(),
                30.0,
            ),
            (
                "pid_3_luid_0x00000000_0x00000002_phys_1_eng_0_engtype_3D".into(),
                40.0,
            ),
        ])
        .unwrap();
        assert_eq!(percent, 70.0);
        assert_eq!(
            adapter,
            Some(crate::hardware_details::AdapterKey {
                high: 0,
                low: 2,
                physical: 1
            })
        );
    }
}
