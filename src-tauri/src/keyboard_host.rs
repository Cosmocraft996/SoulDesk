//! 固定键盘演示能力、消费者与只读叠加层协调；不提供任意系统键盘API。
use crate::{
    key_capture::Capture,
    keyboard::Snapshot,
    store::{err, AppResult, Store},
    widgets::{self, Sessions},
    windows, Core,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
pub(crate) const PACKAGE: &str = "local.souldesk.widgetjs.key-stroke";
const CAPABILITY: &str = "keyboard.display";
#[derive(Default)]
struct Runtime {
    owner: Option<String>,
    generation: u64,
    ready: bool,
    listening: bool,
    show: bool,
    snapshot: Snapshot,
    error: Option<String>,
}
#[derive(Default)]
pub(crate) struct Service {
    runtime: Mutex<Runtime>,
    epoch: AtomicU64,
    dirty: AtomicBool,
    stopped: AtomicBool,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}
pub(crate) fn enabled(db: &Connection, id: &str) -> AppResult<bool> {
    let value: Option<String> = db
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [format!("keyboard.enabled/{id}")],
            |row| row.get(0),
        )
        .optional()
        .map_err(err)?;
    match value.as_deref() {
        None | Some("false") => Ok(false),
        Some("true") => Ok(true),
        _ => Err("DATA_UNAVAILABLE: 键盘演示开关无效".into()),
    }
}
fn owners(store: &Store) -> AppResult<Vec<String>> {
    let db = store.db.lock().map_err(err)?;
    let mut stmt=db.prepare("SELECT i.id,p.manifest FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE p.id=?1 ORDER BY i.id").map_err(err)?;
    let rows = stmt
        .query_map([PACKAGE], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(err)?;
    let mut result = Vec::new();
    for row in rows {
        let (id, manifest) = row.map_err(err)?;
        let manifest: widgets::Manifest = serde_json::from_str(&manifest).map_err(err)?;
        if manifest.capabilities.iter().any(|cap| cap == CAPABILITY) && enabled(&db, &id)? {
            result.push(id);
        }
    }
    Ok(result)
}
fn choose_owner(current: Option<&str>, owners: &[String], suspended: bool) -> Option<String> {
    if suspended {
        None
    } else {
        current
            .filter(|id| owners.iter().any(|other| other == id))
            .map(str::to_owned)
            .or_else(|| owners.first().cloned())
    }
}
pub(crate) fn overlay_method(window: &str, method: &str) -> AppResult<()> {
    if window.starts_with("keyboard-")
        && !matches!(
            method,
            "runtime.ready" | "keyboard.peek" | "keyboard.status"
        )
    {
        return Err("CAPABILITY_DENIED: 键盘叠加层只读".into());
    }
    Ok(())
}
fn access(
    service: &Service,
    store: &Store,
    sessions: &Sessions,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    overlay_method(window, method)?;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Change {
        enabled: bool,
    }
    let change = match method {
        "keyboard.configure" => Some(serde_json::from_value::<Change>(args).map_err(err)?),
        "keyboard.status" | "keyboard.peek"
            if args.is_null() || args.as_object().is_some_and(|value| value.is_empty()) =>
        {
            None
        }
        _ => return Err("INVALID_ARGUMENT: 键盘演示方法或参数无效".into()),
    };
    // 从鉴权到开关写入保持同一会话/数据库锁顺序，失效会话不能重新授权。
    let sessions = sessions.0.lock().map_err(err)?;
    let session = sessions
        .get(token)
        .filter(|s| s.window == window && s.preview.is_none())
        .ok_or("SESSION_EXPIRED: 键盘演示无有效正式会话")?;
    let db = store.db.lock().map_err(err)?;
    let(package,manifest):(String,String)=db.query_row("SELECT p.id,p.manifest FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE i.id=?1",[&session.instance],|row|Ok((row.get(0)?,row.get(1)?))).map_err(err)?;
    let manifest: widgets::Manifest = serde_json::from_str(&manifest).map_err(err)?;
    if package != PACKAGE || !manifest.capabilities.iter().any(|cap| cap == CAPABILITY) {
        return Err("CAPABILITY_DENIED: 未声明固定键盘演示能力".into());
    }
    let mut active = enabled(&db, &session.instance)?;
    let suspended = db
        .query_row(
            "SELECT value FROM app_settings WHERE key='hidden_all'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(err)?
        .as_deref()
        == Some("true");
    if method == "keyboard.peek" {
        if window != format!("keyboard-{}", session.instance) || !active || suspended {
            return Err("CAPABILITY_DENIED: 仅已启用的专用叠加层可读快捷键".into());
        }
        let state = service.runtime.lock().map_err(err)?;
        return serde_json::to_value(
            if state.owner.as_deref() == Some(&session.instance)
                && state.generation == service.epoch.load(Ordering::Acquire)
            {
                state.snapshot.clone()
            } else {
                Snapshot::default()
            },
        )
        .map_err(err);
    }
    if let Some(change) = change {
        db.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("keyboard.enabled/{}",session.instance),if change.enabled{"true"}else{"false"}]).map_err(err)?;
        active = change.enabled;
        let runtime = service.runtime.lock().map_err(err)?;
        if (!active && runtime.owner.as_deref() == Some(&session.instance))
            || (active && runtime.error.is_some())
        {
            service.epoch.fetch_add(1, Ordering::AcqRel);
        }
        service.dirty.store(true, Ordering::Release);
    }
    let state = service.runtime.lock().map_err(err)?;
    Ok(
        json!({"enabled":active,"listening":active&&!suspended&&state.listening,"suspended":active&&suspended,"lastError":if active{state.error.as_ref()}else{None}}),
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
        access(
            &core.keyboard,
            &core.store,
            &core.sessions,
            &window,
            &token,
            &method,
            args,
        )
    })
    .await
    .map_err(err)?
}
impl Service {
    pub(crate) fn current(&self, owner: &str, generation: u64) -> bool {
        !self.stopped.load(Ordering::Acquire)
            && self.epoch.load(Ordering::Acquire) == generation
            && self.runtime.lock().is_ok_and(|state| {
                state.owner.as_deref() == Some(owner) && state.generation == generation
            })
    }
    pub(crate) fn wants_show(&self, owner: &str, generation: u64) -> bool {
        self.current(owner, generation) && self.runtime.lock().is_ok_and(|state| state.show)
    }
    fn completed(&self, owner: &str, generation: u64, result: AppResult<()>) {
        if self.epoch.load(Ordering::Acquire) != generation {
            return;
        }
        if let Ok(mut state) = self.runtime.lock() {
            if state.owner.as_deref() == Some(owner) && state.generation == generation {
                match result {
                    Ok(()) => state.ready = true,
                    Err(error) => {
                        state.error = Some(error);
                        state.ready = false;
                        state.listening = false;
                        state.snapshot = Snapshot::default();
                    }
                }
            }
        }
    }
    pub(crate) fn start(&self, app: &AppHandle) -> AppResult<()> {
        let mut slot = self.thread.lock().map_err(err)?;
        if slot.is_some() {
            return Ok(());
        }
        let handle = app.clone();
        *slot = Some(
            std::thread::Builder::new()
                .name("keyboard-coordinator".into())
                .spawn(move || run(handle))
                .map_err(err)?,
        );
        Ok(())
    }
    pub(crate) fn stop(&self) -> AppResult<()> {
        self.stopped.store(true, Ordering::Release);
        self.epoch.fetch_add(1, Ordering::AcqRel);
        let thread = self.thread.lock().map_err(err)?.take();
        if let Some(thread) = thread {
            thread
                .join()
                .map_err(|_| "键盘演示后台未正常退出".to_owned())?;
        }
        let mut state = self.runtime.lock().map_err(err)?;
        *state = Runtime::default();
        Ok(())
    }
}
fn run(app: AppHandle) {
    let mut capture: Option<Capture> = None;
    let mut owner: Option<String> = None;
    let mut generation = 0;
    let mut candidates = Vec::new();
    let mut last_read = Instant::now() - Duration::from_secs(1);
    let mut was_visible = false;
    let mut hide_at: Option<Instant> = None;
    let mut failure_closed = false;
    loop {
        let core = app.state::<Core>();
        let service = &core.keyboard;
        if core.quitting.load(Ordering::Acquire) || service.stopped.load(Ordering::Acquire) {
            break;
        }
        if last_read.elapsed() >= Duration::from_secs(1)
            || service.dirty.swap(false, Ordering::AcqRel)
        {
            match owners(&core.store) {
                Ok(value) => candidates = value,
                Err(error) => {
                    candidates.clear();
                    if let Ok(mut state) = service.runtime.lock() {
                        state.error = Some(error);
                    }
                }
            }
            last_read = Instant::now();
        }
        let suspended = windows::hidden(&app).unwrap_or(true);
        let desired = choose_owner(owner.as_deref(), &candidates, suspended);
        if desired != owner || generation != service.epoch.load(Ordering::Acquire) {
            if desired.is_none() {
                drop(capture.take());
            }
            owner = desired;
            generation = service.epoch.fetch_add(1, Ordering::AcqRel) + 1;
            if let Ok(mut state) = service.runtime.lock() {
                *state = Runtime {
                    owner: owner.clone(),
                    generation,
                    ..Default::default()
                };
            }
            was_visible = false;
            hide_at = None;
            failure_closed = false;
            let next = owner.clone();
            let at = generation;
            windows::dispatch(&app, move |app| {
                if app.state::<Core>().keyboard.epoch.load(Ordering::Acquire) != at {
                    return Ok(());
                }
                if let Some(id) = next {
                    // 当前代次清理全部旧叠加层；前一代次跳过的清理不能留下同标签窗口。
                    let result = windows::keyboard_close_all(app)
                        .and_then(|()| windows::keyboard_create(app, &id, at));
                    app.state::<Core>().keyboard.completed(&id, at, result);
                } else {
                    windows::keyboard_close_all(app)?;
                }
                Ok(())
            });
        }
        if let Some(id) = owner.as_deref() {
            let initialized = service
                .runtime
                .lock()
                .is_ok_and(|state| state.ready && state.error.is_none());
            if initialized {
                let failed =
                    core.visibility.lock().ok().and_then(|state| {
                        state.failed.get(&format!("keyboard-{id}/{id}")).cloned()
                    });
                if let Some(error) = failed {
                    service.completed(id, generation, Err(format!("键盘演示显示失败：{error}")));
                } else if app.get_webview_window(&format!("keyboard-{id}")).is_none() {
                    service.completed(
                        id,
                        generation,
                        Err("键盘演示叠加层已关闭，请显式重试".into()),
                    );
                }
            }
            let failed = service
                .runtime
                .lock()
                .is_ok_and(|state| state.error.is_some());
            if failed && !failure_closed {
                drop(capture.take());
                failure_closed = true;
                let id = id.to_owned();
                let at = generation;
                windows::dispatch(&app, move |app| {
                    if app.state::<Core>().keyboard.current(&id, at) {
                        windows::keyboard_close(app, &id)?;
                    }
                    Ok(())
                });
            }
            let ready = service
                .runtime
                .lock()
                .is_ok_and(|state| state.ready && state.error.is_none());
            if ready && capture.is_none() {
                match Capture::start() {
                    Ok(value) => capture = Some(value),
                    Err(error) => service.completed(id, generation, Err(error)),
                }
            }
            if let Some(worker) = capture.as_ref() {
                let event = worker.receiver.borrow().clone();
                if let Some(error) = event.error.or_else(|| {
                    worker
                        .receiver
                        .has_changed()
                        .is_err()
                        .then(|| "键盘演示监听线程意外结束，请显式重试".into())
                }) {
                    service.completed(id, generation, Err(error));
                    drop(capture.take());
                } else if let Ok(mut state) = service.runtime.lock() {
                    if state.generation == generation
                        && service.epoch.load(Ordering::Acquire) == generation
                    {
                        state.listening = event.ready;
                        state.snapshot = event.snapshot;
                        if state.snapshot.visible {
                            hide_at = None;
                        } else if was_visible {
                            hide_at = Some(Instant::now() + Duration::from_millis(320));
                        }
                        was_visible = state.snapshot.visible;
                        let show =
                            state.snapshot.visible || hide_at.is_some_and(|at| Instant::now() < at);
                        if state.show != show {
                            state.show = show;
                            let id = id.to_owned();
                            let at = generation;
                            windows::dispatch(&app, move |app| {
                                let result = windows::keyboard_visibility(app, &id, at);
                                if let Err(error) = result {
                                    app.state::<Core>().keyboard.completed(&id, at, Err(error));
                                }
                                Ok(())
                            });
                        }
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    drop(capture);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_any_enabled_consumer_can_retry_a_shared_failure_without_disabling_others() {
        let (store, sessions, service, first, _) = fixture();
        let second = widgets::create_instance(&store, PACKAGE).unwrap();
        let a = primary(&store, &sessions, &first);
        let b = primary(&store, &sessions, &second);
        for (id, session) in [(&first, &a), (&second, &b)] {
            access(
                &service,
                &store,
                &sessions,
                &format!("float-{id}"),
                session["token"].as_str().unwrap(),
                "keyboard.configure",
                json!({"enabled":true}),
            )
            .unwrap();
        }
        service.epoch.store(1, Ordering::Release);
        *service.runtime.lock().unwrap() = Runtime {
            owner: Some(first.clone()),
            generation: 1,
            error: Some("监听初始化失败".into()),
            ..Default::default()
        };
        access(
            &service,
            &store,
            &sessions,
            &format!("float-{second}"),
            b["token"].as_str().unwrap(),
            "keyboard.configure",
            json!({"enabled":true}),
        )
        .unwrap();
        assert_eq!(
            service.epoch.load(Ordering::Acquire),
            2,
            "非所有者的显式重试必须使共享服务进入新代次"
        );
        *service.runtime.lock().unwrap() = Runtime {
            owner: Some(first.clone()),
            generation: 2,
            listening: true,
            ..Default::default()
        };
        access(
            &service,
            &store,
            &sessions,
            &format!("float-{second}"),
            b["token"].as_str().unwrap(),
            "keyboard.configure",
            json!({"enabled":false}),
        )
        .unwrap();
        assert_eq!(
            service.epoch.load(Ordering::Acquire),
            2,
            "停用其他消费者不能重启仍有效的监听器"
        );
        assert!(enabled(&store.db.lock().unwrap(), &first).unwrap());
    }
    fn fixture() -> (Store, Sessions, Service, String, widgets::StagedPackage) {
        use std::io::{Cursor, Write};
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-keyboard-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(include_bytes!(
            "../component-assets/key-stroke-manifest.json"
        ))
        .unwrap();
        zip.start_file("dist/index.html", options).unwrap();
        zip.write_all(b"<!doctype html><title>keyboard fixture</title>")
            .unwrap();
        let package = widgets::stage_bytes(&store, zip.finish().unwrap().into_inner()).unwrap();
        widgets::activate(&store, &package).unwrap();
        let id = widgets::create_instance(&store, PACKAGE).unwrap();
        (store, Sessions::default(), Service::default(), id, package)
    }
    fn primary(store: &Store, sessions: &Sessions, id: &str) -> Value {
        widgets::open_display_session(store, sessions, &format!("float-{id}"), id, None).unwrap()
    }
    #[test]
    fn keyboard_configuration_is_opt_in_scoped_strict_and_window_bound() {
        let (store, sessions, service, id, _) = fixture();
        let label = format!("float-{id}");
        let session = primary(&store, &sessions, &id);
        let token = session["token"].as_str().unwrap();
        assert_eq!(
            access(
                &service,
                &store,
                &sessions,
                &label,
                token,
                "keyboard.status",
                Value::Null
            )
            .unwrap()["enabled"],
            false
        );
        assert!(owners(&store).unwrap().is_empty());
        assert!(access(
            &service,
            &store,
            &sessions,
            "other-window",
            token,
            "keyboard.configure",
            json!({"enabled":true})
        )
        .is_err());
        assert!(access(
            &service,
            &store,
            &sessions,
            &label,
            token,
            "keyboard.configure",
            json!({"enabled":true,"instanceId":"other"})
        )
        .is_err());
        assert!(access(
            &service,
            &store,
            &sessions,
            &label,
            token,
            "keyboard.configure",
            json!({"enabled":"true"})
        )
        .is_err());
        assert_eq!(
            access(
                &service,
                &store,
                &sessions,
                &label,
                token,
                "keyboard.configure",
                json!({"enabled":true})
            )
            .unwrap()["enabled"],
            true
        );
        assert_eq!(owners(&store).unwrap(), vec![id.clone()]);
        let second = widgets::create_instance(&store, PACKAGE).unwrap();
        assert!(!enabled(&store.db.lock().unwrap(), &second).unwrap());
        assert!(access(
            &service,
            &store,
            &sessions,
            &label,
            token,
            "keyboard.peek",
            Value::Null
        )
        .is_err());
    }
    #[test]
    fn keyboard_overlay_and_primary_coexist_while_hidden_control_is_not_a_stop() {
        let (store, sessions, service, id, _) = fixture();
        let label = format!("float-{id}");
        let session = primary(&store, &sessions, &id);
        let token = session["token"].as_str().unwrap();
        let overlay = format!("keyboard-{id}");
        assert!(widgets::open_display_session(&store, &sessions, &overlay, &id, None).is_err());
        access(
            &service,
            &store,
            &sessions,
            &label,
            token,
            "keyboard.configure",
            json!({"enabled":true}),
        )
        .unwrap();
        let extra = widgets::open_display_session(&store, &sessions, &overlay, &id, None).unwrap();
        assert_eq!(extra["role"], "keyboard-overlay");
        let overlay_token = extra["token"].as_str().unwrap();
        assert_eq!(sessions.0.lock().unwrap().len(), 2);
        primary(&store, &sessions, &id);
        assert!(sessions.0.lock().unwrap().contains_key(overlay_token));
        assert!(widgets::gateway(
            &store,
            &sessions,
            &overlay,
            overlay_token,
            "settings.get",
            Value::Null
        )
        .is_err());
        assert!(widgets::require_capability(
            &store,
            &sessions,
            &overlay,
            overlay_token,
            CAPABILITY
        )
        .is_err());
        store.presentation(&id, "desktop", false, false).unwrap();
        assert!(widgets::open_display_session(&store, &sessions, &overlay, &id, None).is_ok());
        assert_eq!(owners(&store).unwrap(), vec![id.clone()]);
        store.save_preference("hidden_all", "true").unwrap();
        assert!(widgets::open_display_session(&store, &sessions, &overlay, &id, None).is_err());
    }
    #[test]
    fn keyboard_preview_cannot_enable_capture() {
        let (store, sessions, service, id, package) = fixture();
        sessions.0.lock().unwrap().insert(
            "preview-token".into(),
            widgets::Session {
                instance: id,
                package_digest: None,
                window: "trial-keyboard".into(),
                preview: Some(std::sync::Arc::new(Mutex::new(widgets::Preview {
                    package,
                    settings: json!({}),
                    storage: json!({}),
                    ready_since: None,
                    failure: None,
                }))),
                shared_cursor: None,
            },
        );
        assert!(access(
            &service,
            &store,
            &sessions,
            "trial-keyboard",
            "preview-token",
            "keyboard.configure",
            json!({"enabled":true})
        )
        .is_err());
        assert!(owners(&store).unwrap().is_empty());
        assert!(service.thread.lock().unwrap().is_none());
    }
    #[test]
    fn keyboard_disable_invalidates_queued_show_and_old_overlay_reads() {
        let (store, sessions, service, id, _) = fixture();
        let label = format!("float-{id}");
        let session = primary(&store, &sessions, &id);
        let token = session["token"].as_str().unwrap();
        access(
            &service,
            &store,
            &sessions,
            &label,
            token,
            "keyboard.configure",
            json!({"enabled":true}),
        )
        .unwrap();
        let overlay = format!("keyboard-{id}");
        let extra = widgets::open_display_session(&store, &sessions, &overlay, &id, None).unwrap();
        service.epoch.store(1, Ordering::Release);
        *service.runtime.lock().unwrap() = Runtime {
            owner: Some(id.clone()),
            generation: 1,
            show: true,
            ..Default::default()
        };
        assert!(service.wants_show(&id, 1));
        access(
            &service,
            &store,
            &sessions,
            &label,
            token,
            "keyboard.configure",
            json!({"enabled":false}),
        )
        .unwrap();
        assert!(!service.current(&id, 1));
        assert!(!service.wants_show(&id, 1));
        assert!(access(
            &service,
            &store,
            &sessions,
            &overlay,
            extra["token"].as_str().unwrap(),
            "keyboard.peek",
            Value::Null
        )
        .is_err());
        assert!(owners(&store).unwrap().is_empty());
    }
    #[test]
    fn keyboard_owner_retained_and_other_consumers_survive() {
        let ids = vec!["a".into(), "b".into()];
        assert_eq!(choose_owner(Some("b"), &ids, false).as_deref(), Some("b"));
        assert_eq!(
            choose_owner(Some("a"), &ids[1..], false).as_deref(),
            Some("b")
        );
        assert_eq!(choose_owner(Some("a"), &ids, true), None);
        assert_eq!(choose_owner(None, &[], false), None);
    }
    #[test]
    fn keyboard_overlay_only_has_fixed_read_methods() {
        for method in ["runtime.ready", "keyboard.peek", "keyboard.status"] {
            assert!(overlay_method("keyboard-id", method).is_ok());
        }
        for method in [
            "keyboard.configure",
            "storage.get",
            "settings.update",
            "view.openDetails",
            "market.subscribe",
        ] {
            assert!(overlay_method("keyboard-id", method).is_err());
            assert!(overlay_method("float-id", method).is_ok());
        }
    }
}
