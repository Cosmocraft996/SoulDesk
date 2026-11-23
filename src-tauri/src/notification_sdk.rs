//! 通用组件通知协议。宿主负责来源身份、渠道和允许的操作。
use crate::{
    reminder_engine::NoticeKind,
    reminder_state::PhonePreset,
    reminder_store::{self, Channel},
    store::{err, AppResult},
    widgets,
};
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use uuid::Uuid;

pub(crate) const SEND: &str = "notifications.send";
pub(crate) const CALL: &str = "notifications.call";
pub(crate) const QUEUE_LIMIT: usize = 16;
pub(crate) const MAX_WAIT_MS: u64 = 45_000;
const MAX_COUNTDOWN_MS: u64 = 60 * 60 * 1000;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    pub request_id: String,
    pub channel: Option<Channel>,
    #[serde(default = "info")]
    pub kind: NoticeKind,
    pub title: Option<String>,
    pub body: Option<String>,
    pub target_at: Option<u64>,
    pub preset: Option<PhonePreset>,
}

fn info() -> NoticeKind {
    NoticeKind::Info
}

impl Request {
    pub(crate) fn parse(value: serde_json::Value) -> AppResult<Self> {
        let mut request: Self =
            serde_json::from_value(value).map_err(|e| format!("INVALID_ARGUMENT: {e}"))?;
        request.request_id = Uuid::parse_str(&request.request_id)
            .map_err(|_| "INVALID_ARGUMENT: 请求标识须为 UUID")?
            .to_string();
        if request.kind == NoticeKind::MarketAlert {
            return Err("CAPABILITY_DENIED: 行情通知仅由宿主价格警报产生".into());
        }
        if request.kind == NoticeKind::Call {
            if request.preset.is_none()
                || request.title.is_some()
                || request.body.is_some()
                || request.target_at.is_some()
            {
                return Err("INVALID_ARGUMENT: 模拟来电只接受固定 preset".into());
            }
            return Ok(request);
        }
        if request.preset.is_some()
            || request.wants_countdown() != request.target_at.is_some()
            || request
                .title
                .as_deref()
                .is_none_or(|title| !valid_text(title, 80))
            || request
                .body
                .as_deref()
                .is_some_and(|body| !valid_text(body, 200))
        {
            return Err("INVALID_ARGUMENT: 请核对通知标题、正文和目标时间".into());
        }
        Ok(request)
    }

    fn wants_countdown(&self) -> bool {
        matches!(
            self.kind,
            NoticeKind::AdvanceCountdown | NoticeKind::Countdown
        )
    }

    pub(crate) fn validate_time(&self, now: u64) -> AppResult<()> {
        if self
            .target_at
            .is_some_and(|target| target <= now || target.saturating_sub(now) > MAX_COUNTDOWN_MS)
        {
            return Err("INVALID_ARGUMENT: 倒计时目标须在未来一小时内".into());
        }
        Ok(())
    }
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= max
        && value.chars().all(|c| !c.is_control())
}

/// 每次创建展示目标和执行按钮时复核来源，防止停用或升级后操作旧通知。
pub(crate) fn source(
    db: &Connection,
    instance: &str,
    kind: NoticeKind,
) -> AppResult<Option<String>> {
    let manifest: Option<String> = db.query_row(
        "SELECT p.manifest FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE i.id=?1",
        [instance],
        |row| row.get(0),
    ).optional().map_err(err)?;
    let Some(manifest) =
        manifest.and_then(|raw| serde_json::from_str::<widgets::Manifest>(&raw).ok())
    else {
        return Ok(None);
    };
    let calls = kind == NoticeKind::Call;
    if !manifest.capabilities.iter().any(|cap| cap == SEND)
        || (calls && !manifest.capabilities.iter().any(|cap| cap == CALL))
    {
        return Ok(None);
    }
    Ok(Some(manifest.name))
}

pub(crate) fn available(
    db: &Connection,
    instance: &str,
    request: &Request,
) -> AppResult<Option<(Channel, String)>> {
    let Some(name) = source(db, instance, request.kind)? else {
        return Err("CAPABILITY_DENIED: 未声明对应的组件通知能力".into());
    };
    let channel = match request.channel {
        Some(channel) => channel,
        None => Channel::legacy_default(db)?,
    };
    if !channel.enabled(db)?
        || request.kind == NoticeKind::Call
            && reminder_store::preference(db, reminder_store::WIDGET_CALLS)?.as_deref()
                != Some("true")
    {
        return Ok(None);
    }
    Ok(Some((channel, name)))
}

#[cfg(windows)]
pub(crate) fn windows(
    app: &tauri::AppHandle,
    source: &str,
    notice: &crate::reminder_engine::Notice,
    now: u64,
) -> AppResult<()> {
    let message = match notice.target_at {
        Some(target) => {
            let seconds = target.saturating_sub(now).div_ceil(1_000);
            format!(
                "{} 分 {:02} 秒后到期。{}",
                seconds / 60,
                seconds % 60,
                notice.message
            )
        }
        None => notice.message.clone(),
    };
    tauri_winrt_notification::Toast::new(&app.config().identifier)
        .title(&format!("{source} · {}", notice.title))
        .text1(&message)
        .sound(None)
        .show()
        .map_err(err)
}

#[cfg(not(windows))]
pub(crate) fn windows(
    _: &tauri::AppHandle,
    _: &str,
    _: &crate::reminder_engine::Notice,
    _: u64,
) -> AppResult<()> {
    Err("当前系统未验证 Windows 通知".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        reminder_engine::{Action, Clock, Engine, Surface, HOST_NOTICE_OWNER},
        reminder_state::LocalMinute,
        store::Store,
    };
    use serde_json::{json, Value};

    fn clock(ms: u64) -> Clock {
        Clock {
            wall_ms: 1_800_000_000_000 + ms,
            monotonic_ms: ms,
            local: LocalMinute {
                day: 1,
                weekday: 1,
                minute: 0,
                second: 0,
            },
        }
    }
    fn fixture(capabilities: &[&str]) -> (Store, String) {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-notifications-{}", Uuid::new_v4())),
        )
        .unwrap();
        let package = "local.souldesk.fixture.notifications";
        let manifest = json!({"schemaVersion":1,"id":package,"name":"通知测试","version":"1.0.0","hostApiMajor":1,"entry":"dist/index.html","defaultSize":{"width":350,"height":180},"minSize":{"width":280,"height":140},"capabilities":capabilities,"settingsSchema":{"type":"object","additionalProperties":false,"properties":{}}});
        store.db.lock().unwrap().execute("INSERT INTO widget_packages(id,manifest,directory,digest) VALUES(?1,?2,'fixture','fixture')", rusqlite::params![package, manifest.to_string()]).unwrap();
        let id = widgets::create_instance(&store, package).unwrap();
        (store, id)
    }
    fn request(kind: &str) -> Request {
        let id = Uuid::new_v4().to_string();
        let mut payload = match kind {
            "call" => json!({"requestId":id,"kind":kind,"preset":"tea"}),
            "countdown" | "advance-countdown" => {
                json!({"requestId":id,"kind":kind,"title":"任务","targetAt":clock(60_000).wall_ms})
            }
            _ => json!({"requestId":id,"kind":kind,"title":"任务","body":"提醒正文"}),
        };
        payload["channel"] = json!("island");
        Request::parse(payload).unwrap()
    }
    fn send(
        engine: &mut Engine,
        store: &Store,
        source: &str,
        request: Request,
        ms: u64,
    ) -> AppResult<Value> {
        engine.send(
            store,
            source,
            request,
            clock(ms),
            false,
            |_, _| unreachable!(),
        )
    }

    #[test]
    fn notification_sdk_all_eight_kinds_reach_the_host_without_legacy_widget() {
        let (store, source) = fixture(&[SEND, CALL]);
        store
            .save_preference(reminder_store::WIDGET_CALLS, "true")
            .unwrap();
        for kind in [
            "info",
            "success",
            "warning",
            "error",
            "reminder",
            "advance-countdown",
            "countdown",
            "call",
        ] {
            let mut engine = Engine::default();
            let reply = send(&mut engine, &store, &source, request(kind), 0).unwrap();
            let target = engine.target(Surface::Notice).unwrap();
            let notice = engine.notification().unwrap();
            assert_eq!(target.owner, HOST_NOTICE_OWNER);
            assert_eq!(notice.source, source);
            assert_eq!(serde_json::to_value(notice.kind).unwrap(), kind);
            assert_eq!(reply["id"], notice.id);
            assert_eq!(reply["channel"], "island");
            if kind == "reminder" {
                assert!(engine
                    .act(&store, &target, &target.event, Action::Confirm, clock(1))
                    .is_err());
                engine
                    .act(&store, &target, &target.event, Action::OpenSource, clock(1))
                    .unwrap();
                assert!(engine.rest().is_none());
            }
            if kind == "advance-countdown" {
                engine
                    .act(&store, &target, &target.event, Action::Countdown, clock(1))
                    .unwrap();
                assert_eq!(engine.notification().unwrap().kind, NoticeKind::Countdown);
            }
        }
    }
    #[test]
    fn notification_sdk_keeps_order_deduplicates_and_discards_revoked_source() {
        let (store, source) = fixture(&[SEND]);
        let mut engine = Engine::default();
        let req = request("info");
        let first = send(&mut engine, &store, &source, req.clone(), 0).unwrap();
        assert_eq!(
            first,
            send(&mut engine, &store, &source, req.clone(), 1).unwrap()
        );
        let mut conflict = req;
        conflict.title = Some("其他通知".into());
        assert!(send(&mut engine, &store, &source, conflict, 2).is_err());
        send(&mut engine, &store, &source, request("success"), 3).unwrap();
        send(&mut engine, &store, &source, request("warning"), 4).unwrap();
        let first = engine.notice_target(first["id"].as_str().unwrap()).unwrap();
        assert_eq!(engine.notification().unwrap().kind, NoticeKind::Warning);
        assert_eq!(engine.visible().count(), 3);
        engine
            .act(&store, &first, &first.event, Action::Dismiss, clock(5))
            .unwrap();
        engine.show_queued(&store, clock(5)).unwrap();
        assert_eq!(engine.notification().unwrap().kind, NoticeKind::Warning);
        assert_eq!(engine.visible().count(), 2);
        widgets::remove_instance(&store, &widgets::Sessions::default(), &source).unwrap();
        engine
            .tick(&store, clock(7), false, &mut || unreachable!())
            .unwrap();
        assert!(engine.notification().is_none());
    }
    #[test]
    fn notification_sdk_checks_call_permission_opt_in_and_windows_routing() {
        let (limited, source) = fixture(&[SEND]);
        let mut engine = Engine::default();
        assert!(send(&mut engine, &limited, &source, request("call"), 0).is_err());
        let (store, source) = fixture(&[SEND, CALL]);
        assert_eq!(
            send(&mut engine, &store, &source, request("call"), 0).unwrap()["status"],
            "suppressed"
        );
        store
            .save_preference(reminder_store::WIDGET_CALLS, "true")
            .unwrap();
        let mut call = request("call");
        call.channel = Some(Channel::Windows);
        let result = engine
            .send(&store, &source, call, clock(1), false, |name, notice| {
                assert_eq!(name, "通知测试");
                assert_eq!(notice.kind, NoticeKind::Call);
                Ok(())
            })
            .unwrap();
        assert_eq!(result["channel"], "windows");
        assert!(engine.notification().is_none());
    }
    #[test]
    fn notification_sdk_explicit_channels_are_enabled_independently_without_fallback() {
        let (store, source) = fixture(&[SEND]);
        let mut engine = Engine::default();
        assert!(Channel::Island.enabled_in(&store).unwrap());
        assert!(Channel::Windows.enabled_in(&store).unwrap());
        let mut windows = request("info");
        windows.channel = Some(Channel::Windows);
        let reply = engine
            .send(&store, &source, windows, clock(0), false, |_, _| Ok(()))
            .unwrap();
        assert_eq!(reply["channel"], "windows");
        assert_eq!(
            send(&mut engine, &store, &source, request("info"), 1).unwrap()["channel"],
            "island"
        );

        store
            .save_preference(Channel::Island.preference_key(), "false")
            .unwrap();
        assert!(Channel::Windows.enabled_in(&store).unwrap());
        assert_eq!(
            send(&mut engine, &store, &source, request("info"), 2).unwrap()["status"],
            "suppressed"
        );
        engine
            .tick(&store, clock(3), false, &mut || unreachable!())
            .unwrap();
        assert!(engine.notification().is_none());

        store
            .save_preference(Channel::Island.preference_key(), "true")
            .unwrap();
        store
            .save_preference(Channel::Windows.preference_key(), "false")
            .unwrap();
        let mut windows = request("info");
        windows.channel = Some(Channel::Windows);
        assert_eq!(
            send(&mut engine, &store, &source, windows, 4).unwrap()["status"],
            "suppressed"
        );
        assert_eq!(
            send(&mut engine, &store, &source, request("info"), 5).unwrap()["channel"],
            "island"
        );
    }
    #[test]
    fn notification_sdk_rejects_arbitrary_controls_and_expired_countdowns() {
        for key in ["action", "url", "audioUrl", "instanceId"] {
            let mut payload = json!({"requestId":Uuid::new_v4().to_string(),"title":"任务"});
            payload[key] = json!("组件不能指定");
            assert!(Request::parse(payload).is_err(), "{key}");
        }
        assert!(Request::parse(
            json!({"requestId":Uuid::new_v4().to_string(),"title":"任务","channel":"both"})
        )
        .is_err());
        assert!(request("countdown")
            .validate_time(clock(60_000).wall_ms)
            .is_err());
    }
}
