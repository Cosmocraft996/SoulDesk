//! 显式G3探针：采样本次宿主及其WebView2后代，不计其他应用。
use crate::store::{err, AppResult};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[cfg(windows)]
pub fn processes(known: &mut BTreeMap<u32, u64>) -> AppResult<Vec<Value>> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            },
            ProcessStatus::{
                K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
            },
            Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
        },
    };
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(err(std::io::Error::last_os_error()));
    }
    let snapshot = Handle(snapshot);
    let mut entry = PROCESSENTRY32W::default();
    entry.dwSize = std::mem::size_of_val(&entry) as u32;
    if unsafe { Process32FirstW(snapshot.0, &mut entry) } == 0 {
        return Err(err(std::io::Error::last_os_error()));
    }
    let mut entries = Vec::new();
    loop {
        let end = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
        if entry.th32ProcessID == std::process::id()
            || name.eq_ignore_ascii_case("msedgewebview2.exe")
        {
            entries.push((entry.th32ProcessID, entry.th32ParentProcessID, name));
        }
        if unsafe { Process32NextW(snapshot.0, &mut entry) } == 0 {
            break;
        }
    }
    let ticks = |v: FILETIME| ((v.dwHighDateTime as u64) << 32) | v.dwLowDateTime as u64;
    let mut selected = BTreeMap::new();
    // 层序读取；创建时间排除父PID已复用的旧进程。
    for _ in 0..entries.len() {
        let before = selected.len();
        for (pid, parent, name) in &entries {
            if selected.contains_key(pid)
                || (*pid != std::process::id()
                    && !selected.contains_key(parent)
                    && !known.contains_key(pid))
            {
                continue;
            }
            let handle =
                unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, *pid) };
            if handle.is_null() {
                return Err(format!(
                    "G3无法完整读取进程{pid}：{}",
                    std::io::Error::last_os_error()
                ));
            }
            let handle = Handle(handle);
            let (mut created, mut exited, mut kernel, mut user) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            if unsafe {
                GetProcessTimes(handle.0, &mut created, &mut exited, &mut kernel, &mut user)
            } == 0
            {
                return Err(err(std::io::Error::last_os_error()));
            }
            let created = ticks(created);
            if *pid != std::process::id()
                && !selected.contains_key(parent)
                && known.get(pid) != Some(&created)
            {
                continue;
            }
            if selected
                .get(parent)
                .is_some_and(|p: &Value| p["createdTicks"].as_u64().unwrap() > created)
            {
                continue;
            }
            let mut memory = PROCESS_MEMORY_COUNTERS_EX::default();
            memory.cb = std::mem::size_of_val(&memory) as u32;
            if unsafe {
                K32GetProcessMemoryInfo(
                    handle.0,
                    &mut memory as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
                    memory.cb,
                )
            } == 0
            {
                return Err(err(std::io::Error::last_os_error()));
            }
            selected.insert(*pid, json!({"pid":pid,"parent":parent,"name":name,"createdTicks":created,"cpuTicks":ticks(kernel)+ticks(user),"privateBytes":memory.PrivateUsage,"workingSetBytes":memory.WorkingSetSize}));
        }
        if selected.len() == before {
            break;
        }
    }
    if !selected.contains_key(&std::process::id()) {
        return Err("G3缺少宿主进程指标".into());
    }
    // 已观测的后代即使暂时失去父进程，也继续计数；PID复用须再次核对创建时间。
    *known = selected
        .iter()
        .map(|(pid, p)| (*pid, p["createdTicks"].as_u64().unwrap()))
        .collect();
    Ok(selected.into_values().collect())
}

#[cfg(not(windows))]
pub fn processes(_known: &mut BTreeMap<u32, u64>) -> AppResult<Vec<Value>> {
    Err("G3进程树探针仅支持Windows".into())
}

pub fn cpu_percent(before: &[Value], after: &[Value], elapsed_ms: u64) -> AppResult<f64> {
    if elapsed_ms == 0 {
        return Err("CPU采样间隔为零".into());
    }
    let mut delta = 0;
    for p in after {
        // 新建/退出进程不强行推算CPU；报告保留逐PID原值，稳态阶段另行汇总。
        if let Some(old) = before
            .iter()
            .find(|old| old["pid"] == p["pid"] && old["createdTicks"] == p["createdTicks"])
        {
            delta += p["cpuTicks"]
                .as_u64()
                .ok_or("CPU数据缺失")?
                .checked_sub(old["cpuTicks"].as_u64().ok_or("CPU数据缺失")?)
                .ok_or("同一进程CPU累计值倒退")?;
        }
    }
    Ok(delta as f64 / (elapsed_ms as f64 * 10000.) * 100.)
}

pub fn summary(values: &[f64]) -> AppResult<Value> {
    if values.is_empty() || values.iter().any(|v| !v.is_finite()) {
        return Err("资源样本为空或无效".into());
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    Ok(
        json!({"count":n,"median":if n % 2 == 0 {(sorted[n/2-1]+sorted[n/2])/2.} else {sorted[n/2]},"peak":sorted[n-1],"first":values[0],"last":values[n-1],"growth":values[n-1]-values[0]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recycled_pid_is_not_counted_as_cpu_growth() {
        let a = vec![json!({"pid":1,"createdTicks":2,"cpuTicks":500000})];
        assert_eq!(
            cpu_percent(&a, &[json!({"pid":1,"createdTicks":3,"cpuTicks":1})], 1000).unwrap(),
            0.
        );
        assert_eq!(
            cpu_percent(
                &a,
                &[json!({"pid":1,"createdTicks":2,"cpuTicks":1500000})],
                1000
            )
            .unwrap(),
            10.
        );
        assert!(cpu_percent(&a, &[json!({"pid":1,"createdTicks":2,"cpuTicks":1})], 1000).is_err());
    }
    #[test]
    fn summary_keeps_trend_and_averages_even_median() {
        assert_eq!(
            summary(&[8., 2., 4., 6.]).unwrap(),
            json!({"count":4,"median":5.,"peak":8.,"first":8.,"last":6.,"growth":-2.})
        );
        assert!(summary(&[]).is_err());
    }
}
