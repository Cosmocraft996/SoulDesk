//! 固定剪切板组件的会话网关与后台消费者；不接受页面传入实例标识。
use crate::{
    clipboard_actions,
    clipboard_capture::{Consumer, Gate, Reader, SystemClipboard},
    clipboard_history,
    clipboard_native::{self, Target},
    clipboard_shortcuts::{Driver, Registry, Route, Spec},
    clipboard_surface::{self, Book, Kind, Phase, Popup},
    store::{err, now_ms, AppResult, Store},
    widgets, Core,
};
use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

pub(crate) const HISTORY: &str = "local.souldesk.widgetjs.clipboard";
pub(crate) const SEARCH: &str = "local.souldesk.widgetjs.clipboard-search";
pub(crate) const CAPABILITY: &str = "clipboard.manage";
pub(crate) const CONTROL: &str = "clipboard.control";
pub(crate) const HISTORY_ACCESS: &str = "clipboard.history";
pub(crate) const COPY: &str = "clipboard.copy";
pub(crate) const PASTE: &str = "clipboard.paste";
pub(crate) const SEARCH_ACCESS: &str = "clipboard.search";
pub(crate) const TOOLS: &str = "clipboard.tools";

pub(crate) fn grants(capabilities: &[String], operation: &str) -> bool {
    capabilities
        .iter()
        .any(|cap| cap == CAPABILITY || cap == operation)
}

pub(crate) fn validate_permissions(package: &str, capabilities: &[String]) -> AppResult<()> {
    let scoped = [CONTROL, HISTORY_ACCESS, COPY, PASTE, SEARCH_ACCESS, TOOLS];
    let has = |name: &str| capabilities.iter().any(|cap| cap == name);
    if !scoped.into_iter().any(has) {
        return Ok(());
    }
    let allowed = match package {
        HISTORY => &[CONTROL, HISTORY_ACCESS, COPY, PASTE][..],
        SEARCH => &[CONTROL, SEARCH_ACCESS, TOOLS][..],
        _ => return Err("CAPABILITY_DENIED: 剪切板权限仅供对应固定组件使用".into()),
    };
    if has(CAPABILITY)
        || !has(CONTROL)
        || scoped.iter().any(|cap| has(cap) && !allowed.contains(cap))
        || (has(PASTE) && !has(COPY))
    {
        return Err("CAPABILITY_DENIED: 剪切板细分权限组合无效".into());
    }
    Ok(())
}

fn with_permission<T>(
    store: &Store,
    sessions: &widgets::Sessions,
    window: &str,
    token: &str,
    operation: &str,
    action: impl FnOnce(&mut widgets::Session) -> AppResult<T>,
) -> AppResult<T> {
    widgets::with_any_capability(
        store,
        sessions,
        window,
        token,
        &[CAPABILITY, operation],
        action,
    )
}

fn action_permission(paste: bool) -> &'static str {
    if paste {
        PASTE
    } else {
        COPY
    }
}

struct Source {
    consumer: Consumer,
    kind: Kind,
    state: Value,
    can_observe: bool,
}
#[derive(Default)]
struct Live {
    revision: u64,
    kind: Kind,
    listening: bool,
    text: String,
    serial: u64,
    updated_at: u64,
    last_error: Option<String>,
    pending_open: bool,
    shortcut_registered: bool,
    shortcut_error: Option<String>,
}
#[derive(Default)]
struct Runtime {
    entries: HashMap<String, Live>,
    error: Option<String>,
}
#[derive(Default)]
pub(crate) struct Service {
    actions: clipboard_actions::Engine,
    surfaces: Mutex<Book>,
    shortcuts: Mutex<Registry>,
    ui_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    gate: Mutex<Gate>,
    runtime: Mutex<Runtime>,
    stopped: AtomicBool,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}
enum ShortcutAction {
    Show(Popup),
    Hide(String),
    Search(url::Url),
}
struct AppDriver<'a>(&'a AppHandle);
impl Driver for AppDriver<'_> {
    fn register(&mut self, key: tauri_plugin_global_shortcut::Shortcut) -> AppResult<()> {
        self.0.global_shortcut().register(key).map_err(err)
    }
    fn unregister(&mut self, key: tauri_plugin_global_shortcut::Shortcut) -> AppResult<()> {
        self.0.global_shortcut().unregister(key).map_err(err)
    }
}
fn search_url(platform: &str, text: &str) -> AppResult<url::Url> {
    if text.trim().is_empty() || text.contains('\0') || text.chars().take(50001).count() > 50000 {
        return Err("INVALID_ARGUMENT: 搜索内容须为1～50000字符且不含零字符".into());
    }
    let (base, param) = match platform {
        "google" => ("https://www.google.com/search", "q"),
        "bing" => ("https://cn.bing.com/search", "q"),
        "baidu" => ("https://www.baidu.com/s", "wd"),
        _ => return Err("INVALID_ARGUMENT: 搜索平台无效".into()),
    };
    let mut url = url::Url::parse(base).map_err(err)?;
    url.query_pairs_mut().append_pair(param, text);
    Ok(url)
}
fn source(store: &Store, id: &str) -> AppResult<Source> {
    let row: Option<(String,String)> = store.db.lock().map_err(err)?
        .query_row("SELECT p.id,p.manifest FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE i.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?)))
        .optional().map_err(|_| "STORAGE_ERROR: 剪切板实例读取失败")?;
    let (package, manifest) = row.ok_or("SESSION_EXPIRED: 剪切板实例已移除")?;
    let kind = match package.as_str() {
        HISTORY => Kind::History,
        SEARCH => Kind::Search,
        _ => return Err("CAPABILITY_DENIED: 仅固定剪切板组件可访问此能力".into()),
    };
    let manifest: widgets::Manifest =
        serde_json::from_str(&manifest).map_err(|_| "DATA_UNAVAILABLE: 剪切板清单无效")?;
    if !grants(&manifest.capabilities, CONTROL) {
        return Err("CAPABILITY_DENIED: 未声明固定剪切板能力".into());
    }
    let mut state = clipboard_history::status(store, id)?;
    if kind == Kind::Search && state["configRevision"] == 0 {
        state["shortcut"] = json!("Super+Alt+S");
    }
    Ok(Source {
        consumer: Consumer {
            instance: id.into(),
            config_revision: state["configRevision"]
                .as_u64()
                .ok_or("DATA_UNAVAILABLE: 剪切板配置版本无效")?,
        },
        kind,
        state,
        can_observe: grants(
            &manifest.capabilities,
            if kind == Kind::History {
                HISTORY_ACCESS
            } else {
                SEARCH_ACCESS
            },
        ),
    })
}
fn candidates(store: &Store) -> AppResult<Vec<String>> {
    let db = store.db.lock().map_err(err)?;
    let mut stmt=db.prepare("SELECT i.id FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE p.id IN (?1,?2) ORDER BY i.id")
        .map_err(|_| "STORAGE_ERROR: 剪切板消费者读取失败")?;
    let rows = stmt
        .query_map([HISTORY, SEARCH], |r| r.get::<_, String>(0))
        .map_err(|_| "STORAGE_ERROR: 剪切板消费者读取失败")?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|_| "STORAGE_ERROR: 剪切板消费者读取失败".into())
}
fn empty(args: &Value) -> AppResult<()> {
    if args.is_null() || args.as_object().is_some_and(|v| v.is_empty()) {
        Ok(())
    } else {
        Err("INVALID_ARGUMENT: 此剪切板方法不接受参数".into())
    }
}
fn hidden(store: &Store) -> AppResult<bool> {
    let value: Option<String> = store
        .db
        .lock()
        .map_err(err)?
        .query_row(
            "SELECT value FROM app_settings WHERE key='hidden_all'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "STORAGE_ERROR: 剪切板显隐状态不可用")?;
    match value.as_deref() {
        None | Some("false") => Ok(false),
        Some("true") => Ok(true),
        _ => Err("DATA_UNAVAILABLE: 剪切板显隐状态无效".into()),
    }
}
impl Service {
    fn shortcuts_with(&self, store: &Store, driver: &mut impl Driver) -> AppResult<()> {
        if self.stopped.load(Ordering::Acquire) {
            return self.shortcuts.lock().map_err(err)?.release(driver);
        }
        let mut sources = Vec::new();
        for id in candidates(store)? {
            if let Ok(s) = source(store, &id) {
                sources.push(s);
            }
        }
        let specs = sources
            .iter()
            .filter(|s| s.state["shortcutEnabled"] == true)
            .map(|s| Spec {
                instance: s.consumer.instance.clone(),
                revision: s.consumer.config_revision,
                shortcut: s.state["shortcut"].as_str().unwrap_or("").into(),
            })
            .collect::<Vec<_>>();
        let primary = crate::panel_shortcut(store);
        let states = self
            .shortcuts
            .lock()
            .map_err(err)?
            .sync(&specs, &primary, driver);
        let mut runtime = self.runtime.lock().map_err(err)?;
        if self.stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        for s in sources {
            let live = runtime
                .entries
                .entry(s.consumer.instance.clone())
                .or_insert_with(|| Live {
                    revision: s.consumer.config_revision,
                    kind: s.kind,
                    ..Default::default()
                });
            if live.revision == s.consumer.config_revision {
                live.shortcut_registered = states
                    .get(&s.consumer.instance)
                    .is_some_and(|s| s.registered);
                live.shortcut_error = states
                    .get(&s.consumer.instance)
                    .and_then(|s| s.error.clone());
            }
        }
        Ok(())
    }
    fn shortcut_source(&self, store: &Store, spec: &Spec) -> AppResult<Source> {
        let current = source(store, &spec.instance)?;
        if self.stopped.load(Ordering::Acquire)
            || hidden(store)?
            || current.state["shortcutEnabled"] != true
            || current.consumer.config_revision != spec.revision
            || current.state["shortcut"] != spec.shortcut
        {
            return Err("PRESENTATION_EXPIRED: 快捷键所属配置已失效".into());
        }
        Ok(current)
    }
    fn shortcut_action(
        &self,
        store: &Store,
        spec: &Spec,
        target: Option<Target>,
    ) -> AppResult<ShortcutAction> {
        let _native = store.native_actions.lock().map_err(err)?;
        let current = self.shortcut_source(store, spec)?;
        if current.kind == Kind::History {
            let existing = self
                .surfaces
                .lock()
                .map_err(err)?
                .entries
                .get(&spec.instance)
                .filter(|p| p.phase != Phase::Hidden && p.revision == spec.revision)
                .map(|p| p.label.clone());
            if let Some(label) = existing {
                self.closed(&label);
                return Ok(ShortcutAction::Hide(label));
            }
            return self
                .begin_popup(store, &spec.instance, target, None)
                .map(ShortcutAction::Show);
        }
        if !current.can_observe {
            return Err("CAPABILITY_DENIED: 未声明剪切板搜索权限".into());
        }
        let text = self
            .runtime
            .lock()
            .map_err(err)?
            .entries
            .get(&spec.instance)
            .filter(|live| {
                live.revision == spec.revision && current.state["captureEnabled"] == true
            })
            .map(|live| live.text.clone())
            .unwrap_or_default();
        search_url(
            current.state["searchPlatform"].as_str().unwrap_or("google"),
            &text,
        )
        .map(ShortcutAction::Search)
    }
    pub(crate) fn shortcut_route(&self, store: &Store, event: u32) -> Route {
        if self.stopped.load(Ordering::Acquire) {
            return Route::Ignore;
        }
        let primary = crate::panel_shortcut(store);
        self.shortcuts
            .try_lock()
            .map(|r| r.route(event, &primary))
            .unwrap_or(Route::Ignore)
    }
    pub(crate) fn shortcut_activated(
        &self,
        app: &AppHandle,
        spec: &Spec,
        target: Option<Target>,
    ) -> AppResult<()> {
        let core = app.state::<Core>();
        match self.shortcut_action(&core.store, spec, target)? {
            ShortcutAction::Show(p) => crate::windows::clipboard_show(app, &p),
            ShortcutAction::Hide(label) => crate::windows::clipboard_close(app, &label),
            ShortcutAction::Search(url) => {
                let _native = core.store.native_actions.lock().map_err(err)?;
                self.shortcut_source(&core.store, spec)?;
                crate::content::open_external(&url).map(|_| ())
            }
        }
    }
    pub(crate) fn stop_shortcuts(&self, app: &AppHandle) -> AppResult<()> {
        self.shortcuts
            .lock()
            .map_err(err)?
            .release(&mut AppDriver(app))
    }
    fn begin_popup(
        &self,
        store: &Store,
        id: &str,
        target: Option<Target>,
        automatic: Option<u64>,
    ) -> AppResult<Popup> {
        if self.stopped.load(Ordering::Acquire) || hidden(store)? {
            return Err("SESSION_EXPIRED: 剪切板显示已暂停".into());
        }
        let current = source(store, id)?;
        if let Some(revision) = automatic {
            if current.kind != Kind::Search
                || current.state["captureEnabled"] != true
                || current.consumer.config_revision != revision
            {
                return Err("PRESENTATION_EXPIRED: 自动搜索条所属配置已失效".into());
            }
        }
        if current.kind == Kind::History {
            clipboard_history::prune(store, id, now_ms())?;
        }
        self.surfaces.lock().map_err(err)?.begin(
            id,
            current.kind,
            current.consumer.config_revision,
            target,
            now_ms(),
            automatic.is_some(),
        )
    }
    fn prepare_popup(
        &self,
        store: &Store,
        id: &str,
        target: Option<Target>,
        automatic: Option<u64>,
    ) -> AppResult<Popup> {
        let _native = store.native_actions.lock().map_err(err)?;
        self.begin_popup(store, id, target, automatic)
    }
    fn request_popup(
        &self,
        store: &Store,
        sessions: &widgets::Sessions,
        window: &str,
        token: &str,
        target: Option<Target>,
    ) -> AppResult<Popup> {
        let _native = store.native_actions.lock().map_err(err)?;
        with_permission(store, sessions, window, token, CONTROL, |session| {
            self.begin_popup(store, &session.instance, target, None)
        })
    }
    pub(crate) fn current_window(&self, label: &str, instance: &str) -> bool {
        !self.stopped.load(Ordering::Acquire)
            && self
                .surfaces
                .lock()
                .is_ok_and(|book| book.current(label, instance))
    }
    pub(crate) fn closed(&self, label: &str) {
        if let Ok(mut book) = self.surfaces.lock() {
            book.close(label);
        }
    }
    pub(crate) fn focus_window(&self, label: &str, focused: bool) {
        if let Ok(mut book) = self.surfaces.lock() {
            let hover = book
                .entries
                .values()
                .find(|p| p.label == label)
                .map(|p| p.hovered);
            if let Some(hover) = hover {
                let _ = book.interact(label, focused, hover, now_ms());
            }
        }
    }
    pub(crate) fn attach_window(&self, store: &Store, p: &Popup, owner: usize) -> AppResult<()> {
        let _native = store.native_actions.lock().map_err(err)?;
        let current = source(store, &p.instance)?;
        if current.consumer.config_revision != p.revision || self.stopped.load(Ordering::Acquire) {
            return Err("PRESENTATION_EXPIRED: 建窗时配置已失效".into());
        }
        self.surfaces
            .lock()
            .map_err(err)?
            .attach(&p.instance, &p.id, owner)
    }
    pub(crate) fn show_window(
        &self,
        store: &Store,
        p: &Popup,
        show: impl FnOnce() -> AppResult<()>,
    ) -> AppResult<()> {
        let _native = store.native_actions.lock().map_err(err)?;
        let current = source(store, &p.instance)?;
        if current.consumer.config_revision != p.revision
            || self.stopped.load(Ordering::Acquire)
            || hidden(store)?
        {
            return Err("PRESENTATION_EXPIRED: 显示时配置已失效".into());
        }
        self.surfaces
            .lock()
            .map_err(err)?
            .shown(&p.instance, &p.id, now_ms())?;
        show()
    }
    fn bound_popup(
        &self,
        store: &Store,
        sessions: &widgets::Sessions,
        window: &str,
        token: &str,
        operation: &str,
    ) -> AppResult<Popup> {
        let _native = store.native_actions.lock().map_err(err)?;
        with_permission(store, sessions, window, token, operation, |session| {
            let current = source(store, &session.instance)?;
            if current.kind != Kind::History
                || clipboard_surface::role(window, &session.instance) != Some(Kind::History)
            {
                return Err("CAPABILITY_DENIED: 只能从本实例历史弹窗执行所选项".into());
            }
            self.surfaces
                .lock()
                .map_err(err)?
                .entries
                .get(&session.instance)
                .filter(|p| p.label == window && p.revision == current.consumer.config_revision)
                .cloned()
                .ok_or_else(|| "PRESENTATION_EXPIRED: 历史弹窗已替换".into())
        })
    }
    fn act_with(
        &self,
        store: &Store,
        sessions: &widgets::Sessions,
        window: &str,
        token: &str,
        request: clipboard_actions::Request,
        desktop: &mut impl clipboard_actions::Desktop,
    ) -> AppResult<Value> {
        let permission = action_permission(request.paste);
        let popup = self.bound_popup(store, sessions, window, token, permission)?;
        let id = popup.instance.clone();
        let request_id = request.request_id.clone();
        let presentation = request.presentation_id.clone();
        let result = self
            .actions
            .execute(store, &id, request, desktop, |stage, operation| {
                let _native = store.native_actions.lock().map_err(err)?;
                if self.stopped.load(Ordering::Acquire) {
                    return Err("SESSION_EXPIRED: 剪切板后台已退出".into());
                }
                with_permission(store, sessions, window, token, permission, |session| {
                    let current = source(store, &session.instance)?;
                    if session.instance != id
                        || current.kind != Kind::History
                        || clipboard_surface::role(window, &id) != Some(Kind::History)
                    {
                        return Err("CAPABILITY_DENIED: 动作来源无效".into());
                    }
                    let permit = self.surfaces.lock().map_err(err)?.claim(
                        &id,
                        window,
                        &presentation,
                        &request_id,
                        current.consumer.config_revision,
                        stage,
                        current.state["pasteEnabled"] == true,
                    )?;
                    operation(permit)
                })
            });
        self.surfaces
            .lock()
            .map_err(err)?
            .finish(&id, &presentation, &request_id);
        result
    }
    fn reconcile_windows(&self, store: &Store) -> AppResult<Vec<String>> {
        let _native = store.native_actions.lock().map_err(err)?;
        let ids = self
            .surfaces
            .lock()
            .map_err(err)?
            .entries
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let mut valid = HashMap::new();
        for id in ids {
            if let Ok(s) = source(store, &id) {
                valid.insert(id, s.consumer.config_revision);
            }
        }
        let hidden = self.stopped.load(Ordering::Acquire) || hidden(store)?;
        let mut book = self.surfaces.lock().map_err(err)?;
        let due = book.due(now_ms());
        for label in due {
            book.close(&label);
        }
        let mut close = book.reconcile(&valid, hidden);
        close.extend(std::mem::take(&mut book.closing));
        close.sort();
        close.dedup();
        Ok(close)
    }
    fn snapshot(&self, store: &Store, source: &Source) -> AppResult<Value> {
        let state = self.runtime.lock().map_err(err)?;
        let live = state
            .entries
            .get(&source.consumer.instance)
            .filter(|l| l.revision == source.consumer.config_revision);
        let mut value = source.state.clone();
        value["kind"] = json!(if source.kind == Kind::History {
            "history"
        } else {
            "search"
        });
        value["listening"] = json!(
            source.state["captureEnabled"] == true
                && live.is_some_and(|l| l.listening)
                && !self.stopped.load(Ordering::Acquire)
        );
        value["lastError"] = json!(live
            .and_then(|l| l.last_error.as_ref())
            .or(state.error.as_ref()));
        let registered = source.state["shortcutEnabled"] == true
            && live.is_some_and(|l| l.shortcut_registered)
            && !self.stopped.load(Ordering::Acquire);
        let shortcut_error = live.and_then(|l| l.shortcut_error.clone());
        value["shortcutRegistered"] = json!(registered);
        value["shortcutError"] = json!(shortcut_error);
        value["shortcutState"] = json!(if source.state["shortcutEnabled"] != true {
            "disabled"
        } else if registered {
            "registered"
        } else if shortcut_error.is_some() {
            "failed"
        } else {
            "pending"
        });
        drop(state);
        match self.actions.latest(store, &source.consumer.instance) {
            Ok((revision, last)) => {
                value["actionRevision"] = json!(revision);
                value["actionError"] = Value::Null;
                value["lastAction"] = last;
            }
            Err(error) => {
                value["actionRevision"] = Value::Null;
                value["actionError"] = json!(error);
                value["lastAction"] = Value::Null;
            }
        }
        Ok(value)
    }
    fn access(
        &self,
        store: &Store,
        sessions: &widgets::Sessions,
        window: &str,
        token: &str,
        method: &str,
        args: Value,
    ) -> AppResult<Value> {
        crate::keyboard_host::overlay_method(window, method)?;
        crate::reminder_host::surface_method(window, method)?;
        clipboard_surface::surface_method(window, method)?;
        let _native = store.native_actions.lock().map_err(err)?;
        if self.stopped.load(Ordering::Acquire) {
            return Err("SESSION_EXPIRED: 剪切板后台已退出".into());
        }
        let permission = match method {
            "clipboard.status" | "clipboard.surface" | "clipboard.hide" | "clipboard.hover" => {
                CONTROL
            }
            "clipboard.actionStatus" => COPY,
            "clipboard.peek" => SEARCH_ACCESS,
            "clipboard.query" | "clipboard.get" => HISTORY_ACCESS,
            "clipboard.mutate" if args["command"]["type"] == "configure" => CONTROL,
            "clipboard.mutate" => HISTORY_ACCESS,
            _ => return Err("CAPABILITY_DENIED: 未开放的剪切板操作".into()),
        };
        with_permission(store, sessions, window, token, permission, |session| {
            if clipboard_surface::role(window, &session.instance).is_some()
                && !self.current_window(window, &session.instance)
                && !matches!(method, "clipboard.status" | "clipboard.actionStatus")
            {
                return Err("PRESENTATION_EXPIRED: 剪切板窗口已关闭或替换".into());
            }
            let current = source(store, &session.instance)?;
            match method {
                "clipboard.surface" => {
                    empty(&args)?;
                    let (revision, action_error) =
                        match clipboard_actions::revision(store, &session.instance) {
                            Ok(revision) => (json!(revision), Value::Null),
                            Err(error) => (Value::Null, json!(error)),
                        };
                    let book = self.surfaces.lock().map_err(err)?;
                    let p = book
                        .entries
                        .get(&session.instance)
                        .filter(|p| p.label == window && p.phase != Phase::Hidden)
                        .ok_or("PRESENTATION_EXPIRED: 当前不是剪切板弹窗")?;
                    Ok(
                        json!({"presentationId":p.id,"configRevision":p.revision,"kind":if p.kind==Kind::History{"history"}else{"search"},"pasteTargetAvailable":p.target.is_some(),"actionRevision":revision,"actionError":action_error}),
                    )
                }
                "clipboard.hide" => {
                    empty(&args)?;
                    if clipboard_surface::role(window, &session.instance).is_none() {
                        return Err("CAPABILITY_DENIED: 当前不是剪切板弹窗".into());
                    }
                    self.closed(window);
                    Ok(Value::Null)
                }
                "clipboard.hover" => {
                    #[derive(Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct Hover {
                        entered: bool,
                    }
                    let value: Hover = serde_json::from_value(args)
                        .map_err(|_| "INVALID_ARGUMENT: 悬停只接受entered")?;
                    let mut book = self.surfaces.lock().map_err(err)?;
                    let focus = book
                        .entries
                        .get(&session.instance)
                        .filter(|p| p.label == window)
                        .map(|p| p.focused)
                        .ok_or("PRESENTATION_EXPIRED: 悬停窗口已失效")?;
                    book.interact(window, focus, value.entered, now_ms())?;
                    Ok(Value::Null)
                }
                "clipboard.status" => {
                    empty(&args)?;
                    self.snapshot(store, &current)
                }
                "clipboard.actionStatus" if current.kind == Kind::History => {
                    #[derive(Deserialize)]
                    #[serde(rename_all = "camelCase", deny_unknown_fields)]
                    struct Status {
                        request_id: String,
                    }
                    let request: Status = serde_json::from_value(args)
                        .map_err(|_| "INVALID_ARGUMENT: 动作状态仅接受requestId")?;
                    self.actions
                        .status(store, &session.instance, &request.request_id)
                }
                "clipboard.peek" if current.kind == Kind::Search => {
                    empty(&args)?;
                    let state = self.runtime.lock().map_err(err)?;
                    let live = state.entries.get(&session.instance).filter(|l| {
                        l.revision == current.consumer.config_revision
                            && current.state["captureEnabled"] == true
                    });
                    Ok(
                        json!({"text":live.map(|l|l.text.as_str()).unwrap_or(""),"serial":live.map(|l|l.serial).unwrap_or(0),"updatedAt":live.map(|l|l.updated_at),"configRevision":current.consumer.config_revision}),
                    )
                }
                "clipboard.query" if current.kind == Kind::History => {
                    clipboard_history::query(store, &session.instance, args)
                }
                "clipboard.get" if current.kind == Kind::History => {
                    #[derive(Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct Get {
                        id: String,
                    }
                    let value: Get = serde_json::from_value(args)
                        .map_err(|_| "INVALID_ARGUMENT: 读取记录仅接受id")?;
                    clipboard_history::get(store, &session.instance, &value.id)
                }
                "clipboard.mutate" => {
                    let operation = args["command"]["type"].as_str().unwrap_or("");
                    if !matches!(operation, "configure" | "edit" | "delete" | "clear")
                        || (current.kind == Kind::Search && operation != "configure")
                    {
                        return Err(
                            "CAPABILITY_DENIED: 页面不能写入采样或使用次数，AI剪切板不建立历史库"
                                .into(),
                        );
                    }
                    let result = clipboard_history::mutate(store, &session.instance, args)?;
                    let mut runtime = self.runtime.lock().map_err(err)?;
                    if result["configRevision"] != current.state["configRevision"] {
                        runtime.entries.remove(&session.instance);
                    } else if let Some(live) = runtime.entries.get_mut(&session.instance) {
                        live.last_error = None;
                    }
                    drop(runtime);
                    if result["configRevision"] != current.state["configRevision"] {
                        let label = self
                            .surfaces
                            .lock()
                            .map_err(err)?
                            .entries
                            .get(&session.instance)
                            .map(|p| p.label.clone());
                        if let Some(label) = label {
                            self.closed(&label);
                        }
                    }
                    let mut snapshot = self.snapshot(store, &source(store, &session.instance)?)?;
                    snapshot["replayed"] = result["replayed"].clone();
                    Ok(snapshot)
                }
                _ => Err("INVALID_ARGUMENT: 剪切板方法或数据源无效".into()),
            }
        })
    }
    fn step(&self, store: &Store, reader: &mut impl Reader) -> AppResult<Vec<Consumer>> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("SESSION_EXPIRED: 剪切板后台已退出".into());
        }
        let ids = candidates(store)?;
        let mut sources = Vec::new();
        {
            let keep: HashSet<_> = ids.iter().cloned().collect();
            self.runtime
                .lock()
                .map_err(err)?
                .entries
                .retain(|id, _| keep.contains(id));
        }
        for id in ids {
            match source(store, &id) {
                Ok(source) => {
                    let mut runtime = self.runtime.lock().map_err(err)?;
                    if self.stopped.load(Ordering::Acquire) {
                        return Ok(Vec::new());
                    }
                    let live = runtime.entries.entry(id).or_default();
                    if live.revision != source.consumer.config_revision || live.kind != source.kind
                    {
                        *live = Live {
                            revision: source.consumer.config_revision,
                            kind: source.kind,
                            ..Default::default()
                        };
                    }
                    if !source.can_observe || source.state["captureEnabled"] != true {
                        live.listening = false;
                        live.text.clear();
                        live.pending_open = false;
                    }
                    drop(runtime);
                    sources.push(source);
                }
                Err(error) => {
                    let mut runtime = self.runtime.lock().map_err(err)?;
                    if self.stopped.load(Ordering::Acquire) {
                        return Ok(Vec::new());
                    }
                    let live = runtime.entries.entry(id).or_default();
                    live.listening = false;
                    live.text.clear();
                    live.pending_open = false;
                    live.last_error = Some(error);
                }
            }
        }
        let consumers: Vec<_> = sources
            .iter()
            .filter(|s| s.can_observe && s.state["captureEnabled"] == true)
            .map(|s| s.consumer.clone())
            .collect();
        // 系统读取期间不持原生操作锁/SQLite锁，停用、删除与退出无需等待外部剪贴板所有者。
        if self.stopped.load(Ordering::Acquire) {
            return Ok(Vec::new());
        }
        let tick = self.gate.lock().map_err(err)?.tick(&consumers, reader);
        let mut notices = Vec::new();
        if self.stopped.load(Ordering::Acquire) {
            return Ok(notices);
        }
        {
            let mut runtime = self.runtime.lock().map_err(err)?;
            runtime.error = tick.as_ref().err().cloned();
            for owner in &consumers {
                if let Some(live) = runtime
                    .entries
                    .get_mut(&owner.instance)
                    .filter(|l| l.revision == owner.config_revision)
                {
                    live.listening = tick.is_ok();
                }
            }
        }
        let tick = tick?;
        for failure in tick.failures {
            let mut runtime = self.runtime.lock().map_err(err)?;
            if let Some(live) = runtime
                .entries
                .get_mut(&failure.consumer.instance)
                .filter(|l| l.revision == failure.consumer.config_revision)
            {
                live.last_error = Some(failure.error);
            }
        }
        for delivery in tick.deliveries {
            let _native = store.native_actions.lock().map_err(err)?;
            if self.stopped.load(Ordering::Acquire) {
                break;
            }
            let Ok(current) = source(store, &delivery.consumer.instance) else {
                self.runtime
                    .lock()
                    .map_err(err)?
                    .entries
                    .remove(&delivery.consumer.instance);
                continue;
            };
            if current.consumer.config_revision != delivery.consumer.config_revision
                || !current.can_observe
                || current.state["captureEnabled"] != true
                || !sources.iter().any(|s| {
                    s.consumer.instance == current.consumer.instance && s.kind == current.kind
                })
            {
                continue;
            }
            let saved = if current.kind == Kind::History {
                clipboard_history::capture(
                    store,
                    &current.consumer.instance,
                    current.consumer.config_revision,
                    &delivery.text,
                    now_ms(),
                )
                .map(|_| ())
            } else {
                Ok(())
            };
            let mut runtime = self.runtime.lock().map_err(err)?;
            let Some(live) = runtime
                .entries
                .get_mut(&current.consumer.instance)
                .filter(|l| l.revision == current.consumer.config_revision)
            else {
                continue;
            };
            if let Err(error) = saved {
                live.last_error = Some(error);
                continue;
            }
            live.last_error = None;
            if current.kind == Kind::Search {
                live.text = delivery.text;
                live.serial = live.serial.saturating_add(1);
                live.updated_at = now_ms();
                live.pending_open = true;
                notices.push(current.consumer);
            }
        }
        Ok(notices)
    }
    pub(crate) fn forget(&self, id: &str) -> AppResult<()> {
        self.runtime.lock().map_err(err)?.entries.remove(id);
        let mut book = self.surfaces.lock().map_err(err)?;
        if let Some(p) = book.entries.remove(id) {
            book.closing.push(p.label);
        }
        Ok(())
    }
    pub(crate) fn start(&self, app: &AppHandle) -> AppResult<()> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("SESSION_EXPIRED: 剪切板后台已退出".into());
        }
        let mut slot = self.thread.lock().map_err(err)?;
        if slot.is_some() {
            return Ok(());
        }
        let app = app.clone();
        let ui_app = app.clone();
        *slot = Some(
            std::thread::Builder::new()
                .name("clipboard-coordinator".into())
                .spawn(move || {
                    let mut reader = SystemClipboard::default();
                    loop {
                        let core = app.state::<Core>();
                        if core.quitting.load(Ordering::Acquire)
                            || core.clipboard.stopped.load(Ordering::Acquire)
                        {
                            break;
                        }
                        match core.clipboard.step(&core.store, &mut reader) {
                            Ok(notices) => {
                                for consumer in notices {
                                    let handle = app.clone();
                                    crate::windows::dispatch(&app, move |app| {
                                        let core = handle.state::<Core>();
                                        if let Ok(p) = core.clipboard.prepare_popup(
                                            &core.store,
                                            &consumer.instance,
                                            None,
                                            Some(consumer.config_revision),
                                        ) {
                                            crate::windows::clipboard_show(app, &p)?;
                                        }
                                        Ok(())
                                    });
                                }
                            }
                            Err(error) => {
                                if let Ok(mut runtime) = core.clipboard.runtime.lock() {
                                    runtime.error = Some(error);
                                }
                            }
                        }
                        std::thread::sleep(Duration::from_millis(200));
                    }
                })
                .map_err(err)?,
        );
        *self.ui_thread.lock().map_err(err)? = Some(
            std::thread::Builder::new()
                .name("clipboard-surfaces".into())
                .spawn(move || {
                    let mut last_shortcuts = std::time::Instant::now() - Duration::from_secs(2);
                    loop {
                        let core = ui_app.state::<Core>();
                        if core.quitting.load(Ordering::Acquire)
                            || core.clipboard.stopped.load(Ordering::Acquire)
                        {
                            break;
                        }
                        if last_shortcuts.elapsed() >= Duration::from_secs(1) {
                            if let Err(error) = core
                                .clipboard
                                .shortcuts_with(&core.store, &mut AppDriver(&ui_app))
                            {
                                if let Ok(mut runtime) = core.clipboard.runtime.lock() {
                                    runtime.error = Some(error);
                                }
                            }
                            last_shortcuts = std::time::Instant::now();
                        }
                        if let Ok(labels) = core.clipboard.reconcile_windows(&core.store) {
                            for label in labels {
                                crate::windows::dispatch(&ui_app, move |app| {
                                    crate::windows::clipboard_close(app, &label)
                                });
                            }
                        }
                        if let Ok(labels) = core.ai.reconcile_tools(&core.store) {
                            for label in labels {
                                crate::windows::dispatch(&ui_app, move |app| {
                                    crate::windows::clipboard_close(app, &label)
                                });
                            }
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                })
                .map_err(err)?,
        );
        Ok(())
    }
    pub(crate) fn stop(&self, store: &Store) -> AppResult<()> {
        self.stopped.store(true, Ordering::Release);
        // 等已经获得许可的短事务完成；读取系统剪贴板本身不会占用这把锁。
        let _native = store.native_actions.lock().map_err(err)?;
        // 外部进程的延迟呈现可能阻塞Win32读取；退出不等待该调用，返回后也不能再提交。
        self.thread.lock().map_err(err)?.take();
        self.ui_thread.lock().map_err(err)?.take();
        self.runtime.lock().map_err(err)?.entries.clear();
        let mut book = self.surfaces.lock().map_err(err)?;
        let labels = book
            .entries
            .values()
            .map(|p| p.label.clone())
            .collect::<Vec<_>>();
        for label in labels {
            book.close(&label);
        }
        Ok(())
    }
}
pub(crate) async fn gateway(
    app: &AppHandle,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    let app = app.clone();
    let (window, token, method) = (window.to_owned(), token.to_owned(), method.to_owned());
    tauri::async_runtime::spawn_blocking(move || {
        let core = app.state::<Core>();
        if matches!(method.as_str(), "clipboard.tool" | "clipboard.settings") {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct ToolInput {
                kind: String,
                content: String,
            }
            let (kind, content) = if method == "clipboard.settings" {
                empty(&args)?;
                (crate::clipboard_tools::Kind::Settings, String::new())
            } else {
                let input: ToolInput = serde_json::from_value(args)
                    .map_err(|_| "INVALID_ARGUMENT: 工具只接受kind/content")?;
                let kind = crate::clipboard_tools::kind(&input.kind)?;
                if kind.task().is_none() {
                    return Err("INVALID_ARGUMENT: 仅支持导图和翻译".into());
                }
                (kind, input.content)
            };
            let p = {
                let _native = core.store.native_actions.lock().map_err(err)?;
                with_permission(
                    &core.store,
                    &core.sessions,
                    &window,
                    &token,
                    TOOLS,
                    |session| {
                        let current = source(&core.store, &session.instance)?;
                        if current.kind != Kind::Search {
                            return Err("CAPABILITY_DENIED: 仅AI剪切板可打开工具".into());
                        }
                        if clipboard_surface::role(&window, &session.instance).is_some()
                            && !core.clipboard.current_window(&window, &session.instance)
                        {
                            return Err("PRESENTATION_EXPIRED: 搜索条已关闭".into());
                        }
                        if window.starts_with("clipboard-tool-") {
                            return Err("CAPABILITY_DENIED: 工具不再打开其他工具".into());
                        }
                        core.ai
                            .prepare_tool(&core.store, &session.instance, kind, &content)
                    },
                )?
            };
            crate::windows::serialized(&app, |app| crate::windows::clipboard_tool_show(app, &p))?;
            return Ok(json!({"presentationId":p.id}));
        }
        if method == "clipboard.hide" && window.starts_with("clipboard-tool-") {
            empty(&args)?;
            {
                let _native = core.store.native_actions.lock().map_err(err)?;
                with_permission(
                    &core.store,
                    &core.sessions,
                    &window,
                    &token,
                    TOOLS,
                    |session| {
                        core.ai.tool(&core.store, &window, &session.instance)?;
                        Ok(())
                    },
                )?;
            }
            crate::windows::serialized(&app, |app| crate::windows::clipboard_close(app, &window))?;
            return Ok(Value::Null);
        }
        if method == "clipboard.open" {
            empty(&args)?;
            #[cfg(windows)]
            let target = clipboard_native::system::capture_target().ok();
            #[cfg(not(windows))]
            let target = None;
            let p = core.clipboard.request_popup(
                &core.store,
                &core.sessions,
                &window,
                &token,
                target,
            )?;
            crate::windows::serialized(&app, |app| crate::windows::clipboard_show(app, &p))?;
            return Ok(json!({"presentationId":p.id}));
        }
        if method == "clipboard.act" {
            let request: clipboard_actions::Request =
                serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENT: 剪切板动作参数无效")?;
            let p = core.clipboard.bound_popup(
                &core.store,
                &core.sessions,
                &window,
                &token,
                action_permission(request.paste),
            )?;
            if request.paste && p.target.is_none() {
                return Err("TARGET_UNAVAILABLE: 未捕获原应用目标，可使用仅复制".into());
            }
            #[cfg(windows)]
            {
                let handle = app.clone();
                let (label, instance, presentation, request_id) = (
                    window.clone(),
                    p.instance.clone(),
                    p.id.clone(),
                    request.request_id.clone(),
                );
                let platform = clipboard_native::system::Native::new(move || {
                    crate::windows::serialized(&handle, |app| {
                        let core = app.state::<Core>();
                        let _native = core.store.native_actions.lock().map_err(err)?;
                        let current = source(&core.store, &instance)?;
                        if current.consumer.config_revision != p.revision
                            || core.clipboard.stopped.load(Ordering::Acquire)
                        {
                            return Err("PRESENTATION_EXPIRED: 隐藏前动作已失效".into());
                        }
                        core.clipboard.surfaces.lock().map_err(err)?.action_hidden(
                            &instance,
                            &presentation,
                            &request_id,
                        )?;
                        app.get_webview_window(&label)
                            .ok_or("PRESENTATION_EXPIRED: 原弹窗已关闭")?
                            .hide()
                            .map_err(err)
                    })
                });
                let mut desktop = clipboard_native::Controlled::new(
                    p.owner,
                    if request.paste { p.target } else { None },
                    platform,
                );
                return core.clipboard.act_with(
                    &core.store,
                    &core.sessions,
                    &window,
                    &token,
                    request,
                    &mut desktop,
                );
            }
            #[cfg(not(windows))]
            return Err("UNSUPPORTED: 剪切板主动输入目前仅支持Windows".into());
        }
        if method == "clipboard.search" {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Search {
                content: String,
            }
            let value: Search =
                serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENT: 搜索只接受content")?;
            let _native = core.store.native_actions.lock().map_err(err)?;
            return with_permission(
                &core.store,
                &core.sessions,
                &window,
                &token,
                SEARCH_ACCESS,
                |session| {
                    let current = source(&core.store, &session.instance)?;
                    if current.kind != Kind::Search
                        || !core.clipboard.current_window(&window, &session.instance)
                        || core.clipboard.stopped.load(Ordering::Acquire)
                    {
                        return Err("PRESENTATION_EXPIRED: 搜索条已失效".into());
                    }
                    let url = search_url(
                        current.state["searchPlatform"].as_str().unwrap_or("google"),
                        &value.content,
                    )?;
                    crate::content::open_external(&url)?;
                    Ok(json!({"submitted":true}))
                },
            );
        }
        let value =
            core.clipboard
                .access(&core.store, &core.sessions, &window, &token, &method, args)?;
        if method == "clipboard.hide" {
            crate::windows::serialized(&app, |app| crate::windows::clipboard_close(app, &window))?;
        }
        Ok(value)
    })
    .await
    .map_err(err)?
}
pub(crate) fn shortcut_target(store: &Store, spec: &Spec) -> Option<Target> {
    if source(store, &spec.instance).is_ok_and(|s| s.kind == Kind::History) {
        #[cfg(windows)]
        {
            return clipboard_native::system::capture_target().ok();
        }
    }
    None
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::{clipboard_capture::Read, clipboard_history};
    use serde_json::json;
    use std::collections::VecDeque;
    fn uid() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn fixture() -> (Store, widgets::Sessions, Service) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("artifacts")
            .join(format!("widgetjs-clipboard-host-test-{}", uid()));
        (
            Store::open(root).unwrap(),
            widgets::Sessions::default(),
            Service::default(),
        )
    }
    fn add(
        s: &Store,
        sessions: &widgets::Sessions,
        package: &str,
        capability: &str,
    ) -> (String, String, String) {
        add_with(s, sessions, package, &[capability])
    }
    fn add_with(
        s: &Store,
        sessions: &widgets::Sessions,
        package: &str,
        capabilities: &[&str],
    ) -> (String, String, String) {
        let manifest = json!({"schemaVersion":1,"id":package,"name":"剪切板宿主检查","version":"0.1.0","hostApiMajor":1,"entry":"dist/index.html","defaultSize":{"width":500,"height":400},"minSize":{"width":200,"height":160},"capabilities":capabilities,"settingsSchema":{"type":"object","additionalProperties":false,"properties":{}}});
        s.db.lock().unwrap().execute("INSERT OR IGNORE INTO widget_packages(id,manifest,directory,digest) VALUES(?1,?2,'clipboard-test','clipboard-test')",rusqlite::params![package,manifest.to_string()]).unwrap();
        let id = widgets::create_instance(s, package).unwrap();
        let window = format!("float-{id}");
        let session = widgets::open_display_session(s, sessions, &window, &id, None).unwrap();
        (id, window, session["token"].as_str().unwrap().into())
    }
    #[test]
    fn scoped_clipboard_control_does_not_grant_history_paste_or_system_capture() {
        let (store, sessions, service) = fixture();
        let row = add_with(&store, &sessions, HISTORY, &[CONTROL, COPY]);
        configure(&service, &store, &sessions, &row, 0, true);
        assert!(service
            .access(
                &store,
                &sessions,
                &row.1,
                &row.2,
                "clipboard.status",
                Value::Null
            )
            .is_ok());
        assert!(service
            .access(
                &store,
                &sessions,
                &row.1,
                &row.2,
                "clipboard.query",
                json!({})
            )
            .is_err());
        assert!(with_permission(
            &store,
            &sessions,
            &row.1,
            &row.2,
            action_permission(false),
            |_| Ok(())
        )
        .is_ok());
        assert!(with_permission(
            &store,
            &sessions,
            &row.1,
            &row.2,
            action_permission(true),
            |_| Ok(())
        )
        .is_err());
        let mut reader = Fake::new(1);
        reader.text("隔离夹具");
        assert!(service.step(&store, &mut reader).unwrap().is_empty());
        assert_eq!(reader.sequences, 0);
        for (package, names) in [
            (HISTORY, vec![CONTROL, PASTE]),
            (SEARCH, vec![CONTROL, HISTORY_ACCESS]),
            (SEARCH, vec![CONTROL, CAPABILITY]),
            ("local.test.foreign", vec![CONTROL]),
        ] {
            assert!(validate_permissions(
                package,
                &names.into_iter().map(str::to_owned).collect::<Vec<_>>()
            )
            .is_err());
        }
    }
    fn config(revision: u64, enabled: bool) -> Value {
        json!({"requestId":uid(),"command":{"type":"configure","expectedRevision":revision,"captureEnabled":enabled,"shortcutEnabled":false,"pasteEnabled":false,"shortcut":"Super+Alt+V","historyExpiredHours":12}})
    }
    fn configure(
        service: &Service,
        s: &Store,
        sessions: &widgets::Sessions,
        row: &(String, String, String),
        rev: u64,
        on: bool,
    ) {
        service
            .access(
                s,
                sessions,
                &row.1,
                &row.2,
                "clipboard.mutate",
                config(rev, on),
            )
            .unwrap();
    }
    struct Fake<'a> {
        seq: u32,
        reads: usize,
        sequences: usize,
        values: VecDeque<Read>,
        before: Option<Box<dyn FnOnce() + 'a>>,
    }
    impl Fake<'_> {
        fn new(seq: u32) -> Self {
            Self {
                seq,
                reads: 0,
                sequences: 0,
                values: VecDeque::new(),
                before: None,
            }
        }
        fn text(&mut self, text: &str) {
            self.values.push_back(Read::Text(text.into()));
        }
    }
    impl Reader for Fake<'_> {
        fn sequence(&mut self) -> AppResult<u32> {
            self.sequences += 1;
            Ok(self.seq)
        }
        fn read(&mut self, expected: u32) -> AppResult<Read> {
            assert_eq!(expected, self.seq);
            self.reads += 1;
            if let Some(action) = self.before.take() {
                action();
            }
            Ok(self.values.pop_front().expect("意外真实读取"))
        }
    }
    #[test]
    fn clipboard_host_shortcut_event_does_not_wait_for_native_registration_thread() {
        let (s, _, service) = fixture();
        let _registering = service.shortcuts.lock().unwrap();
        assert_eq!(
            service.shortcut_route(
                &s,
                crate::panel_shortcut(&s)
                    .parse::<tauri_plugin_global_shortcut::Shortcut>()
                    .unwrap()
                    .id()
            ),
            Route::Ignore
        );
    }
    #[test]
    fn clipboard_host_requires_fixed_package_capability_and_exact_current_window() {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, HISTORY, CAPABILITY);
        let state = service
            .access(
                &s,
                &sessions,
                &row.1,
                &row.2,
                "clipboard.status",
                Value::Null,
            )
            .unwrap();
        assert_eq!(state["captureEnabled"], false);
        assert_eq!(state["listening"], false);
        assert!(service
            .access(
                &s,
                &sessions,
                "other-window",
                &row.2,
                "clipboard.status",
                Value::Null
            )
            .is_err());
        let other = add(&s, &sessions, "local.test.clipboard-foreign", CAPABILITY);
        assert!(service
            .access(
                &s,
                &sessions,
                &other.1,
                &other.2,
                "clipboard.status",
                Value::Null
            )
            .unwrap_err()
            .contains("固定"));
        let no_cap = add(&s, &sessions, SEARCH, "storage.instance");
        assert!(service
            .access(
                &s,
                &sessions,
                &no_cap.1,
                &no_cap.2,
                "clipboard.status",
                Value::Null
            )
            .unwrap_err()
            .contains("未声明"));
        sessions.0.lock().unwrap().remove(&row.2);
        assert!(service
            .access(
                &s,
                &sessions,
                &row.1,
                &row.2,
                "clipboard.mutate",
                config(0, true)
            )
            .is_err());
    }
    #[test]
    fn clipboard_host_gateway_cannot_inject_samples_usage_counts_foreign_instances_or_arbitrary_fields(
    ) {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, HISTORY, CAPABILITY);
        let search = add(&s, &sessions, SEARCH, CAPABILITY);
        configure(&service, &s, &sessions, &row, 0, true);
        for (method, args) in [
            ("clipboard.capture", json!({"content":"伪造"})),
            (
                "clipboard.mutate",
                json!({"requestId":uid(),"command":{"type":"used","id":uid(),"expectedItemRevision":1}}),
            ),
            ("clipboard.query", json!({"instance":search.0})),
            ("clipboard.status", json!({"extra":true})),
        ] {
            assert!(service
                .access(&s, &sessions, &row.1, &row.2, method, args)
                .is_err());
        }
        assert!(service
            .access(
                &s,
                &sessions,
                &search.1,
                &search.2,
                "clipboard.query",
                json!({})
            )
            .is_err());
        assert!(service
            .access(&s, &sessions, &row.1, &row.2, "clipboard.peek", json!({}))
            .is_err());
        assert_eq!(clipboard_history::status(&s, &row.0).unwrap()["count"], 0);
    }
    #[test]
    fn clipboard_host_disabled_has_zero_io_and_background_survives_closing_display_sessions() {
        let (s, sessions, service) = fixture();
        let a = add(&s, &sessions, HISTORY, CAPABILITY);
        let b = add(&s, &sessions, SEARCH, CAPABILITY);
        let mut io = Fake::new(10);
        service.step(&s, &mut io).unwrap();
        assert_eq!(io.sequences, 0);
        assert_eq!(io.reads, 0);
        configure(&service, &s, &sessions, &a, 0, true);
        configure(&service, &s, &sessions, &b, 0, true);
        service.step(&s, &mut io).unwrap();
        assert_eq!(io.reads, 0);
        io.seq = 11;
        io.text("当前文本🙂");
        let notices = service.step(&s, &mut io).unwrap();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].instance, b.0);
        assert_eq!(io.reads, 1);
        assert_eq!(
            clipboard_history::query(&s, &a.0, json!({})).unwrap()["items"][0]["content"],
            "当前文本🙂"
        );
        assert_eq!(clipboard_history::status(&s, &b.0).unwrap()["count"], 0);
        assert_eq!(
            service
                .access(&s, &sessions, &b.1, &b.2, "clipboard.peek", Value::Null)
                .unwrap()["text"],
            "当前文本🙂"
        );
        sessions.0.lock().unwrap().clear();
        io.seq = 12;
        io.text("视图已关闭");
        service.step(&s, &mut io).unwrap();
        assert_eq!(clipboard_history::status(&s, &a.0).unwrap()["count"], 2);
    }
    #[test]
    fn clipboard_host_disabling_one_consumer_clears_its_ephemeral_text_without_stopping_other_history(
    ) {
        let (s, sessions, service) = fixture();
        let a = add(&s, &sessions, HISTORY, CAPABILITY);
        let b = add(&s, &sessions, SEARCH, CAPABILITY);
        let mut io = Fake::new(1);
        configure(&service, &s, &sessions, &a, 0, true);
        configure(&service, &s, &sessions, &b, 0, true);
        service.step(&s, &mut io).unwrap();
        io.seq = 2;
        io.text("共享读取");
        service.step(&s, &mut io).unwrap();
        configure(&service, &s, &sessions, &b, 1, false);
        assert_eq!(
            service
                .access(&s, &sessions, &b.1, &b.2, "clipboard.peek", Value::Null)
                .unwrap()["text"],
            ""
        );
        io.seq = 3;
        io.text("独立继续");
        assert!(service.step(&s, &mut io).unwrap().is_empty());
        assert_eq!(clipboard_history::status(&s, &a.0).unwrap()["count"], 2);
        assert_eq!(clipboard_history::status(&s, &b.0).unwrap()["count"], 0);
    }
    #[test]
    fn clipboard_host_revalidates_configuration_after_io_without_holding_native_action_lock() {
        let (s, sessions, service) = fixture();
        let a = add(&s, &sessions, HISTORY, CAPABILITY);
        let b = add(&s, &sessions, HISTORY, CAPABILITY);
        let mut io = Fake::new(1);
        configure(&service, &s, &sessions, &a, 0, true);
        configure(&service, &s, &sessions, &b, 0, true);
        service.step(&s, &mut io).unwrap();
        io.seq = 2;
        io.text("读期间停用");
        io.before = Some(Box::new(|| {
            configure(&service, &s, &sessions, &a, 1, false);
        }));
        service.step(&s, &mut io).unwrap();
        assert_eq!(clipboard_history::status(&s, &a.0).unwrap()["count"], 0);
        assert_eq!(clipboard_history::status(&s, &b.0).unwrap()["count"], 1);
    }
    #[test]
    fn clipboard_host_delete_during_read_cleans_private_data_and_late_sample_cannot_recreate_it() {
        let (s, sessions, service) = fixture();
        let a = add(&s, &sessions, HISTORY, CAPABILITY);
        let mut io = Fake::new(1);
        configure(&service, &s, &sessions, &a, 0, true);
        service.step(&s, &mut io).unwrap();
        io.seq = 2;
        io.text("删除前的一次采样");
        io.before = Some(Box::new(|| {
            widgets::remove_instance(&s, &sessions, &a.0).unwrap();
        }));
        service.step(&s, &mut io).unwrap();
        let count: i64 =
            s.db.lock()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM app_settings WHERE key=?1",
                    [format!("clipboard.history/{}", a.0)],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(count, 0);
        assert!(service
            .access(&s, &sessions, &a.1, &a.2, "clipboard.status", Value::Null)
            .is_err());
    }
    #[test]
    fn clipboard_host_stop_during_read_discards_pending_text_and_rejects_every_later_request() {
        let (s, sessions, service) = fixture();
        let a = add(&s, &sessions, HISTORY, CAPABILITY);
        let mut io = Fake::new(1);
        configure(&service, &s, &sessions, &a, 0, true);
        service.step(&s, &mut io).unwrap();
        io.seq = 2;
        io.text("退出前迟到");
        io.before = Some(Box::new(|| {
            service.stop(&s).unwrap();
        }));
        service.step(&s, &mut io).unwrap();
        assert_eq!(clipboard_history::status(&s, &a.0).unwrap()["count"], 0);
        let reads = io.sequences;
        assert!(service.step(&s, &mut io).is_err());
        assert_eq!(io.sequences, reads);
        assert!(service
            .access(
                &s,
                &sessions,
                &a.1,
                &a.2,
                "clipboard.mutate",
                config(1, true)
            )
            .is_err());
    }
    #[test]
    fn clipboard_host_one_broken_document_or_failed_save_cannot_stop_healthy_consumer() {
        let (s, sessions, service) = fixture();
        let a = add(&s, &sessions, HISTORY, CAPABILITY);
        let b = add(&s, &sessions, HISTORY, CAPABILITY);
        let mut io = Fake::new(1);
        configure(&service, &s, &sessions, &a, 0, true);
        configure(&service, &s, &sessions, &b, 0, true);
        service.step(&s, &mut io).unwrap();
        s.db.lock().unwrap().execute_batch(&format!("CREATE TRIGGER refuse_one_clipboard BEFORE UPDATE ON app_settings WHEN new.key='clipboard.history/{}' BEGIN SELECT RAISE(FAIL,'fixture save denied'); END;",a.0)).unwrap();
        io.seq = 2;
        io.text("仅一个保存失败");
        service.step(&s, &mut io).unwrap();
        assert_eq!(clipboard_history::status(&s, &a.0).unwrap()["count"], 0);
        assert_eq!(clipboard_history::status(&s, &b.0).unwrap()["count"], 1);
        let state = service
            .access(&s, &sessions, &a.1, &a.2, "clipboard.status", Value::Null)
            .unwrap();
        assert!(state["lastError"].as_str().unwrap().contains("保存失败"));
        s.db.lock()
            .unwrap()
            .execute_batch("DROP TRIGGER refuse_one_clipboard")
            .unwrap();
        s.save_preference(&format!("clipboard.history/{}", a.0), "broken-cipher")
            .unwrap();
        io.seq = 3;
        io.text("损坏源不影响正常源");
        service.step(&s, &mut io).unwrap();
        assert_eq!(clipboard_history::status(&s, &b.0).unwrap()["count"], 2);
    }
    #[test]
    fn clipboard_host_preview_cannot_read_history_enable_capture_or_peek_ephemeral_text() {
        use std::{
            io::{Cursor, Write},
            sync::{Arc, Mutex},
        };
        let (s, sessions, service) = fixture();
        let _a = add(&s, &sessions, HISTORY, CAPABILITY);
        let manifest = json!({"schemaVersion":1,"id":HISTORY,"name":"预览检查","version":"0.1.1","hostApiMajor":1,"entry":"dist/index.html","defaultSize":{"width":500,"height":400},"minSize":{"width":200,"height":160},"capabilities":["storage.instance"],"settingsSchema":{"type":"object","additionalProperties":false,"properties":{}}});
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.start_file("dist/index.html", options).unwrap();
        zip.write_all(b"<!doctype html><title>preview</title>")
            .unwrap();
        let package = widgets::stage_bytes(&s, zip.finish().unwrap().into_inner()).unwrap();
        let window = format!("trial-{}", uid());
        let preview = Arc::new(Mutex::new(widgets::Preview {
            package,
            settings: json!({}),
            storage: json!({}),
            ready_since: None,
            failure: None,
        }));
        let session = widgets::open_preview(&sessions, &window, &window, preview).unwrap();
        for (method, args) in [
            ("clipboard.status", Value::Null),
            ("clipboard.query", json!({})),
            ("clipboard.mutate", config(0, true)),
            ("clipboard.peek", Value::Null),
        ] {
            assert!(service
                .access(
                    &s,
                    &sessions,
                    &window,
                    session["token"].as_str().unwrap(),
                    method,
                    args
                )
                .unwrap_err()
                .contains("试加载"));
        }
    }

    #[test]
    fn clipboard_host_stop_waits_for_current_native_commit_but_does_not_wait_for_system_reader() {
        let (s, _sessions, service) = fixture();
        let (tx, rx) = std::sync::mpsc::channel();
        let guard = s.native_actions.lock().unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                service.stop(&s).unwrap();
                tx.send(()).unwrap();
            });
            assert!(
                rx.recv_timeout(Duration::from_millis(50)).is_err(),
                "退出必须等当前已获准的数据库提交结束"
            );
            drop(guard);
            rx.recv_timeout(Duration::from_secs(2)).unwrap();
        });
        let mut io = Fake::new(1);
        assert!(service.step(&s, &mut io).is_err());
        assert_eq!(io.sequences, 0);
    }
    #[test]
    fn clipboard_host_action_status_is_current_instance_only_and_corrupt_ledger_does_not_stop_capture(
    ) {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, HISTORY, CAPABILITY);
        let other = add(&s, &sessions, HISTORY, CAPABILITY);
        assert_eq!(
            service
                .access(
                    &s,
                    &sessions,
                    &row.1,
                    &row.2,
                    "clipboard.status",
                    Value::Null
                )
                .unwrap()["actionRevision"],
            0
        );
        let request = uid();
        let receipt = json!({"id":request,"fingerprint":"0".repeat(64),"revision":1,"state":"pending","copy":"copied","paste":"unknown","code":null,"at":1});
        s.save_preference(
            &format!("clipboard.actions/{}", row.0),
            &json!({"version":1,"revision":1,"entries":[receipt]}).to_string(),
        )
        .unwrap();
        let state = service
            .access(
                &s,
                &sessions,
                &row.1,
                &row.2,
                "clipboard.actionStatus",
                json!({"requestId":request}),
            )
            .unwrap();
        assert_eq!(state["state"], "unknown");
        let latest = service
            .access(
                &s,
                &sessions,
                &row.1,
                &row.2,
                "clipboard.status",
                Value::Null,
            )
            .unwrap();
        assert_eq!(latest["lastAction"]["requestId"], request);
        assert_eq!(latest["lastAction"]["state"], "unknown");
        assert!(service
            .access(
                &s,
                &sessions,
                &other.1,
                &other.2,
                "clipboard.status",
                Value::Null
            )
            .unwrap()["lastAction"]
            .is_null());
        assert!(service
            .access(
                &s,
                &sessions,
                &other.1,
                &other.2,
                "clipboard.actionStatus",
                json!({"requestId":request})
            )
            .unwrap_err()
            .contains("NOT_FOUND"));
        configure(&service, &s, &sessions, &row, 0, true);
        let mut io = Fake::new(1);
        service.step(&s, &mut io).unwrap();
        s.save_preference(
            &format!("clipboard.actions/{}", row.0),
            "broken-action-ledger",
        )
        .unwrap();
        let state = service
            .access(
                &s,
                &sessions,
                &row.1,
                &row.2,
                "clipboard.status",
                Value::Null,
            )
            .unwrap();
        assert!(state["actionRevision"].is_null());
        assert!(state["actionError"]
            .as_str()
            .unwrap()
            .contains("DATA_UNAVAILABLE"));
        let (p, token) = popup(&service, &s, &sessions, &row.0);
        let surface = service
            .access(
                &s,
                &sessions,
                &p.label,
                &token,
                "clipboard.surface",
                Value::Null,
            )
            .unwrap();
        assert_eq!(surface["presentationId"], p.id);
        assert!(surface["actionRevision"].is_null());
        assert!(surface["actionError"]
            .as_str()
            .unwrap()
            .contains("DATA_UNAVAILABLE"));
        io.seq = 2;
        io.text("动作回执损坏仍可采集");
        service.step(&s, &mut io).unwrap();
        assert_eq!(clipboard_history::status(&s, &row.0).unwrap()["count"], 1);
    }
    #[test]
    fn clipboard_host_delete_transaction_removes_action_receipts_without_decoding_or_touching_other_instance(
    ) {
        let (s, sessions, _service) = fixture();
        let a = add(&s, &sessions, HISTORY, CAPABILITY);
        let b = add(&s, &sessions, HISTORY, CAPABILITY);
        for id in [&a.0, &b.0] {
            s.save_preference(&format!("clipboard.actions/{id}"), "fixture-invalid-ledger")
                .unwrap();
        }
        widgets::remove_instance(&s, &sessions, &a.0).unwrap();
        let db = s.db.lock().unwrap();
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM app_settings WHERE key=?1",
                [format!("clipboard.actions/{}", a.0)],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        let value: String = db
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                [format!("clipboard.actions/{}", b.0)],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(value, "fixture-invalid-ledger");
    }
    fn popup(
        service: &Service,
        s: &Store,
        sessions: &widgets::Sessions,
        id: &str,
    ) -> (Popup, String) {
        let p = service.prepare_popup(s, id, None, None).unwrap();
        service
            .surfaces
            .lock()
            .unwrap()
            .attach(id, &p.id, 42)
            .unwrap();
        service
            .surfaces
            .lock()
            .unwrap()
            .shown(id, &p.id, now_ms())
            .unwrap();
        let session = widgets::open_display_session(s, sessions, &p.label, id, None).unwrap();
        (p, session["token"].as_str().unwrap().into())
    }
    #[derive(Default)]
    struct Actions<'a> {
        writes: usize,
        sends: usize,
        on_prepare: Option<Box<dyn FnOnce() + 'a>>,
    }
    impl clipboard_actions::Desktop for Actions<'_> {
        fn ready(&mut self) -> AppResult<()> {
            Ok(())
        }
        fn write(&mut self, _text: &str) -> AppResult<clipboard_actions::Write> {
            self.writes += 1;
            Ok(clipboard_actions::Write::Copied(88))
        }
        fn prepare_paste(&mut self) -> AppResult<()> {
            if let Some(f) = self.on_prepare.take() {
                f();
            }
            Ok(())
        }
        fn target_current(&mut self) -> bool {
            true
        }
        fn send_paste(&mut self) -> AppResult<clipboard_actions::Paste> {
            self.sends += 1;
            Ok(clipboard_actions::Paste::Submitted)
        }
    }
    fn action_request(p: &Popup, item: &Value, paste: bool) -> clipboard_actions::Request {
        clipboard_actions::Request {
            request_id: uid(),
            action_revision: 0,
            config_revision: 1,
            presentation_id: p.id.clone(),
            item_id: item["id"].as_str().unwrap().into(),
            item_revision: item["revision"].as_u64().unwrap(),
            paste,
        }
    }
    #[test]
    fn clipboard_host_popup_session_coexists_with_primary_and_rejects_wrong_package_or_role() {
        let (s, sessions, service) = fixture();
        let history = add(&s, &sessions, HISTORY, CAPABILITY);
        let (p, token) = popup(&service, &s, &sessions, &history.0);
        assert!(sessions.0.lock().unwrap().contains_key(&history.2));
        assert_eq!(
            service
                .access(
                    &s,
                    &sessions,
                    &p.label,
                    &token,
                    "clipboard.surface",
                    Value::Null
                )
                .unwrap()["presentationId"],
            p.id
        );
        let forged = format!("clipboard-search-{}-{}", history.0, uid());
        assert!(widgets::open_display_session(&s, &sessions, &forged, &history.0, None).is_err());
        let missing = add(&s, &sessions, "local.test.foreign-popup", CAPABILITY);
        let label = format!("clipboard-history-{}-{}", missing.0, uid());
        assert!(widgets::open_display_session(&s, &sessions, &label, &missing.0, None).is_err());
    }
    #[test]
    fn clipboard_host_popup_open_and_automatic_search_require_live_generation_and_revoke_on_config_change(
    ) {
        let (s, sessions, service) = fixture();
        let history = add(&s, &sessions, HISTORY, CAPABILITY);
        let search = add(&s, &sessions, SEARCH, CAPABILITY);
        assert!(service.prepare_popup(&s, &search.0, None, Some(0)).is_err());
        configure(&service, &s, &sessions, &search, 0, true);
        assert!(service.prepare_popup(&s, &search.0, None, Some(0)).is_err());
        let auto = service.prepare_popup(&s, &search.0, None, Some(1)).unwrap();
        assert!(service
            .surfaces
            .lock()
            .unwrap()
            .current(&auto.label, &search.0));
        let (p, token) = popup(&service, &s, &sessions, &history.0);
        configure(&service, &s, &sessions, &history, 0, true);
        assert!(!service
            .surfaces
            .lock()
            .unwrap()
            .current(&p.label, &history.0));
        assert!(service
            .access(
                &s,
                &sessions,
                &p.label,
                &token,
                "clipboard.mutate",
                config(1, false)
            )
            .is_err());
        s.save_preference("hidden_all", "true").unwrap();
        assert!(service.prepare_popup(&s, &history.0, None, None).is_err());
    }
    #[test]
    fn clipboard_host_production_action_uses_bound_popup_and_request_retry_cannot_repeat_effects() {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, HISTORY, CAPABILITY);
        configure(&service, &s, &sessions, &row, 0, true);
        clipboard_history::capture(&s, &row.0, 1, "正式选择项", now_ms()).unwrap();
        let item = clipboard_history::query(&s, &row.0, json!({})).unwrap()["items"][0].clone();
        let (p, token) = popup(&service, &s, &sessions, &row.0);
        let req = action_request(&p, &item, false);
        let mut io = Actions::default();
        let first = service
            .act_with(&s, &sessions, &p.label, &token, req.clone(), &mut io)
            .unwrap();
        assert_eq!(first["copy"], "copied");
        assert_eq!(first["state"], "completed");
        assert_eq!(
            service
                .act_with(&s, &sessions, &p.label, &token, req.clone(), &mut io)
                .unwrap()["replayed"],
            true
        );
        assert_eq!(io.writes, 1);
        assert!(service
            .act_with(&s, &sessions, &row.1, &row.2, req, &mut io)
            .is_err());
        assert_eq!(io.writes, 1);
    }
    #[test]
    fn clipboard_host_replaced_popup_or_changed_config_between_copy_and_input_cancels_remaining_effects(
    ) {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, HISTORY, CAPABILITY);
        let mut settings = config(0, true);
        settings["command"]["pasteEnabled"] = json!(true);
        service
            .access(&s, &sessions, &row.1, &row.2, "clipboard.mutate", settings)
            .unwrap();
        clipboard_history::capture(&s, &row.0, 1, "版本保护", now_ms()).unwrap();
        let item = clipboard_history::query(&s, &row.0, json!({})).unwrap()["items"][0].clone();
        let (p, token) = popup(&service, &s, &sessions, &row.0);
        let req = action_request(&p, &item, true);
        let mut io = Actions {
            on_prepare: Some(Box::new(|| {
                configure(&service, &s, &sessions, &row, 1, false);
            })),
            ..Default::default()
        };
        let result = service
            .act_with(&s, &sessions, &p.label, &token, req.clone(), &mut io)
            .unwrap();
        assert_ne!(result["state"], "completed");
        assert_eq!((io.writes, io.sends), (1, 0));
        assert!(service
            .act_with(&s, &sessions, &p.label, &token, req, &mut io)
            .is_err());
        assert_eq!(io.writes, 1);
    }
    #[test]
    fn clipboard_host_plain_close_of_popup_does_not_stop_enabled_collector() {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, HISTORY, CAPABILITY);
        configure(&service, &s, &sessions, &row, 0, true);
        let (p, token) = popup(&service, &s, &sessions, &row.0);
        service
            .access(
                &s,
                &sessions,
                &p.label,
                &token,
                "clipboard.hide",
                Value::Null,
            )
            .unwrap();
        assert!(!service.surfaces.lock().unwrap().current(&p.label, &row.0));
        let mut io = Fake::new(1);
        service.step(&s, &mut io).unwrap();
        io.seq = 2;
        io.text("弹窗隐藏后仍采集");
        service.step(&s, &mut io).unwrap();
        assert_eq!(clipboard_history::status(&s, &row.0).unwrap()["count"], 1);
    }
    #[derive(Default)]
    struct ShortcutIo {
        registered: Vec<u32>,
        released: Vec<u32>,
    }
    impl Driver for ShortcutIo {
        fn register(&mut self, key: tauri_plugin_global_shortcut::Shortcut) -> AppResult<()> {
            self.registered.push(key.id());
            Ok(())
        }
        fn unregister(&mut self, key: tauri_plugin_global_shortcut::Shortcut) -> AppResult<()> {
            self.released.push(key.id());
            Ok(())
        }
    }
    #[test]
    fn clipboard_host_shortcuts_are_default_off_report_actual_registration_and_disable_stale_events(
    ) {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, HISTORY, CAPABILITY);
        let mut io = ShortcutIo::default();
        service.shortcuts_with(&s, &mut io).unwrap();
        assert!(io.registered.is_empty());
        let mut c = config(0, false);
        c["command"]["shortcutEnabled"] = json!(true);
        service
            .access(&s, &sessions, &row.1, &row.2, "clipboard.mutate", c)
            .unwrap();
        service.shortcuts_with(&s, &mut io).unwrap();
        assert_eq!(io.registered.len(), 1);
        assert_eq!(
            service
                .access(
                    &s,
                    &sessions,
                    &row.1,
                    &row.2,
                    "clipboard.status",
                    Value::Null
                )
                .unwrap()["shortcutRegistered"],
            true
        );
        let spec = Spec {
            instance: row.0.clone(),
            revision: 1,
            shortcut: "Super+Alt+V".into(),
        };
        let ShortcutAction::Show(p) = service.shortcut_action(&s, &spec, None).unwrap() else {
            panic!("应呼出历史弹窗")
        };
        service
            .surfaces
            .lock()
            .unwrap()
            .attach(&row.0, &p.id, 42)
            .unwrap();
        service
            .surfaces
            .lock()
            .unwrap()
            .shown(&row.0, &p.id, now_ms())
            .unwrap();
        assert!(matches!(
            service.shortcut_action(&s, &spec, None).unwrap(),
            ShortcutAction::Hide(_)
        ));
        configure(&service, &s, &sessions, &row, 1, false);
        service.shortcuts_with(&s, &mut io).unwrap();
        assert_eq!(io.released.len(), 1);
        assert!(service.shortcut_action(&s, &spec, None).is_err());
    }
    #[test]
    fn clipboard_host_search_shortcut_uses_selected_original_engine_and_encodes_complete_text_without_popup(
    ) {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, SEARCH, CAPABILITY);
        let mut c = config(0, true);
        c["command"]["shortcutEnabled"] = json!(true);
        c["command"]["shortcut"] = json!("Super+Alt+S");
        c["command"]["searchPlatform"] = json!("bing");
        service
            .access(&s, &sessions, &row.1, &row.2, "clipboard.mutate", c)
            .unwrap();
        let mut capture = Fake::new(1);
        service.step(&s, &mut capture).unwrap();
        capture.seq = 2;
        capture.text("a&b #中文🙂");
        service.step(&s, &mut capture).unwrap();
        let mut io = ShortcutIo::default();
        service.shortcuts_with(&s, &mut io).unwrap();
        let spec = Spec {
            instance: row.0.clone(),
            revision: 1,
            shortcut: "Super+Alt+S".into(),
        };
        let ShortcutAction::Search(url) = service.shortcut_action(&s, &spec, None).unwrap() else {
            panic!("应直接搜索而非改成呼出弹窗")
        };
        assert_eq!(url.host_str(), Some("cn.bing.com"));
        assert_eq!(
            url.query_pairs().find(|(k, _)| k == "q").unwrap().1,
            "a&b #中文🙂"
        );
        assert!(url.fragment().is_none());
        assert!(service.surfaces.lock().unwrap().entries.is_empty());
    }
}
