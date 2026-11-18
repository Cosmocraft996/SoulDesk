//! 固定喝水提醒后台，状态独立于组件视图；不开放任意脚本或通知文本。
use crate::{
    store::{err, now_ms, AppResult, Store},
    widgets, Core,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};
use uuid::Uuid;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    last_at: u64,
    pending: Option<Pending>,
    delivery: String,
    error: Option<String>,
    run_id: String,
    receipts: Vec<Receipt>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Receipt {
    id: String,
    operation: String,
}
fn due(
    state: &State,
    now: u64,
    enabled: bool,
    interval_minutes: u64,
    count: u32,
    target: u32,
) -> bool {
    enabled
        && count < target
        && now
            .checked_sub(state.last_at)
            .is_some_and(|elapsed| elapsed >= interval_minutes * 60000)
}
#[derive(Clone)]
struct Clock {
    now: u64,
    day: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Pending {
    id: String,
    day: String,
    expires_at: u64,
}
pub struct Service {
    busy: AtomicBool,
    run_id: String,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            busy: AtomicBool::new(false),
            run_id: Uuid::new_v4().to_string(),
        }
    }
}
struct Config {
    enabled: bool,
    interval: u64,
    target: u32,
}
fn config(db: &Connection, id: &str) -> AppResult<Config> {
    let (manifest,settings):(String,String)=db.query_row("SELECT p.manifest,i.settings FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE i.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
    let manifest: Value = serde_json::from_str(&manifest).map_err(err)?;
    let settings: Value = serde_json::from_str(&settings).map_err(err)?;
    if !manifest["capabilities"]
        .as_array()
        .is_some_and(|caps| caps.iter().any(|cap| cap == "reminders.water"))
    {
        return Err("CAPABILITY_DENIED: 未声明喝水提醒能力".into());
    }
    let enabled = settings["enableReminder"]
        .as_bool()
        .ok_or("喝水提醒开关无效")?;
    let interval = settings["interval"]
        .as_u64()
        .filter(|n| (5..=100).contains(n))
        .ok_or("喝水提醒间隔须为5～100分钟")?;
    let target = settings["targetCup"]
        .as_u64()
        .filter(|n| (1..=20).contains(n))
        .ok_or("喝水目标须为1～20杯")? as u32;
    Ok(Config {
        enabled,
        interval,
        target,
    })
}
fn state_key(id: &str) -> String {
    format!("water.state:{}", URL_SAFE_NO_PAD.encode(id))
}
fn history_prefix(id: &str) -> String {
    format!("water.history:{}:", URL_SAFE_NO_PAD.encode(id))
}
fn history_key(id: &str, day: &str) -> String {
    format!("{}{day}", history_prefix(id))
}
fn load(db: &Connection, id: &str) -> AppResult<Option<State>> {
    db.query_row(
        "SELECT value FROM app_settings WHERE key=?1",
        [state_key(id)],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(err)?
    .map(|v| serde_json::from_str(&v).map_err(err))
    .transpose()
}
fn put(db: &Connection, key: &str, value: &str) -> AppResult<()> {
    db.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE app_settings.value<>excluded.value",params![key,value]).map_err(err)?;
    Ok(())
}
fn save(db: &Connection, id: &str, state: &State) -> AppResult<()> {
    put(
        db,
        &state_key(id),
        &serde_json::to_string(state).map_err(err)?,
    )
}
fn count(db: &Connection, id: &str, day: &str) -> AppResult<u32> {
    db.query_row(
        "SELECT value FROM app_settings WHERE key=?1",
        [history_key(id, day)],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(err)?
    .map(|v| v.parse().map_err(err))
    .transpose()
    .map(|v| v.unwrap_or(0))
}
fn add(db: &Connection, id: &str, day: &str, previous: u32) -> AppResult<()> {
    put(
        db,
        &history_key(id, day),
        &previous
            .checked_add(1)
            .ok_or("杯数超出支持范围")?
            .to_string(),
    )
}
fn access(store: &Store, id: &str, at: &Clock, op: Option<(&str, &str)>) -> AppResult<Value> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    let config = config(&tx, id)?;
    let mut state = load(&tx, id)?.unwrap_or_else(|| State {
        last_at: at.now,
        ..Default::default()
    });
    if let Some((request, operation)) = op {
        Uuid::parse_str(request)
            .map_err(|_| "INVALID_ARGUMENT: 喝水操作需要UUID请求标识".to_owned())?;
        if !["add", "reset"].contains(&operation) {
            return Err("INVALID_ARGUMENT: 未知喝水操作".into());
        }
        if let Some(receipt) = state.receipts.iter().find(|r| r.id == request) {
            if receipt.operation != operation {
                return Err("INVALID_ARGUMENT: 请求标识已用于其他操作".into());
            }
        } else {
            if operation == "add" {
                add(&tx, id, &at.day, count(&tx, id, &at.day)?)?;
            } else {
                put(&tx, &history_key(id, &at.day), "0")?;
            }
            state.receipts.push(Receipt {
                id: request.into(),
                operation: operation.into(),
            });
            if state.receipts.len() > 128 {
                state.receipts.remove(0);
            }
        }
    }
    let cups = count(&tx, id, &at.day)?;
    let value = json!({"sampledAt":at.now,"day":at.day,"count":cups,"targetCup":config.target,"enableReminder":config.enabled,"interval":config.interval,"lastReminderAt":state.last_at,"nextReminderAt":if config.enabled&&cups<config.target{state.last_at.checked_add(config.interval*60000)}else{None},"delivery":state.delivery,"lastError":state.error});
    save(&tx, id, &state)?;
    tx.commit().map_err(err)?;
    Ok(value)
}
impl Service {
    fn reserve(&self, store: &Store, id: &str, at: &Clock) -> AppResult<Option<Pending>> {
        let mut db = store.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let cfg = config(&tx, id)?;
        let Some(mut state) = load(&tx, id)? else {
            return Ok(None);
        };
        if state.run_id != self.run_id && state.delivery == "sending" {
            state.delivery = "unknown".into();
            state.error = Some("上次提醒递送结果未确定".into());
        }
        state.run_id = self.run_id.clone();
        let cups = count(&tx, id, &at.day)?;
        if state.last_at > at.now {
            state.last_at = at.now;
            state.pending = None;
        }
        if !cfg.enabled
            || cups >= cfg.target
            || state
                .pending
                .as_ref()
                .is_some_and(|p| p.day != at.day || at.now >= p.expires_at)
        {
            state.pending = None;
        }
        let pending = if due(&state, at.now, cfg.enabled, cfg.interval, cups, cfg.target) {
            let pending = Pending {
                id: Uuid::new_v4().to_string(),
                day: at.day.clone(),
                expires_at: at
                    .now
                    .checked_add(cfg.interval * 60000)
                    .ok_or("提醒时间超出支持范围")?,
            };
            state.last_at = at.now;
            state.pending = Some(pending.clone());
            state.delivery = "sending".into();
            state.error = None;
            Some(pending)
        } else {
            None
        };
        save(&tx, id, &state)?;
        tx.commit().map_err(err)?;
        Ok(pending)
    }
    pub fn dispatch(&self, app: &AppHandle) {
        if self.busy.swap(true, Ordering::AcqRel) {
            return;
        }
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            struct Release(AppHandle);
            impl Drop for Release {
                fn drop(&mut self) {
                    self.0
                        .state::<Core>()
                        .water
                        .busy
                        .store(false, Ordering::Release);
                }
            }
            let _release = Release(handle.clone());
            let core = handle.state::<Core>();
            let result = (|| -> AppResult<()> {
                let ids = {
                    let db = core.store.db.lock().map_err(err)?;
                    let mut query=db.prepare("SELECT i.id FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE instr(p.manifest,'\"reminders.water\"')>0").map_err(err)?;
                    let ids = query
                        .query_map([], |r| r.get::<_, String>(0))
                        .map_err(err)?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(err)?;
                    ids
                };
                if ids.is_empty() {
                    return Ok(());
                }
                let at = Clock::current()?;
                for id in ids {
                    if core.quitting.load(Ordering::Relaxed) {
                        break;
                    }
                    match core.water.reserve(&core.store, &id, &at) {
                        Ok(Some(pending)) => {
                            let result = notify(&handle, &id, &pending);
                            finish(&core.store, &id, &pending, result)?;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            core.report(format!("喝水提醒：{error}"));
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                core.report(format!("喝水提醒：{error}"));
            }
        });
    }
}
fn finish(store: &Store, id: &str, pending: &Pending, result: AppResult<()>) -> AppResult<()> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    let Some(mut state) = load(&tx, id)? else {
        return Ok(());
    };
    if state.pending.as_ref().is_some_and(|p| p.id == pending.id) && state.delivery == "sending" {
        state.delivery = if result.is_ok() { "accepted" } else { "failed" }.into();
        state.error = result.err();
        save(&tx, id, &state)?;
    }
    tx.commit().map_err(err)
}
fn acknowledge(
    store: &Store,
    id: &str,
    pending: &Pending,
    drink: bool,
    at: &Clock,
) -> AppResult<bool> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    let cfg = config(&tx, id)?;
    let Some(mut state) = load(&tx, id)? else {
        return Ok(false);
    };
    if !cfg.enabled
        || pending.day != at.day
        || at.now >= pending.expires_at
        || !state.pending.as_ref().is_some_and(|p| {
            p.id == pending.id && p.day == pending.day && p.expires_at == pending.expires_at
        })
    {
        return Ok(false);
    }
    if drink {
        add(&tx, id, &at.day, count(&tx, id, &at.day)?)?;
    }
    state.pending = None;
    save(&tx, id, &state)?;
    tx.commit().map_err(err)?;
    Ok(true)
}
pub fn remove_data(db: &Connection, id: &str) -> AppResult<()> {
    let prefix = history_prefix(id);
    db.execute(
        "DELETE FROM app_settings WHERE key=?1 OR substr(key,1,length(?2))=?2",
        params![state_key(id), prefix],
    )
    .map_err(err)?;
    Ok(())
}
fn authorized_access(
    store: &Store,
    sessions: &widgets::Sessions,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
    at: &Clock,
) -> AppResult<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Change {
        request_id: String,
        operation: String,
    }
    let change = match method {
        "water.get" if args.is_null() || args.as_object().is_some_and(|o| o.is_empty()) => None,
        "water.change" => Some(serde_json::from_value::<Change>(args).map_err(err)?),
        _ => return Err("INVALID_ARGUMENT: 喝水接口或参数无效".into()),
    };
    let id = widgets::require_capability(store, sessions, window, token, "reminders.water")?;
    access(
        store,
        &id,
        at,
        change
            .as_ref()
            .map(|c| (c.request_id.as_str(), c.operation.as_str())),
    )
}
pub async fn gateway(
    app: &AppHandle,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    let handle = app.clone();
    let window = window.to_owned();
    let token = token.to_owned();
    let method = method.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let core = handle.state::<Core>();
        authorized_access(
            &core.store,
            &core.sessions,
            &window,
            &token,
            &method,
            args,
            &Clock::current()?,
        )
    })
    .await
    .map_err(err)?
}
#[cfg(windows)]
fn notify(app: &AppHandle, id: &str, pending: &Pending) -> AppResult<()> {
    use tauri_winrt_notification::{Duration, Toast};
    let core = app.state::<Core>();
    let store = std::sync::Arc::downgrade(&core.store);
    let last_error = std::sync::Arc::downgrade(&core.last_error);
    let id = id.to_owned();
    let pending = pending.clone();
    Toast::new(&app.config().identifier)
        .title("喝水提醒")
        .text1("起来喝杯水吧！")
        .duration(Duration::Short)
        .add_button("关闭", "close")
        .add_button("喝一杯", "drink")
        .on_activated(move |action| {
            if let (Some(store), Some(action)) = (store.upgrade(), action) {
                if ["drink", "close"].contains(&action.as_str()) {
                    if let Err(error) = Clock::current()
                        .and_then(|at| acknowledge(&store, &id, &pending, action == "drink", &at))
                    {
                        if let Some(slot) = last_error.upgrade() {
                            if let Ok(mut value) = slot.lock() {
                                *value = Some(format!("喝水提醒回执失败：{error}"));
                            }
                        }
                    }
                }
            }
            Ok(())
        })
        .show()
        .map_err(err)
}
#[cfg(not(windows))]
fn notify(_app: &AppHandle, _id: &str, _pending: &Pending) -> AppResult<()> {
    Err("当前平台未实现喝水系统通知".into())
}
impl Clock {
    #[cfg(windows)]
    fn current() -> AppResult<Self> {
        use windows_sys::Win32::{
            Foundation::{FILETIME, SYSTEMTIME},
            System::Time::{
                FileTimeToSystemTime, GetDynamicTimeZoneInformation,
                SystemTimeToTzSpecificLocalTimeEx, DYNAMIC_TIME_ZONE_INFORMATION,
                TIME_ZONE_ID_INVALID,
            },
        };
        let now = now_ms();
        let ticks = now
            .checked_add(11644473600000)
            .and_then(|n| n.checked_mul(10000))
            .filter(|n| *n < 0x8000000000000000)
            .ok_or("当前时间超出支持范围")?;
        let ft = FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
        let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
        let mut zone: DYNAMIC_TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { FileTimeToSystemTime(&ft, &mut utc) } == 0
            || unsafe { GetDynamicTimeZoneInformation(&mut zone) } == TIME_ZONE_ID_INVALID
            || unsafe { SystemTimeToTzSpecificLocalTimeEx(&zone, &utc, &mut local) } == 0
        {
            return Err("无法读取系统本地日期，未修改喝水记录".into());
        }
        Ok(Self {
            now,
            day: format!("{:04}-{:02}-{:02}", local.wYear, local.wMonth, local.wDay),
        })
    }
    #[cfg(not(windows))]
    fn current() -> AppResult<Self> {
        Err("当前平台未实现本地日期转换".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standard_water_package_enforces_formal_session_and_preview_boundaries() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-water-formal-{}", Uuid::new_v4())),
        )
        .unwrap();
        let staged = widgets::stage_bytes(
            &store,
            crate::presets::find("local.souldesk.widgetjs.water-reminder")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        let defaults = widgets::initial_settings(&staged.manifest);
        widgets::activate(&store, &staged).unwrap();
        let a = widgets::create_instance(&store, "local.souldesk.widgetjs.water-reminder").unwrap();
        let b = widgets::create_instance(&store, "local.souldesk.widgetjs.water-reminder").unwrap();
        let sessions = widgets::Sessions::default();
        let label = format!("float-{a}");
        let session = widgets::open_display_session(&store, &sessions, &label, &a, None).unwrap();
        let token = session["token"].as_str().unwrap();
        let at = clock(1000, "2026-11-15");
        sessions.0.lock().unwrap().insert(
            "preview".into(),
            widgets::Session {
                instance: a.clone(),
                window: "trial-water".into(),
                shared_cursor: None,
                preview: Some(std::sync::Arc::new(std::sync::Mutex::new(
                    widgets::Preview {
                        package: staged,
                        settings: defaults,
                        storage: json!({}),
                        ready_since: None,
                        failure: None,
                    },
                ))),
            },
        );
        assert!(authorized_access(
            &store,
            &sessions,
            "trial-water",
            "preview",
            "water.get",
            Value::Null,
            &at
        )
        .is_err());
        assert!(load(&store.db.lock().unwrap(), &a).unwrap().is_none());
        assert!(authorized_access(
            &store,
            &sessions,
            "other-window",
            token,
            "water.get",
            Value::Null,
            &at
        )
        .is_err());
        assert_eq!(
            authorized_access(
                &store,
                &sessions,
                &label,
                token,
                "water.get",
                Value::Null,
                &at
            )
            .unwrap()["count"],
            0
        );
        let request = Uuid::new_v4().to_string();
        assert!(authorized_access(
            &store,
            &sessions,
            &label,
            token,
            "water.change",
            json!({"requestId":request,"operation":"add","instance":b}),
            &at
        )
        .is_err());
        assert_eq!(
            authorized_access(
                &store,
                &sessions,
                &label,
                token,
                "water.change",
                json!({"requestId":request,"operation":"add"}),
                &at
            )
            .unwrap()["count"],
            1
        );
        assert_eq!(access(&store, &b, &at, None).unwrap()["count"], 0);
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
        let other = widgets::create_instance(&store, "local.fixture.surface").unwrap();
        let other_label = format!("float-{other}");
        let other_session =
            widgets::open_display_session(&store, &sessions, &other_label, &other, None).unwrap();
        assert!(authorized_access(
            &store,
            &sessions,
            &other_label,
            other_session["token"].as_str().unwrap(),
            "water.get",
            Value::Null,
            &at
        )
        .is_err());
        #[cfg(windows)]
        {
            let actual = Clock::current().unwrap();
            assert_eq!(actual.day.len(), 10);
            assert!(now_ms().saturating_sub(actual.now) < 5000);
        }
    }
    fn fixture() -> (Store, String, String) {
        let store =
            Store::open(std::env::temp_dir().join(format!("souldesk-water-{}", Uuid::new_v4())))
                .unwrap();
        // 固定业务测试不创建视图，也不发送真实通知。
        let manifest = json!({"schemaVersion":1,"id":"local.test.water","name":"喝水测试","version":"0.1.0","hostApiMajor":1,"entry":"dist/index.html","defaultSize":{"width":240,"height":240},"minSize":{"width":155,"height":155},"capabilities":["storage.instance","reminders.water"],"settingsSchema":{"type":"object","additionalProperties":false,"required":["enableReminder","interval","targetCup"],"properties":{"enableReminder":{"type":"boolean","default":true},"interval":{"type":"integer","minimum":5,"maximum":100,"default":5},"targetCup":{"type":"integer","minimum":1,"maximum":20,"default":8}}}});
        store.db.lock().unwrap().execute("INSERT INTO widget_packages(id,manifest,directory,digest) VALUES(?1,?2,'test-water','test-water')",rusqlite::params!["local.test.water",manifest.to_string()]).unwrap();
        let a = crate::widgets::create_instance(&store, "local.test.water").unwrap();
        let b = crate::widgets::create_instance(&store, "local.test.water").unwrap();
        (store, a, b)
    }
    fn clock(now: u64, day: &str) -> Clock {
        Clock {
            now,
            day: day.into(),
        }
    }
    fn change(store: &Store, id: &str, at: &Clock, op: &str) -> Value {
        access(store, id, at, Some((&Uuid::new_v4().to_string(), op))).unwrap()
    }
    #[test]
    fn cups_are_persisted_isolated_and_retry_safe() {
        let (store, a, b) = fixture();
        let at = clock(1000, "2026-11-15");
        let request = Uuid::new_v4().to_string();
        assert_eq!(access(&store, &a, &at, None).unwrap()["count"], 0);
        assert_eq!(
            access(&store, &a, &at, Some((&request, "add"))).unwrap()["count"],
            1
        );
        assert_eq!(
            access(&store, &a, &at, Some((&request, "add"))).unwrap()["count"],
            1
        );
        assert!(access(&store, &a, &at, Some((&request, "reset"))).is_err());
        assert_eq!(access(&store, &b, &at, None).unwrap()["count"], 0);
        let root = store.root.clone();
        drop(store);
        let reopened = Store::open(root).unwrap();
        assert_eq!(access(&reopened, &a, &at, None).unwrap()["count"], 1);
        assert_eq!(change(&reopened, &a, &at, "reset")["count"], 0);
        assert!(access(&reopened, &a, &at, Some(("invalid", "add"))).is_err());
    }
    #[test]
    fn idle_reads_and_ticks_do_not_rewrite_identical_persisted_state() {
        let (store, a, _) = fixture();
        let at = clock(1000, "2026-11-15");
        let service = Service::default();
        access(&store, &a, &at, None).unwrap();
        service.reserve(&store, &a, &at).unwrap();
        let changes = || {
            store
                .db
                .lock()
                .unwrap()
                .query_row("SELECT total_changes()", [], |r| r.get::<_, u64>(0))
                .unwrap()
        };
        let before = changes();
        access(&store, &a, &at, None).unwrap();
        service.reserve(&store, &a, &at).unwrap();
        access(&store, &a, &at, None).unwrap();
        service.reserve(&store, &a, &at).unwrap();
        assert_eq!(changes(), before, "没有状态变化的轮询不应持续写SQLite");
    }
    #[test]
    fn days_keep_history_and_late_retries_do_not_change_new_day() {
        let (store, a, _) = fixture();
        let first = clock(1000, "2026-11-15");
        let next = clock(2000, "2026-11-16");
        let request = Uuid::new_v4().to_string();
        access(&store, &a, &first, Some((&request, "add"))).unwrap();
        assert_eq!(
            access(&store, &a, &next, Some((&request, "add"))).unwrap()["count"],
            0
        );
        change(&store, &a, &next, "add");
        change(&store, &a, &next, "add");
        assert_eq!(access(&store, &a, &first, None).unwrap()["count"], 1);
        assert_eq!(access(&store, &a, &next, None).unwrap()["count"], 2);
    }
    #[test]
    fn hidden_views_keep_scheduling_without_duplicate_sends() {
        let (store, a, b) = fixture();
        let service = Service::default();
        let start = clock(1000, "2026-11-15");
        assert!(service.reserve(&store, &a, &start).unwrap().is_none());
        access(&store, &a, &start, None).unwrap();
        store.presentation(&a, "desktop", false, false).unwrap();
        let at = clock(301000, "2026-11-15");
        let pending = service
            .reserve(&store, &a, &at)
            .unwrap()
            .expect("隐藏后后台仍须到期");
        assert!(service.reserve(&store, &a, &at).unwrap().is_none());
        assert!(service.reserve(&store, &b, &at).unwrap().is_none());
        finish(&store, &a, &pending, Err("模拟系统通知失败".into())).unwrap();
        let snapshot = access(&store, &a, &at, None).unwrap();
        assert_eq!(snapshot["delivery"], "failed");
        assert!(snapshot["lastError"].as_str().unwrap().contains("失败"));
        assert!(service
            .reserve(&store, &a, &clock(601000, "2026-11-15"))
            .unwrap()
            .is_some());
    }
    #[test]
    fn actions_are_instance_scoped_current_day_and_exactly_once() {
        let (store, a, b) = fixture();
        let start = clock(1000, "2026-11-15");
        let at = clock(301000, "2026-11-15");
        let service = Service::default();
        access(&store, &a, &start, None).unwrap();
        access(&store, &b, &start, None).unwrap();
        let pending = service.reserve(&store, &a, &at).unwrap().unwrap();
        finish(&store, &a, &pending, Ok(())).unwrap();
        assert_eq!(
            access(&store, &a, &at, None).unwrap()["delivery"],
            "accepted"
        );
        assert!(!acknowledge(&store, &b, &pending, true, &at).unwrap());
        assert!(!acknowledge(&store, &a, &pending, true, &clock(301001, "2026-11-16")).unwrap());
        assert!(!acknowledge(&store, &a, &pending, true, &clock(601000, "2026-11-15")).unwrap());
        assert!(acknowledge(&store, &a, &pending, true, &at).unwrap());
        assert!(!acknowledge(&store, &a, &pending, true, &at).unwrap());
        assert_eq!(access(&store, &a, &at, None).unwrap()["count"], 1);
        let later = clock(601000, "2026-11-15");
        let second = service.reserve(&store, &a, &later).unwrap().unwrap();
        assert!(acknowledge(&store, &a, &second, false, &later).unwrap());
        assert_eq!(access(&store, &a, &later, None).unwrap()["count"], 1);
    }
    #[test]
    fn disable_target_clock_reversal_and_restart_do_not_create_bursts() {
        let (store, a, _) = fixture();
        let service = Service::default();
        access(&store, &a, &clock(1000, "2026-11-15"), None).unwrap();
        let at = clock(301000, "2026-11-15");
        service.reserve(&store, &a, &at).unwrap().unwrap();
        let restarted = Service::default();
        assert!(restarted.reserve(&store, &a, &at).unwrap().is_none());
        assert_eq!(
            access(&store, &a, &at, None).unwrap()["delivery"],
            "unknown"
        );
        assert!(restarted
            .reserve(&store, &a, &clock(500, "2026-11-15"))
            .unwrap()
            .is_none());
        assert_eq!(
            access(&store, &a, &clock(500, "2026-11-15"), None).unwrap()["lastReminderAt"],
            500
        );
        store.db.lock().unwrap().execute("UPDATE widget_instances SET settings=json_set(settings,'$.enableReminder',json('false')) WHERE id=?1",[&a]).unwrap();
        assert!(restarted
            .reserve(&store, &a, &clock(900000, "2026-11-15"))
            .unwrap()
            .is_none());
        store.db.lock().unwrap().execute("UPDATE widget_instances SET settings=json_set(settings,'$.enableReminder',json('true'),'$.targetCup',1) WHERE id=?1",[&a]).unwrap();
        change(&store, &a, &at, "add");
        assert!(restarted
            .reserve(&store, &a, &clock(900000, "2026-11-15"))
            .unwrap()
            .is_none());
    }
    #[test]
    fn removal_cleans_only_own_history_and_old_actions_cannot_revive_it() {
        let (store, a, b) = fixture();
        let start = clock(1000, "2026-11-15");
        let at = clock(301000, "2026-11-15");
        change(&store, &a, &start, "add");
        change(&store, &b, &start, "add");
        let pending = Service::default()
            .reserve(&store, &a, &at)
            .unwrap()
            .unwrap();
        crate::widgets::remove_instance(&store, &Default::default(), &a).unwrap();
        assert!(acknowledge(&store, &a, &pending, true, &at).is_err());
        assert_eq!(access(&store, &b, &at, None).unwrap()["count"], 1);
        let count: u32 = store
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM app_settings WHERE key LIKE 'water.%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }
    #[test]
    fn only_enabled_below_target_and_due_water_reminders_run() {
        let state = State {
            last_at: 1000,
            ..Default::default()
        };
        assert!(!due(&state, 300999, true, 5, 0, 8));
        assert!(due(&state, 301000, true, 5, 0, 8));
        assert!(!due(&state, 301000, false, 5, 0, 8));
        assert!(!due(&state, 301000, true, 5, 8, 8));
        assert!(!due(&state, 500, true, 5, 0, 8));
    }
}
