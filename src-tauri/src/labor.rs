//! 打工进度的固定下班倒计时，生命周期由核心持有。
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
// 无包身份的桌面程序按标签移除通知时必须使用非空分组；发送、更新和移除保持一致。
pub(crate) const NOTIFICATION_GROUP: &str = "souldesk-labor";
#[derive(Clone)]
struct Cycle {
    start: u64,
    end: u64,
    weekday: u32,
}
#[derive(Clone)]
struct Config {
    start: u32,
    end: u32,
}
#[derive(Clone, Debug, PartialEq)]
enum Action {
    Show {
        tag: String,
        remaining: u32,
    },
    Update {
        tag: String,
        remaining: u32,
        sequence: u32,
    },
    Remove {
        tag: String,
    },
}
#[derive(Clone, Debug)]
enum Delivery {
    Accepted,
    Missing,
}
#[derive(Default)]
pub struct Service {
    busy: AtomicBool,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    sent: Vec<u64>,
    target: u64,
    started: u64,
    tag: String,
    last_sequence: u32,
    delivery: String,
    error: Option<String>,
}
fn key(id: &str) -> String {
    format!("labor.state:{}", URL_SAFE_NO_PAD.encode(id))
}
fn load(db: &Connection, id: &str) -> AppResult<Option<State>> {
    db.query_row(
        "SELECT value FROM app_settings WHERE key=?1",
        [key(id)],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(err)?
    .map(|s| serde_json::from_str(&s).map_err(err))
    .transpose()
}
fn save(db: &Connection, id: &str, state: &State) -> AppResult<()> {
    db.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE app_settings.value<>excluded.value",params![key(id),serde_json::to_string(state).map_err(err)?]).map_err(err)?;
    Ok(())
}
fn config(db: &Connection, id: &str) -> AppResult<Config> {
    let(manifest,settings):(String,String)=db.query_row("SELECT p.manifest,i.settings FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE i.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
    let manifest: Value = serde_json::from_str(&manifest).map_err(err)?;
    let settings: Value = serde_json::from_str(&settings).map_err(err)?;
    if !manifest["capabilities"]
        .as_array()
        .is_some_and(|caps| caps.iter().any(|cap| cap == "reminders.labor"))
    {
        return Err("CAPABILITY_DENIED: 未声明打工提醒能力".into());
    }
    let number = |key: &str, max| {
        settings[key]
            .as_u64()
            .filter(|n| *n <= max)
            .map(|n| n as u32)
            .ok_or_else(|| "INVALID_ARGUMENT: 上下班时间无效".to_owned())
    };
    let start = number("startHour", 23)? * 60 + number("startMinute", 59)?;
    let end = number("endHour", 23)? * 60 + number("endMinute", 59)?;
    if start == end {
        return Err("INVALID_ARGUMENT: 上下班时间不能相同".into());
    }
    Ok(Config { start, end })
}
fn get(store: &Store, id: &str, cycle: &Cycle, now: u64) -> AppResult<Value> {
    if cycle.end <= cycle.start || !(1..=7).contains(&cycle.weekday) {
        return Err("DATA_UNAVAILABLE: 上下班时间区间无效".into());
    }
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    config(&tx, id)?;
    let state = load(&tx, id)?.unwrap_or_default();
    save(&tx, id, &state)?;
    let elapsed = now.saturating_sub(cycle.start).min(cycle.end - cycle.start);
    let percent = (u128::from(elapsed) * 1000 / u128::from(cycle.end - cycle.start)) as f64 / 10.;
    let value = json!({"sampledAt":now,"startAt":cycle.start,"endAt":cycle.end,"isoWeekday":cycle.weekday,"percent":percent,"remainingSeconds":cycle.end.saturating_sub(now).div_ceil(1000),"delivery":state.delivery,"lastError":state.error});
    tx.commit().map_err(err)?;
    Ok(value)
}
fn tick(
    store: &Store,
    id: &str,
    cycle: &Cycle,
    now: u64,
    send: &mut impl FnMut(Action) -> AppResult<Delivery>,
) -> AppResult<()> {
    let action = {
        let mut db = store.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        config(&tx, id)?;
        let Some(mut state) = load(&tx, id)? else {
            return Ok(());
        };
        let remaining = cycle
            .end
            .saturating_sub(now)
            .div_ceil(1000)
            .min(u32::MAX as u64) as u32;
        let channel_enabled = crate::reminder_store::Channel::Windows.enabled(&tx)?;
        let action = if state.delivery == "sending" {
            state.delivery = "unknown".into();
            state.error = Some("上次下班通知递送结果未确定".into());
            Some(Action::Remove {
                tag: state.tag.clone(),
            })
        } else if state.delivery == "accepted" {
            if !channel_enabled
                || state.target != cycle.end
                || now >= cycle.end
                || now < state.started
            {
                state.delivery = "stopped".into();
                Some(Action::Remove {
                    tag: state.tag.clone(),
                })
            } else {
                let sequence = ((now - state.started) / 1000 + 2).min(u32::MAX as u64) as u32;
                if sequence > state.last_sequence {
                    state.last_sequence = sequence;
                    Some(Action::Update {
                        tag: state.tag.clone(),
                        remaining,
                        sequence,
                    })
                } else {
                    None
                }
            }
        } else if !state.sent.contains(&cycle.end)
            && now >= cycle.start
            && remaining > 0
            && remaining <= 300
        {
            state.target = cycle.end;
            state.started = now;
            state.last_sequence = 1;
            state.tag = Uuid::new_v4().simple().to_string()[..16].to_owned();
            state.delivery = if channel_enabled {
                "sending"
            } else {
                "suppressed"
            }
            .into();
            state.error = None;
            state.sent.push(cycle.end);
            if state.sent.len() > 32 {
                state.sent.remove(0);
            }
            channel_enabled.then(|| Action::Show {
                tag: state.tag.clone(),
                remaining,
            })
        } else {
            None
        };
        save(&tx, id, &state)?;
        tx.commit().map_err(err)?;
        action
    };
    if let Some(action) = action {
        let result = send(action.clone());
        let mut db = store.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let Some(mut state) = load(&tx, id)? else {
            drop(tx);
            drop(db);
            if let Action::Show { tag, .. } | Action::Update { tag, .. } = action {
                let _ = send(Action::Remove { tag });
            }
            return Ok(());
        };
        match &action {
            Action::Show { tag, .. } if state.tag == *tag => match result {
                Ok(Delivery::Accepted) => state.delivery = "accepted".into(),
                Ok(Delivery::Missing) => state.delivery = "dismissed".into(),
                Err(error) => {
                    state.delivery = "failed".into();
                    state.error = Some(error);
                }
            },
            Action::Update { tag, .. } if state.tag == *tag => match result {
                Ok(Delivery::Accepted) => {}
                Ok(Delivery::Missing) => {
                    state.delivery = "dismissed".into();
                }
                Err(error) => {
                    state.delivery = "failed".into();
                    state.error = Some(error);
                }
            },
            Action::Remove { .. } => {
                if let Err(error) = result {
                    state.error = Some(error);
                }
            }
            _ => {}
        }
        save(&tx, id, &state)?;
        tx.commit().map_err(err)?;
    }
    Ok(())
}
pub fn remove_data(db: &Connection, id: &str) -> AppResult<()> {
    if let Some(state) = load(db, id)? {
        if !state.tag.is_empty() && ["accepted", "sending"].contains(&state.delivery.as_str()) {
            db.execute(
                "INSERT OR IGNORE INTO app_settings(key,value) VALUES(?1,'pending')",
                [format!("labor.cleanup:{}", state.tag)],
            )
            .map_err(err)?;
        }
    }
    db.execute("DELETE FROM app_settings WHERE key=?1", [key(id)])
        .map_err(err)?;
    Ok(())
}
impl Service {
    // 隔离验收等待原生通知操作完成；不向组件开放新的消息方法。
    pub(crate) fn is_idle(&self) -> bool {
        !self.busy.load(Ordering::Acquire)
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
                        .labor
                        .busy
                        .store(false, Ordering::Release);
                }
            }
            let _release = Release(handle.clone());
            let core = handle.state::<Core>();
            let result = (|| -> AppResult<()> {
                let (ids, cleanup) = {
                    let db = core.store.db.lock().map_err(err)?;
                    let mut query=db.prepare("SELECT i.id FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE instr(p.manifest,'\"reminders.labor\"')>0").map_err(err)?;
                    let ids = query
                        .query_map([], |r| r.get::<_, String>(0))
                        .map_err(err)?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(err)?;
                    let mut query = db
                        .prepare(
                            "SELECT key FROM app_settings WHERE substr(key,1,14)='labor.cleanup:'",
                        )
                        .map_err(err)?;
                    let cleanup = query
                        .query_map([], |r| r.get::<_, String>(0))
                        .map_err(err)?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(err)?;
                    (ids, cleanup)
                };
                for key in cleanup {
                    if let Some(tag) = key
                        .strip_prefix("labor.cleanup:")
                        .filter(|s| s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                    {
                        platform::send(
                            &handle.config().identifier,
                            Action::Remove { tag: tag.into() },
                        )?;
                        core.store
                            .db
                            .lock()
                            .map_err(err)?
                            .execute("DELETE FROM app_settings WHERE key=?1", [key])
                            .map_err(err)?;
                    }
                }
                let now = now_ms();
                for id in ids {
                    if core.quitting.load(Ordering::Relaxed) {
                        break;
                    }
                    let result = (|| {
                        let cfg = {
                            let db = core.store.db.lock().map_err(err)?;
                            if load(&db, &id)?.is_none() {
                                return Ok(());
                            }
                            config(&db, &id)?
                        };
                        let cycle = platform::cycle(&cfg, now)?;
                        tick(&core.store, &id, &cycle, now, &mut |action| {
                            platform::send(&handle.config().identifier, action)
                        })
                    })();
                    if let Err(error) = result {
                        core.report(format!("打工提醒：{error}"));
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                core.report(format!("打工提醒：{error}"));
            }
        });
    }
}
fn authorized_get(
    store: &Store,
    sessions: &widgets::Sessions,
    window: &str,
    token: &str,
    args: Value,
    now: u64,
) -> AppResult<Value> {
    if !args.is_null() && !args.as_object().is_some_and(|map| map.is_empty()) {
        return Err("INVALID_ARGUMENT: 打工进度读取不接受其他参数".into());
    }
    let id = widgets::require_capability(store, sessions, window, token, "reminders.labor")?;
    let cfg = {
        let db = store.db.lock().map_err(err)?;
        config(&db, &id)?
    };
    let cycle = platform::cycle(&cfg, now)?;
    get(store, &id, &cycle, now)
}
pub async fn gateway(app: &AppHandle, window: &str, token: &str, args: Value) -> AppResult<Value> {
    let handle = app.clone();
    let window = window.to_owned();
    let token = token.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let core = handle.state::<Core>();
        authorized_get(&core.store, &core.sessions, &window, &token, args, now_ms())
    })
    .await
    .map_err(err)?
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{FILETIME, SYSTEMTIME},
        System::Time::*,
    };
    const EPOCH: u64 = 116444736000000000;
    fn from_ticks(ticks: u64) -> AppResult<SYSTEMTIME> {
        let file = FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        let mut system: SYSTEMTIME = unsafe { std::mem::zeroed() };
        if unsafe { FileTimeToSystemTime(&file, &mut system) } == 0 {
            return Err("系统日期转换失败".into());
        }
        Ok(system)
    }
    fn ticks(system: &SYSTEMTIME) -> AppResult<u64> {
        let mut file = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        if unsafe { SystemTimeToFileTime(system, &mut file) } == 0 {
            return Err("系统日期无效".into());
        }
        Ok(u64::from(file.dwLowDateTime) | (u64::from(file.dwHighDateTime) << 32))
    }
    fn shifted(mut local: SYSTEMTIME, days: i32, minute: u32) -> AppResult<SYSTEMTIME> {
        local.wHour = 0;
        local.wMinute = 0;
        local.wSecond = 0;
        local.wMilliseconds = 0;
        let shifted = i128::from(ticks(&local)?) + i128::from(days) * 864000000000;
        let mut result = from_ticks(u64::try_from(shifted).map_err(err)?)?;
        result.wHour = (minute / 60) as u16;
        result.wMinute = (minute % 60) as u16;
        Ok(result)
    }
    fn utc_ms(zone: &DYNAMIC_TIME_ZONE_INFORMATION, local: &SYSTEMTIME) -> AppResult<u64> {
        let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
        let mut check: SYSTEMTIME = unsafe { std::mem::zeroed() };
        if unsafe { TzSpecificLocalTimeToSystemTimeEx(zone, local, &mut utc) } == 0
            || unsafe { SystemTimeToTzSpecificLocalTimeEx(zone, &utc, &mut check) } == 0
            || [
                local.wYear,
                local.wMonth,
                local.wDay,
                local.wHour,
                local.wMinute,
            ] != [
                check.wYear,
                check.wMonth,
                check.wDay,
                check.wHour,
                check.wMinute,
            ]
        {
            return Err("本地上下班时间无法转换，可能位于时区跳变区间".into());
        }
        Ok(ticks(&utc)?
            .checked_sub(EPOCH)
            .ok_or("系统时间早于支持范围")?
            / 10000)
    }
    pub(super) fn cycle(config: &Config, now: u64) -> AppResult<Cycle> {
        let mut zone: DYNAMIC_TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetDynamicTimeZoneInformation(&mut zone) } == TIME_ZONE_ID_INVALID {
            return Err("无法读取本地时区".into());
        }
        cycle_in_zone(config, now, &zone)
    }
    fn cycle_in_zone(
        config: &Config,
        now: u64,
        zone: &DYNAMIC_TIME_ZONE_INFORMATION,
    ) -> AppResult<Cycle> {
        let utc = from_ticks(
            now.checked_mul(10000)
                .and_then(|n| n.checked_add(EPOCH))
                .ok_or("系统时间超出范围")?,
        )?;
        let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
        if unsafe { SystemTimeToTzSpecificLocalTimeEx(zone, &utc, &mut local) } == 0 {
            return Err("无法读取本地上下班日期".into());
        }
        let overnight = config.end < config.start;
        let before_start = u32::from(local.wHour) * 60 + u32::from(local.wMinute) < config.start;
        let start = shifted(
            local,
            if overnight && before_start { -1 } else { 0 },
            config.start,
        )?;
        let end = shifted(
            local,
            if overnight && !before_start { 1 } else { 0 },
            config.end,
        )?;
        let result = Cycle {
            start: utc_ms(zone, &start)?,
            end: utc_ms(zone, &end)?,
            weekday: if local.wDayOfWeek == 0 {
                7
            } else {
                u32::from(local.wDayOfWeek)
            },
        };
        if result.end <= result.start {
            return Err("上下班时间区间无效".into());
        }
        Ok(result)
    }
    pub(super) fn send(app_id: &str, action: Action) -> AppResult<Delivery> {
        use windows::{
            core::{h, Interface, HSTRING},
            Data::Xml::Dom::XmlDocument,
            Foundation::{DateTime, IReference, PropertyValue},
            UI::Notifications::{
                NotificationData, NotificationUpdateResult, ToastNotification,
                ToastNotificationManager,
            },
        };
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))
            .map_err(err)?;
        let data = |remaining: u32, sequence: u32| -> AppResult<NotificationData> {
            let data = NotificationData::new().map_err(err)?;
            data.SetSequenceNumber(sequence).map_err(err)?;
            let values = data.Values().map_err(err)?;
            for (key, value) in [
                (
                    "remaining",
                    format!("{:02}:{:02}", remaining / 60, remaining % 60),
                ),
                (
                    "fraction",
                    (f64::from(remaining) / 300.).clamp(0., 1.).to_string(),
                ),
            ] {
                values
                    .Insert(&HSTRING::from(key), &HSTRING::from(value))
                    .map_err(err)?;
            }
            Ok(data)
        };
        match action {
            Action::Show { tag, remaining } => {
                let xml = XmlDocument::new().map_err(err)?;
                xml.LoadXml(h!("<toast><visual><binding template='ToastGeneric'><text>下班倒计时</text><text>{remaining}</text><progress title='下班倒计时' value='{fraction}' valueStringOverride='{remaining}' status='距离下班'/></binding></visual><audio silent='true'/></toast>")).map_err(err)?;
                let toast = ToastNotification::CreateToastNotification(&xml).map_err(err)?;
                toast.SetTag(&HSTRING::from(tag)).map_err(err)?;
                toast
                    .SetGroup(&HSTRING::from(NOTIFICATION_GROUP))
                    .map_err(err)?;
                toast.SetData(&data(remaining, 1)?).map_err(err)?;
                let expiry = now_ms()
                    .checked_add(u64::from(remaining) * 1000)
                    .and_then(|n| n.checked_mul(10000))
                    .and_then(|n| n.checked_add(EPOCH))
                    .and_then(|n| i64::try_from(n).ok())
                    .ok_or("提醒到期时间超出范围")?;
                let expiry = PropertyValue::CreateDateTime(DateTime {
                    UniversalTime: expiry,
                })
                .map_err(err)?
                .cast::<IReference<DateTime>>()
                .map_err(err)?;
                toast.SetExpirationTime(&expiry).map_err(err)?;
                notifier.Show(&toast).map_err(err)?;
                Ok(Delivery::Accepted)
            }
            Action::Update {
                tag,
                remaining,
                sequence,
            } => {
                let result = notifier
                    .UpdateWithTagAndGroup(
                        &data(remaining, sequence)?,
                        &HSTRING::from(tag),
                        &HSTRING::from(NOTIFICATION_GROUP),
                    )
                    .map_err(err)?;
                if result == NotificationUpdateResult::Succeeded {
                    Ok(Delivery::Accepted)
                } else if result == NotificationUpdateResult::NotificationNotFound {
                    Ok(Delivery::Missing)
                } else {
                    Err("系统未接受下班倒计时更新".into())
                }
            }
            Action::Remove { tag } => {
                ToastNotificationManager::History()
                    .map_err(err)?
                    .RemoveGroupedTagWithId(
                        &HSTRING::from(tag),
                        &HSTRING::from(NOTIFICATION_GROUP),
                        &HSTRING::from(app_id),
                    )
                    .map_err(err)?;
                Ok(Delivery::Accepted)
            }
        }
    }
    #[cfg(test)]
    mod calendar_tests {
        use super::*;
        fn date(year: u16, month: u16, day: u16, hour: u16, minute: u16) -> SYSTEMTIME {
            SYSTEMTIME {
                wYear: year,
                wMonth: month,
                wDay: day,
                wHour: hour,
                wMinute: minute,
                ..unsafe { std::mem::zeroed() }
            }
        }
        fn epoch(date: SYSTEMTIME) -> u64 {
            (ticks(&date).unwrap() - EPOCH) / 10000
        }
        fn zone_by_key(name: &str) -> DYNAMIC_TIME_ZONE_INFORMATION {
            for index in 0..512 {
                let mut zone: DYNAMIC_TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
                if unsafe { EnumDynamicTimeZoneInformation(index, &mut zone) } != 0 {
                    break;
                }
                let end = zone
                    .TimeZoneKeyName
                    .iter()
                    .position(|value| *value == 0)
                    .unwrap_or(zone.TimeZoneKeyName.len());
                if String::from_utf16_lossy(&zone.TimeZoneKeyName[..end]) == name {
                    return zone;
                }
            }
            panic!("未找到Windows标准时区：{name}");
        }
        #[test]
        fn overnight_cycle_keeps_previous_start_across_month_and_year() {
            let zone = zone_by_key("UTC");
            let now = epoch(date(2027, 1, 1, 2, 0));
            let result = cycle_in_zone(
                &Config {
                    start: 22 * 60,
                    end: 6 * 60,
                },
                now,
                &zone,
            )
            .unwrap();
            assert_eq!(result.start, epoch(date(2026, 12, 31, 22, 0)));
            assert_eq!(result.end, epoch(date(2027, 1, 1, 6, 0)));
            assert_eq!(result.weekday, 5);
        }
        #[test]
        fn windows_daylight_rules_use_real_elapsed_time_and_reject_nonexistent_local_time() {
            let zone = zone_by_key("Eastern Standard Time");
            let now = epoch(date(2026, 3, 8, 6, 30));
            let shift = cycle_in_zone(
                &Config {
                    start: 60,
                    end: 4 * 60,
                },
                now,
                &zone,
            )
            .unwrap();
            assert_eq!(shift.end - shift.start, 2 * 60 * 60 * 1000);
            assert!(cycle_in_zone(
                &Config {
                    start: 2 * 60 + 30,
                    end: 4 * 60
                },
                now,
                &zone
            )
            .is_err());
        }
    }
}
#[cfg(not(windows))]
mod platform {
    use super::*;
    pub(super) fn cycle(_config: &Config, _now: u64) -> AppResult<Cycle> {
        Err("当前平台未实现本地上下班日期".into())
    }
    pub(super) fn send(_id: &str, _action: Action) -> AppResult<Delivery> {
        Err("当前平台未实现下班倒计时通知".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    fn fixture() -> (Store, String, String) {
        let store =
            Store::open(std::env::temp_dir().join(format!("souldesk-labor-{}", Uuid::new_v4())))
                .unwrap();
        let manifest = json!({"schemaVersion":1,"id":"local.test.labor","name":"打工测试","version":"0.1.0","hostApiMajor":1,"entry":"dist/index.html","defaultSize":{"width":480,"height":150},"minSize":{"width":320,"height":100},"capabilities":["storage.instance","reminders.labor"],"settingsSchema":{"type":"object","additionalProperties":false,"required":["startHour","startMinute","endHour","endMinute"],"properties":{"startHour":{"type":"integer","default":9},"startMinute":{"type":"integer","default":0},"endHour":{"type":"integer","default":18},"endMinute":{"type":"integer","default":0}}}});
        store.db.lock().unwrap().execute("INSERT INTO widget_packages(id,manifest,directory,digest) VALUES(?1,?2,'test-labor','test-labor')",rusqlite::params!["local.test.labor",manifest.to_string()]).unwrap();
        let a = crate::widgets::create_instance(&store, "local.test.labor").unwrap();
        let b = crate::widgets::create_instance(&store, "local.test.labor").unwrap();
        (store, a, b)
    }
    fn cycle() -> Cycle {
        Cycle {
            start: 1000,
            end: 3601000,
            weekday: 1,
        }
    }
    #[test]
    fn progress_uses_exact_interval_and_clamps_only_valid_boundaries() {
        let (store, id, _) = fixture();
        let cycle = cycle();
        assert_eq!(
            get(&store, &id, &cycle, 1801000).unwrap()["percent"].as_f64(),
            Some(50.)
        );
        assert_eq!(
            get(&store, &id, &cycle, 0).unwrap()["percent"].as_f64(),
            Some(0.)
        );
        assert_eq!(
            get(&store, &id, &cycle, 4000000).unwrap()["percent"].as_f64(),
            Some(100.)
        );
        assert!(get(
            &store,
            &id,
            &Cycle {
                start: 1000,
                end: 1000,
                weekday: 1
            },
            1000
        )
        .is_err());
    }
    #[test]
    fn countdown_runs_without_views_shows_once_and_updates_without_resending() {
        let (store, id, _) = fixture();
        let cycle = cycle();
        let mut calls = Vec::new();
        let mut send = |action| {
            calls.push(action);
            Ok(Delivery::Accepted)
        };
        tick(&store, &id, &cycle, 3301000, &mut send).unwrap();
        assert!(calls.is_empty());
        get(&store, &id, &cycle, 1000).unwrap();
        store.presentation(&id, "desktop", false, false).unwrap();
        tick(&store, &id, &cycle, 3301000, &mut |action| {
            calls.push(action);
            Ok(Delivery::Accepted)
        })
        .unwrap();
        assert!(matches!(calls[0], Action::Show { remaining: 300, .. }));
        tick(&store, &id, &cycle, 3302000, &mut |action| {
            calls.push(action);
            Ok(Delivery::Accepted)
        })
        .unwrap();
        assert!(matches!(
            calls[1],
            Action::Update {
                remaining: 299,
                sequence: 3,
                ..
            }
        ));
        tick(&store, &id, &cycle, 3601000, &mut |action| {
            calls.push(action);
            Ok(Delivery::Accepted)
        })
        .unwrap();
        assert!(matches!(calls[2], Action::Remove { .. }));
        assert_eq!(calls.len(), 3);
    }
    #[test]
    fn failed_or_dismissed_countdown_does_not_reappear_same_shift() {
        let (store, id, _) = fixture();
        let cycle = cycle();
        get(&store, &id, &cycle, 1000).unwrap();
        tick(&store, &id, &cycle, 3301000, &mut |_| {
            Err("模拟系统拒绝".into())
        })
        .unwrap();
        assert_eq!(
            get(&store, &id, &cycle, 3301000).unwrap()["delivery"],
            "failed"
        );
        let mut calls = 0;
        tick(&store, &id, &cycle, 3302000, &mut |_| {
            calls += 1;
            Ok(Delivery::Accepted)
        })
        .unwrap();
        assert_eq!(calls, 0);
        let next = Cycle {
            start: 86401000,
            end: 90001000,
            weekday: 2,
        };
        tick(&store, &id, &next, 89701000, &mut |_| {
            Ok(Delivery::Accepted)
        })
        .unwrap();
        tick(&store, &id, &next, 89702000, &mut |_| Ok(Delivery::Missing)).unwrap();
        assert_eq!(
            get(&store, &id, &next, 89702000).unwrap()["delivery"],
            "dismissed"
        );
        tick(&store, &id, &next, 89703000, &mut |_| {
            calls += 1;
            Ok(Delivery::Accepted)
        })
        .unwrap();
        assert_eq!(calls, 0);
    }
    #[test]
    fn persistence_resume_uses_increasing_sequence_and_separate_instance_tags() {
        let (store, a, b) = fixture();
        let cycle = cycle();
        let mut tags = Vec::new();
        for id in [&a, &b] {
            get(&store, id, &cycle, 1000).unwrap();
            tick(&store, id, &cycle, 3301000, &mut |action| {
                if let Action::Show { tag, .. } = action {
                    tags.push(tag);
                }
                Ok(Delivery::Accepted)
            })
            .unwrap();
        }
        assert_ne!(tags[0], tags[1]);
        let root = store.root.clone();
        drop(store);
        let reopened = Store::open(root).unwrap();
        let mut action = None;
        tick(&reopened, &a, &cycle, 3306000, &mut |value| {
            action = Some(value);
            Ok(Delivery::Accepted)
        })
        .unwrap();
        assert!(matches!(
            action,
            Some(Action::Update {
                sequence: 7,
                remaining: 295,
                ..
            })
        ));
    }
    #[test]
    fn clock_reversal_or_schedule_change_removes_old_notice_without_duplicate_show() {
        let (store, id, _) = fixture();
        let cycle = cycle();
        get(&store, &id, &cycle, 1000).unwrap();
        tick(&store, &id, &cycle, 3301000, &mut |_| {
            Ok(Delivery::Accepted)
        })
        .unwrap();
        let mut removed = 0;
        tick(&store, &id, &cycle, 3201000, &mut |action| {
            assert!(matches!(action, Action::Remove { .. }));
            removed += 1;
            Ok(Delivery::Accepted)
        })
        .unwrap();
        assert_eq!(removed, 1);
        tick(&store, &id, &cycle, 3301000, &mut |_| {
            panic!("同一班次不能再次展示")
        })
        .unwrap();
    }
    #[test]
    fn late_start_before_end_sends_one_current_countdown_but_never_after_end() {
        let (store, id, _) = fixture();
        let cycle = cycle();
        get(&store, &id, &cycle, 3541000).unwrap();
        let mut calls = 0;
        tick(&store, &id, &cycle, 3541000, &mut |action| {
            assert!(matches!(action, Action::Show { remaining: 60, .. }));
            calls += 1;
            Ok(Delivery::Accepted)
        })
        .unwrap();
        assert_eq!(calls, 1);
        let second = widgets::create_instance(&store, "local.test.labor").unwrap();
        get(&store, &second, &cycle, cycle.end).unwrap();
        tick(&store, &second, &cycle, cycle.end, &mut |_| {
            panic!("结束后不应补发提醒")
        })
        .unwrap();
    }
    #[test]
    fn repeated_tick_in_same_second_does_not_send_duplicate_sequence() {
        let (store, id, _) = fixture();
        let cycle = cycle();
        get(&store, &id, &cycle, 1000).unwrap();
        tick(&store, &id, &cycle, 3301000, &mut |_| {
            Ok(Delivery::Accepted)
        })
        .unwrap();
        let mut calls = 0;
        for _ in 0..2 {
            tick(&store, &id, &cycle, 3302000, &mut |_| {
                calls += 1;
                Ok(Delivery::Accepted)
            })
            .unwrap();
        }
        assert_eq!(calls, 1, "同一序号会被Windows拒绝，不应重复更新");
    }
    #[test]
    fn idle_reads_do_not_rewrite_state_and_deleting_one_instance_queues_only_own_notice() {
        let (store, a, b) = fixture();
        let cycle = cycle();
        get(&store, &a, &cycle, 1000).unwrap();
        get(&store, &b, &cycle, 1000).unwrap();
        let before = store
            .db
            .lock()
            .unwrap()
            .query_row("SELECT total_changes()", [], |r| r.get::<_, u64>(0))
            .unwrap();
        get(&store, &a, &cycle, 2000).unwrap();
        tick(&store, &a, &cycle, 2000, &mut |_| panic!("尚未到提醒时间")).unwrap();
        let after = store
            .db
            .lock()
            .unwrap()
            .query_row("SELECT total_changes()", [], |r| r.get::<_, u64>(0))
            .unwrap();
        assert_eq!(before, after);
        tick(&store, &a, &cycle, 3301000, &mut |_| Ok(Delivery::Accepted)).unwrap();
        let tag = load(&store.db.lock().unwrap(), &a).unwrap().unwrap().tag;
        crate::widgets::remove_instance(&store, &Default::default(), &a).unwrap();
        let db = store.db.lock().unwrap();
        assert!(load(&db, &a).unwrap().is_none());
        assert!(load(&db, &b).unwrap().is_some());
        let queued: String = db
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                [format!("labor.cleanup:{tag}")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(queued, "pending");
    }
    #[test]
    fn crash_before_system_result_is_unknown_not_success_or_another_show() {
        let (store, id, _) = fixture();
        let cycle = cycle();
        get(&store, &id, &cycle, 1000).unwrap();
        let mut state = load(&store.db.lock().unwrap(), &id).unwrap().unwrap();
        state.delivery = "sending".into();
        state.target = cycle.end;
        state.started = 3301000;
        state.tag = "0123456789abcdef".into();
        state.sent.push(cycle.end);
        save(&store.db.lock().unwrap(), &id, &state).unwrap();
        tick(&store, &id, &cycle, 3302000, &mut |action| {
            assert!(matches!(action, Action::Remove { .. }));
            Ok(Delivery::Accepted)
        })
        .unwrap();
        assert_eq!(
            get(&store, &id, &cycle, 3302000).unwrap()["delivery"],
            "unknown"
        );
        tick(&store, &id, &cycle, 3303000, &mut |_| {
            panic!("未确定的递送不能重新展示")
        })
        .unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn formal_labor_package_enforces_capability_preview_and_independent_settings() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-labor-formal-{}", Uuid::new_v4())),
        )
        .unwrap();
        let staged = widgets::stage_bytes(
            &store,
            crate::presets::find("local.souldesk.widgetjs.labor")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        let defaults = widgets::initial_settings(&staged.manifest);
        widgets::activate(&store, &staged).unwrap();
        let a = widgets::create_instance(&store, "local.souldesk.widgetjs.labor").unwrap();
        let b = widgets::create_instance(&store, "local.souldesk.widgetjs.labor").unwrap();
        let sessions = widgets::Sessions::default();
        let label = format!("float-{a}");
        let session = widgets::open_display_session(&store, &sessions, &label, &a, None).unwrap();
        let token = session["token"].as_str().unwrap();
        let now = now_ms();
        sessions.0.lock().unwrap().insert(
            "preview".into(),
            widgets::Session {
                instance: a.clone(),
                window: "trial-labor".into(),
                shared_cursor: None,
                preview: Some(std::sync::Arc::new(std::sync::Mutex::new(
                    widgets::Preview {
                        package: staged,
                        settings: defaults.clone(),
                        storage: json!({}),
                        ready_since: None,
                        failure: None,
                    },
                ))),
            },
        );
        assert!(authorized_get(
            &store,
            &sessions,
            "trial-labor",
            "preview",
            Value::Null,
            now
        )
        .is_err());
        assert!(load(&store.db.lock().unwrap(), &a).unwrap().is_none());
        assert!(
            authorized_get(&store, &sessions, "wrong-window", token, Value::Null, now).is_err()
        );
        assert!(
            authorized_get(&store, &sessions, &label, token, json!({"instance":b}), now).is_err()
        );
        assert!(
            authorized_get(&store, &sessions, &label, token, Value::Null, now).unwrap()["startAt"]
                .as_u64()
                .is_some()
        );
        let mut night = defaults.clone();
        night["startHour"] = json!(22);
        night["endHour"] = json!(6);
        widgets::gateway(
            &store,
            &sessions,
            &label,
            token,
            "settings.update",
            night.clone(),
        )
        .unwrap();
        let root = store.root.clone();
        drop(store);
        let reopened = Store::open(root).unwrap();
        let sessions = widgets::Sessions::default();
        for (id, expected) in [(&a, night), (&b, defaults)] {
            let label = format!("float-{id}");
            let session =
                widgets::open_display_session(&reopened, &sessions, &label, id, None).unwrap();
            assert_eq!(
                widgets::gateway(
                    &reopened,
                    &sessions,
                    &label,
                    session["token"].as_str().unwrap(),
                    "settings.get",
                    Value::Null
                )
                .unwrap(),
                expected
            );
        }
        let other = widgets::stage_bytes(
            &reopened,
            crate::presets::find("local.fixture.surface")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        widgets::activate(&reopened, &other).unwrap();
        let id = widgets::create_instance(&reopened, "local.fixture.surface").unwrap();
        let label = format!("float-{id}");
        let session =
            widgets::open_display_session(&reopened, &sessions, &label, &id, None).unwrap();
        assert!(authorized_get(
            &reopened,
            &sessions,
            &label,
            session["token"].as_str().unwrap(),
            Value::Null,
            now
        )
        .is_err());
    }
}
