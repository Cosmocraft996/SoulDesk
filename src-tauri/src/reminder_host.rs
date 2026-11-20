//! 宿主通知、固定提醒能力与专用展示角色。
use crate::{
    notification_sdk,
    reminder_engine::{Action, Clock, Demo, Engine, Surface, Target, HOST_NOTICE_OWNER},
    reminder_state::{LocalMinute, PhonePreset},
    reminder_store::{self, Channel, Source},
    store::{err, now_ms, AppResult, Store},
    widgets, windows, Core,
};
use chrono::{Datelike, Timelike};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum IslandCommand {
    Ready,
    Painted { id: String },
    Failed { message: String, id: Option<String> },
    Action { id: String, action: Action },
    Hover { id: String, entered: bool },
    Audio { id: String, state: String },
}

#[tauri::command]
pub(crate) async fn island_surface(
    app: AppHandle,
    window: tauri::WebviewWindow,
    command: IslandCommand,
) -> AppResult<Value> {
    let label = window.label().to_owned();
    if !label.starts_with("island-host-") {
        return Err("CAPABILITY_DENIED: 仅宿主灵动通知窗口可调用".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let core = app.state::<Core>();
        let service = &core.reminders;
        let mut source_to_open = None;
        let schedule = matches!(
            &command,
            IslandCommand::Action { .. } | IslandCommand::Audio { .. }
        );
        let result = (|| {
            let _native = core.store.native_actions.lock().map_err(err)?;
            if service.stopped.load(Ordering::Acquire) || windows::hidden(&app)? {
                return Err("SESSION_EXPIRED: 通知窗口已停用".into());
            }
            if !service
                .island_window
                .lock()
                .map_err(err)?
                .as_ref()
                .is_some_and(|view| view.label == label)
            {
                return Err("SESSION_EXPIRED: 灵动通知窗口已被替换".into());
            }
            let mut engine = service.engine.lock().map_err(err)?;
            let target = engine.target(Surface::Notice);
            let clock = service.clock()?;
            match command {
                IslandCommand::Ready => windows::ready(&app, &label, HOST_NOTICE_OWNER)?,
                IslandCommand::Painted { id } => {
                    require_notice(target, &id)?;
                    windows::ready(&app, &label, &id)?;
                }
                IslandCommand::Failed { message, id } => {
                    if let Some(id) = &id {
                        require_notice(target, id)?;
                    }
                    windows::failed(
                        &app,
                        &label,
                        id.as_deref().unwrap_or(HOST_NOTICE_OWNER),
                        message.chars().take(1_000).collect(),
                    )?;
                }
                IslandCommand::Action { id, action } => {
                    let target = require_notice(target, &id)?;
                    let source = engine.notification().map(|n| n.source.clone());
                    engine.act(&core.store, &target, &id, action, clock)?;
                    engine.show_queued(&core.store, clock)?;
                    if matches!(action, Action::OpenSource) {
                        source_to_open = source;
                    }
                }
                IslandCommand::Hover { id, entered } => {
                    let target = require_notice(target, &id)?;
                    engine.hover(&core.store, &target, &id, entered, clock)?
                }
                IslandCommand::Audio { id, state } => {
                    let target = require_notice(target, &id)?;
                    engine.audio(&core.store, &target, &id, &state, clock)?;
                    if state == "ended" {
                        engine.show_queued(&core.store, clock)?;
                    }
                }
            }
            Ok(Value::Null)
        })();
        if let Some(source) = source_to_open {
            windows::dispatch(&app, move |app| {
                windows::open_notification_source(app, &source)
            });
        }
        // 读取与悬停不等待窗口协调，窗口创建正在等待本页就绪。
        if schedule {
            service.schedule(&app);
        }
        result
    })
    .await
    .map_err(err)?
}

fn require_notice(target: Option<Target>, id: &str) -> AppResult<Target> {
    target
        .filter(|current| current.event == id)
        .ok_or_else(|| "SESSION_EXPIRED: 灵动通知事件已被替换".into())
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum SettingsCommand {
    Get,
    Channel { channel: Channel, enabled: bool },
    Calls { enabled: bool },
    Test { channel: Channel, sample: Demo },
}

#[tauri::command]
pub(crate) async fn notification_settings(
    app: AppHandle,
    window: tauri::WebviewWindow,
    command: SettingsCommand,
) -> AppResult<Value> {
    if window.label() != "main" {
        return Err("CAPABILITY_DENIED: 通知偏好只能在管理窗口操作".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let core = app.state::<Core>();
        let service = &core.reminders;
        let disable_island = matches!(&command, SettingsCommand::Channel { channel: Channel::Island, enabled: false });
        let result = (|| {
            let _native = core.store.native_actions.lock().map_err(err)?;
            match command {
                SettingsCommand::Get => {}
                SettingsCommand::Channel { channel, enabled } => {
                    core.store.save_preference(channel.preference_key(), if enabled { "true" } else { "false" })?;
                    if channel == Channel::Island && !enabled {
                        service.engine.lock().map_err(err)?.cancel_notifications();
                    }
                }
                SettingsCommand::Calls { enabled } => {
                    core.store.save_preference(reminder_store::WIDGET_CALLS, if enabled { "true" } else { "false" })?;
                }
                SettingsCommand::Test { channel, sample } => {
                    let clock = service.clock()?;
                    service.engine.lock().map_err(err)?.host_demo(
                        &core.store, sample, channel, clock, windows::hidden(&app)?,
                        |notice| notification_sdk::windows(&app, "SoulDesk", notice, clock.wall_ms),
                    )?;
                }
            }
            let db = core.store.db.lock().map_err(err)?;
            Ok(json!({
                "windows":Channel::Windows.enabled(&db)?,
                "island":Channel::Island.enabled(&db)?,
                "calls":reminder_store::preference(&db, reminder_store::WIDGET_CALLS)?.as_deref() == Some("true")
            }))
        })();
        service.schedule(&app);
        if disable_island && result.is_ok() {
            windows::dispatch(&app, windows::release_idle_island);
        }
        result
    })
    .await
    .map_err(err)?
}

pub(crate) fn surface_role(window: &str, instance: &str) -> Option<(Source, &'static str, u64)> {
    let suffix = window.strip_prefix(&format!("break-{instance}-"))?;
    let generation = suffix.parse::<u64>().ok()?;
    (generation > 0 && generation.to_string() == suffix).then_some((
        Source::Sit,
        "break-overlay",
        generation,
    ))
}
pub(crate) fn surface_method(window: &str, method: &str) -> AppResult<()> {
    let allowed = if window.starts_with("island-") {
        false
    } else if window.starts_with("break-") {
        matches!(
            method,
            "runtime.ready" | "sit.break.peek" | "sit.break.close"
        )
    } else {
        true
    };
    if !allowed {
        return Err("CAPABILITY_DENIED: 当前专用显示层无此能力".into());
    }
    Ok(())
}
pub(crate) fn surface_allowed(
    db: &rusqlite::Connection,
    window: &str,
    instance: &str,
    manifest: &widgets::Manifest,
) -> AppResult<bool> {
    let Some((source, _, _)) = surface_role(window, instance) else {
        return Ok(false);
    };
    if manifest.id != source.package()
        || !manifest
            .capabilities
            .iter()
            .any(|cap| cap == source.capability())
    {
        return Ok(false);
    }
    Ok(reminder_store::load(db, instance, source)?.config["enabled"] == true)
}
pub(crate) struct IslandWindow {
    pub label: String,
    pub idle_since: Option<Instant>,
}

impl IslandWindow {
    pub(crate) fn expired(&self, now: Instant) -> bool {
        self.idle_since
            .is_some_and(|idle| now.saturating_duration_since(idle) >= Duration::from_secs(30))
    }
}

pub(crate) struct Service {
    engine: Mutex<Engine>,
    origin: Instant,
    busy: AtomicBool,
    stopped: AtomicBool,
    sent: Mutex<[Option<Target>; 2]>,
    rendered: Mutex<std::collections::HashSet<String>>,
    pub(crate) island_window: Mutex<Option<IslandWindow>>,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            engine: Mutex::new(Engine::default()),
            origin: Instant::now(),
            busy: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            sent: Mutex::new([None, None]),
            rendered: Mutex::new(Default::default()),
            island_window: Mutex::new(None),
        }
    }
}
impl Service {
    pub(crate) fn notice_snapshot(&self, target: &Target) -> AppResult<Value> {
        let engine = self.engine.lock().map_err(err)?;
        require_notice(engine.target(Surface::Notice), &target.event)?;
        let notice = engine.notification().ok_or("SESSION_EXPIRED: 通知已关闭")?;
        Ok(json!({
            "notification":notice, "sampledAt":now_ms(), "displayed":false
        }))
    }
    fn clock(&self) -> AppResult<Clock> {
        let wall_ms = now_ms();
        let utc = chrono::DateTime::from_timestamp_millis(i64::try_from(wall_ms).map_err(err)?)
            .ok_or("无法转换当前提醒时间")?;
        let local = utc.with_timezone(&chrono::Local);
        Ok(Clock {
            wall_ms,
            monotonic_ms: self.origin.elapsed().as_millis() as u64,
            local: LocalMinute {
                day: i64::from(local.date_naive().num_days_from_ce()),
                weekday: local.weekday().num_days_from_sunday() as u8,
                minute: (local.hour() * 60 + local.minute()) as u16,
                second: local.second() as u8,
            },
        })
    }
    pub fn target(&self, surface: Surface) -> Option<Target> {
        if self.stopped.load(Ordering::Acquire) {
            return None;
        }
        self.engine.lock().ok()?.target(surface)
    }
    pub fn current_window(&self, label: &str, instance: &str) -> bool {
        self.target(Surface::Rest)
            .is_some_and(|target| target.owner == instance && target.rest_label() == label)
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(mut engine) = self.engine.lock() {
            engine.clear();
        }
    }
    pub fn closed(&self, label: &str) {
        let lost_island = if let Ok(mut view) = self.island_window.lock() {
            if view.as_ref().is_some_and(|view| view.label == label) {
                view.take().is_some_and(|view| view.idle_since.is_none())
            } else {
                false
            }
        } else {
            false
        };
        if let Ok(mut engine) = self.engine.lock() {
            if lost_island {
                engine.cancel(Surface::Notice);
            } else if engine
                .target(Surface::Rest)
                .is_some_and(|target| target.rest_label() == label)
            {
                engine.cancel(Surface::Rest);
            }
        }
        if let Ok(mut rendered) = self.rendered.lock() {
            rendered.remove(label);
        }
    }
    fn schedule(&self, app: &AppHandle) {
        for (index, surface) in [Surface::Notice, Surface::Rest].into_iter().enumerate() {
            let desired = self.target(surface);
            let changed = if let Ok(mut sent) = self.sent.lock() {
                if sent[index] == desired {
                    false
                } else {
                    sent[index] = desired.clone();
                    true
                }
            } else {
                false
            };
            if changed {
                windows::dispatch(app, move |app| {
                    windows::reminder_reconcile(app, surface, desired)
                });
            }
        }
    }
    pub fn show_target(
        &self,
        app: &AppHandle,
        target: &Target,
        label: &str,
        show: impl FnOnce() -> AppResult<()>,
    ) -> AppResult<()> {
        let core = app.state::<Core>();
        let _native = core.store.native_actions.lock().map_err(err)?;
        if core.quitting.load(Ordering::Acquire)
            || self.stopped.load(Ordering::Acquire)
            || windows::hidden(app)?
        {
            return Err("SESSION_EXPIRED: 专用显示已停用".into());
        }
        self.engine.lock().map_err(err)?.validate_target(
            &core.store,
            target,
            &target.event,
            self.clock()?,
        )?;
        show()?;
        self.engine
            .lock()
            .map_err(err)?
            .shown(&core.store, target, self.clock()?)?;
        if target.surface == Surface::Notice {
            app.emit_to(label, "host:island-shown", &target.event)
                .map_err(err)?;
        } else {
            self.rendered.lock().map_err(err)?.insert(label.into());
        }
        Ok(())
    }
    pub fn failed(&self, app: &AppHandle, target: &Target, error: String) {
        let core = app.state::<Core>();
        let result = (|| {
            let _native = core.store.native_actions.lock().map_err(err)?;
            let mut engine = self.engine.lock().map_err(err)?;
            let result = engine.failed(&core.store, target, error, self.clock()?);
            if result.is_err() {
                engine.cancel(target.surface);
            }
            result
        })();
        if let Err(error) = result {
            core.report(format!("专用提醒显示状态未保存：{error}"));
        }
        self.schedule(app);
    }
    pub fn dispatch(&self, app: &AppHandle) {
        if self.stopped.load(Ordering::Acquire) || self.busy.swap(true, Ordering::AcqRel) {
            return;
        }
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            struct Release(AppHandle);
            impl Drop for Release {
                fn drop(&mut self) {
                    self.0
                        .state::<Core>()
                        .reminders
                        .busy
                        .store(false, Ordering::Release);
                }
            }
            let _release = Release(handle.clone());
            let core = handle.state::<Core>();
            let service = &core.reminders;
            let result = (|| -> AppResult<()> {
                let _native = core.store.native_actions.lock().map_err(err)?;
                if core.quitting.load(Ordering::Acquire) || service.stopped.load(Ordering::Acquire)
                {
                    return Ok(());
                }
                let mut engine = service.engine.lock().map_err(err)?;
                engine.tick(
                    &core.store,
                    service.clock()?,
                    windows::hidden(&handle)?,
                    &mut cursor,
                )?;
                {
                    if let Some(target) = engine.target(Surface::Rest) {
                        let label = target.rest_label();
                        let failure = core
                            .visibility
                            .lock()
                            .map_err(err)?
                            .failed
                            .get(&format!("{label}/{}", target.owner))
                            .cloned();
                        let vanished = service.rendered.lock().map_err(err)?.contains(&label)
                            && handle.get_webview_window(&label).is_none();
                        if let Some(error) =
                            failure.or_else(|| vanished.then(|| "专用显示窗口已关闭".into()))
                        {
                            if let Err(error) =
                                engine.failed(&core.store, &target, error, service.clock()?)
                            {
                                engine.cancel(Surface::Rest);
                                return Err(error);
                            }
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                if let Ok(mut engine) = service.engine.lock() {
                    engine.clear();
                }
                core.report(format!("专用提醒后台：{error}"));
            }
            service.schedule(&handle);
            if service
                .island_window
                .lock()
                .ok()
                .as_deref()
                .and_then(Option::as_ref)
                .is_some_and(|view| view.expired(Instant::now()))
            {
                windows::dispatch(&handle, windows::release_idle_island);
            }
        });
    }
}
#[cfg(windows)]
fn cursor() -> AppResult<(i32, i32)> {
    let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok((point.x, point.y))
}
#[cfg(not(windows))]
fn cursor() -> AppResult<(i32, i32)> {
    Err("当前系统不支持久坐鼠标检测".into())
}

fn request(
    service: &Service,
    store: &Store,
    sessions: &widgets::Sessions,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
    clock: Clock,
) -> AppResult<Value> {
    surface_method(window, method)?;
    if service.stopped.load(Ordering::Acquire) {
        return Err("SESSION_EXPIRED: 提醒后台已退出".into());
    }
    if method == "sit.break.close" {
        // 关闭当前休息页只清内存，不依赖数据库可写，避免Esc被保存故障困住。
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Close {
            id: String,
        }
        let args: Close = serde_json::from_value(args).map_err(err)?;
        let sessions = sessions.0.lock().map_err(err)?;
        let session = sessions
            .get(token)
            .filter(|session| session.window == window && session.preview.is_none())
            .ok_or("SESSION_EXPIRED: 休息页会话失效")?;
        let mut engine = service.engine.lock().map_err(err)?;
        let target = engine
            .target(Surface::Rest)
            .filter(|target| {
                target.owner == session.instance
                    && target.rest_label() == window
                    && target.event == args.id
            })
            .ok_or("SESSION_EXPIRED: 休息页已关闭或替换")?;
        engine.cancel(target.surface);
        return Ok(Value::Null);
    }
    if matches!(
        method,
        "island.get"
            | "island.configure"
            | "sit.get"
            | "sit.configure"
            | "phone.get"
            | "phone.configure"
    ) {
        return reminder_store::authorized_with(
            store,
            sessions,
            window,
            token,
            method,
            args,
            |id, _, mut value, changed| {
                let mut engine = service.engine.lock().map_err(err)?;
                if changed {
                    engine.invalidate(id);
                }
                value["runtime"] = engine.status(id);
                Ok(value)
            },
        );
    }
    let source = match method {
        "island.demo" => Source::Island,
        "sit.break.peek" => Source::Sit,
        "phone.preview" => Source::Phone,
        _ => return Err("CAPABILITY_DENIED: 未开放的专用提醒方法".into()),
    };
    let _native = store.native_actions.lock().map_err(err)?;
    widgets::with_capability(
        store,
        sessions,
        window,
        token,
        source.capability(),
        |session| {
            {
                let db = store.db.lock().map_err(err)?;
                let package: String = db
                    .query_row(
                        "SELECT package_id FROM widget_instances WHERE id=?1",
                        [&session.instance],
                        |row| row.get(0),
                    )
                    .map_err(err)?;
                if package != source.package() {
                    return Err("CAPABILITY_DENIED: 仅固定提醒组件可调用".into());
                }
            }
            if service.stopped.load(Ordering::Acquire) {
                return Err("SESSION_EXPIRED: 提醒后台已退出".into());
            }
            let mut engine = service.engine.lock().map_err(err)?;
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct DemoArgs {
                sample: Demo,
            }
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct PreviewArgs {
                preset: PhonePreset,
            }
            match method {
                "island.demo" => {
                    let args: DemoArgs = serde_json::from_value(args).map_err(err)?;
                    engine.demo(store, &session.instance, args.sample, clock)?;
                    return Ok(Value::Null);
                }
                "phone.preview" => {
                    let args: PreviewArgs = serde_json::from_value(args).map_err(err)?;
                    engine.preview(store, &session.instance, args.preset, clock)?;
                    return Ok(Value::Null);
                }
                _ => {}
            }
            if !args.is_null() && args.as_object().is_none_or(|args| !args.is_empty()) {
                return Err("INVALID_ARGUMENT: 休息页读取不接受参数".into());
            }
            let target = engine
                .target(Surface::Rest)
                .filter(|target| target.owner == session.instance && target.rest_label() == window)
                .ok_or("SESSION_EXPIRED: 当前显示代次已失效")?;
            engine.validate_target(store, &target, &target.event, clock)?;
            let rest = engine.rest().ok_or("SESSION_EXPIRED: 休息已关闭")?;
            Ok(json!({
                "id":rest.id, "sampledAt":clock.wall_ms,
                "remainingSeconds":rest.deadline.saturating_sub(clock.wall_ms).div_ceil(1000)
            }))
        },
    )
}
pub(crate) async fn gateway(
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
        let result = if method == "notifications.send" {
            (|| {
                let _native = core.store.native_actions.lock().map_err(err)?;
                if core.reminders.stopped.load(Ordering::Acquire) {
                    return Err("SESSION_EXPIRED: 提醒后台已退出".into());
                }
                let clock = core.reminders.clock()?;
                let hidden = windows::hidden(&handle)?;
                widgets::with_capability(
                    &core.store,
                    &core.sessions,
                    &window,
                    &token,
                    notification_sdk::SEND,
                    |session| {
                        core.reminders.engine.lock().map_err(err)?.send(
                            &core.store,
                            &session.instance,
                            notification_sdk::Request::parse(args)?,
                            clock,
                            hidden,
                            |source, notice| {
                                notification_sdk::windows(&handle, source, notice, clock.wall_ms)
                            },
                        )
                    },
                )
            })()
        } else {
            request(
                &core.reminders,
                &core.store,
                &core.sessions,
                &window,
                &token,
                &method,
                args,
                core.reminders.clock()?,
            )
        };
        core.reminders.schedule(&handle);
        result
    })
    .await
    .map_err(err)?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reminder_cached_page_cannot_prepare_a_replaced_notice() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-audio-ready-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let sessions = widgets::Sessions::default();
        let service = Service::default();
        let id = add_source(&store, Source::Island);
        let (label, token) = primary(&store, &sessions, &id);
        request(&service,&store,&sessions,&label,&token,"island.configure",json!({"requestId":uuid::Uuid::new_v4().to_string(),"expectedRevision":0,"config":{"enabled":true}}),clock(0)).unwrap();
        request(
            &service,
            &store,
            &sessions,
            &label,
            &token,
            "island.demo",
            json!({"sample":"call"}),
            clock(1),
        )
        .unwrap();
        let old = service.target(Surface::Notice).unwrap();
        assert_eq!(service.notice_snapshot(&old).unwrap()["displayed"], false);
        service
            .engine
            .lock()
            .unwrap()
            .demo(&store, &id, Demo::Info, clock(2))
            .unwrap();
        let current = service.target(Surface::Notice);
        assert!(require_notice(current.clone(), &old.event).is_err());
        assert!(service.notice_snapshot(&old).is_err());
        let current = current.unwrap();
        assert!(require_notice(Some(current.clone()), &current.event).is_ok());
        assert_eq!(
            service.notice_snapshot(&current).unwrap()["notification"]["id"],
            current.event
        );
    }
    #[test]
    fn reminder_cached_window_expires_only_after_thirty_seconds_idle() {
        let now = Instant::now();
        let mut cache = IslandWindow {
            label: "island-host-test".into(),
            idle_since: None,
        };
        assert!(!cache.expired(now + Duration::from_secs(60)));
        cache.idle_since = Some(now);
        assert!(!cache.expired(now + Duration::from_secs(29)));
        assert!(cache.expired(now + Duration::from_secs(30)));
    }
    use crate::{store::Store, widgets};
    use serde_json::{json, Value};
    use uuid::Uuid;
    fn add_source(store: &Store, source: Source) -> String {
        let manifest = json!({"schemaVersion":1,"id":source.package(),"name":"后台动作检查","version":"0.1.0","hostApiMajor":1,"entry":"dist/index.html","defaultSize":{"width":350,"height":180},"minSize":{"width":280,"height":140},"capabilities":[source.capability()],"settingsSchema":{"type":"object","additionalProperties":false,"properties":{}}});
        store.db.lock().unwrap().execute("INSERT OR IGNORE INTO widget_packages(id,manifest,directory,digest) VALUES(?1,?2,'action','action')",rusqlite::params![source.package(),manifest.to_string()]).unwrap();
        widgets::create_instance(store, source.package()).unwrap()
    }
    fn primary(store: &Store, sessions: &widgets::Sessions, id: &str) -> (String, String) {
        let label = format!("float-{id}");
        let session = widgets::open_display_session(store, sessions, &label, id, None).unwrap();
        (label, session["token"].as_str().unwrap().into())
    }
    fn clock(at: u64) -> Clock {
        Clock {
            wall_ms: 1_800_000_000_000 + at,
            monotonic_ms: at,
            local: LocalMinute {
                day: 100,
                weekday: 1,
                minute: 1,
                second: 0,
            },
        }
    }
    #[test]
    fn reminder_break_close_does_not_need_a_readable_database_or_allow_old_surface() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-break-close-{}", Uuid::new_v4())),
        )
        .unwrap();
        let sessions = widgets::Sessions::default();
        let service = Service::default();
        let island = add_source(&store, Source::Island);
        let sit = add_source(&store, Source::Sit);
        for (source, id) in [(Source::Island, &island), (Source::Sit, &sit)] {
            let (label, token) = primary(&store, &sessions, id);
            let mut config = source.default_config();
            config["enabled"] = json!(true);
            if source == Source::Sit {
                config["sitInterval"] = json!(1);
            }
            request(&service,&store,&sessions,&label,&token,if source==Source::Island{"island.configure"}else{"sit.configure"},json!({"requestId":Uuid::new_v4().to_string(),"expectedRevision":0,"config":config}),clock(0)).unwrap();
        }
        {
            let mut engine = service.engine.lock().unwrap();
            for n in 0..=6 {
                engine
                    .tick(&store, clock(n * 10000), false, &mut || Ok((n as i32, 0)))
                    .unwrap();
            }
            let target = engine.target(Surface::Notice).unwrap();
            engine
                .act(
                    &store,
                    &target,
                    &target.event,
                    Action::Confirm,
                    clock(60001),
                )
                .unwrap();
        }
        let target = service.target(Surface::Rest).unwrap();
        let label = target.rest_label();
        let session = widgets::open_display_session(&store, &sessions, &label, &sit, None).unwrap();
        let token = session["token"].as_str().unwrap();
        store
            .db
            .lock()
            .unwrap()
            .execute_batch("ALTER TABLE widget_packages RENAME TO unavailable_packages")
            .unwrap();
        assert!(
            request(
                &service,
                &store,
                &sessions,
                &label,
                token,
                "sit.break.close",
                json!({"id":target.event}),
                clock(60002)
            )
            .is_ok(),
            "当前休息页关闭不能依赖数据库读取成功"
        );
        assert!(service.target(Surface::Rest).is_none());
        assert!(request(
            &service,
            &store,
            &sessions,
            &label,
            token,
            "sit.break.close",
            json!({"id":target.event}),
            clock(60003)
        )
        .is_err());
    }
    #[test]
    fn reminder_gateway_validates_phone_preview_without_legacy_container() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-reminder-gateway-{}", Uuid::new_v4())),
        )
        .unwrap();
        let sessions = widgets::Sessions::default();
        let service = Service::default();
        let id = add_source(&store, Source::Phone);
        let (label, token) = primary(&store, &sessions, &id);
        assert!(request(
            &service,
            &store,
            &sessions,
            &label,
            &token,
            "phone.preview",
            json!({"preset":"tea","audio":"https://example.com"}),
            clock(0)
        )
        .is_err());
        assert!(service.engine.lock().unwrap().notification().is_none());
        request(
            &service,
            &store,
            &sessions,
            &label,
            &token,
            "phone.preview",
            json!({"preset":"tea"}),
            clock(0),
        )
        .unwrap();
        assert_eq!(
            service
                .engine
                .lock()
                .unwrap()
                .notification()
                .unwrap()
                .preset,
            Some(PhonePreset::Tea)
        );
        service.stop();
        assert!(request(
            &service,
            &store,
            &sessions,
            &label,
            &token,
            "phone.get",
            Value::Null,
            clock(1)
        )
        .unwrap_err()
        .contains("已退出"));
    }
    #[test]
    fn reminder_surface_roles_have_only_fixed_read_and_current_event_actions() {
        for method in [
            "runtime.ready",
            "island.peek",
            "island.demo",
            "notifications.send",
            "settings.get",
        ] {
            assert!(surface_method("island-host-1", method).is_err());
        }
        for method in ["runtime.ready", "sit.break.peek", "sit.break.close"] {
            assert!(surface_method("break-id-1", method).is_ok());
        }
        for method in ["sit.configure", "island.action", "storage.get"] {
            assert!(surface_method("break-id-1", method).is_err());
        }
    }
    #[test]
    fn reminder_surface_label_requires_matching_instance_and_positive_generation() {
        let id = Uuid::new_v4().to_string();
        assert_eq!(
            surface_role(&format!("break-{id}-9"), &id).unwrap().1,
            "break-overlay"
        );
        for label in [
            format!("break-{id}-0"),
            format!("break-{id}-01"),
            format!("break-{id}-oops"),
            format!("island-{id}-1"),
            "island-another-1".into(),
        ] {
            assert!(surface_role(&label, &id).is_none());
        }
    }
    #[test]
    fn reminder_legacy_control_card_remains_configurable_without_overlay_session() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-reminder-role-{}", Uuid::new_v4())),
        )
        .unwrap();
        let sessions = widgets::Sessions::default();
        let service = Service::default();
        let id = add_source(&store, Source::Island);
        let (label, token) = primary(&store, &sessions, &id);
        request(&service, &store, &sessions, &label, &token, "island.configure",
            json!({"requestId":Uuid::new_v4().to_string(),"expectedRevision":0,"config":{"enabled":true}}), clock(0)).unwrap();
        request(
            &service,
            &store,
            &sessions,
            &label,
            &token,
            "island.demo",
            json!({"sample":"success"}),
            clock(1),
        )
        .unwrap();
        let target = service.target(Surface::Notice).unwrap();
        assert_eq!(target.owner, HOST_NOTICE_OWNER);
        assert!(
            widgets::open_display_session(&store, &sessions, "island-host-test", &id, None)
                .is_err()
        );
        assert_eq!(sessions.0.lock().unwrap().len(), 1);
    }
}
