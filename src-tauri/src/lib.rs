mod ai_credentials;
mod ai_host;
mod ai_protocol;
mod ai_transport;
mod alert_notice;
mod alert_store;
pub mod alerts;
mod asset_permissions;
mod assistant_host;
mod assistant_state;
mod assistant_webviews;
mod business;
mod clipboard_actions;
mod clipboard_capture;
mod clipboard_history;
mod clipboard_host;
mod clipboard_native;
mod clipboard_shortcuts;
mod clipboard_surface;
mod clipboard_tools;
mod coin;
mod coin_live;
mod component_lifecycle;
mod component_registry;
mod content;
mod content_cache;
pub mod desktop_geometry;
mod desktop_grid;
#[cfg(windows)]
mod desktop_input;
#[cfg(windows)]
mod desktop_native;
mod desktop_probe;
mod desktop_widgets;
mod documents;
mod equity;
mod hardware;
mod hardware_details;
mod integration;
mod job_host;
mod job_knowledge;
mod job_knowledge_host;
mod job_protocol;
mod job_state;
mod job_surfaces;
mod job_webviews;
mod key_capture;
mod keyboard;
mod keyboard_host;
mod labor;
mod launcher;
pub mod market;
mod market_history;
mod market_replay;
mod market_runtime;
mod media;
pub mod network;
mod notification_sdk;
mod official_gallery;
mod packages;
mod panel;
mod pomodoro;
mod pomodoro_window;
mod pomodoro_window_host;
mod pomodoro_window_native;
mod power;
mod presets;
mod recovery;
mod reminder_engine;
mod reminder_host;
mod reminder_state;
mod reminder_store;
mod server_monitor;
mod shared_library;
mod soak;
mod store;
mod todo;
mod updates;
mod verification;
mod view_probe;
mod water;
mod weather;
mod website_metadata;
mod website_verification;
mod widgets;
mod windows;

/// 与正常应用完全分离的桌面平台验证入口。
pub fn run_desktop_probe(report: std::path::PathBuf) -> Result<(), String> {
    desktop_probe::run(report, None)
}
/// 使用本次独立测试库加载外部普通组件包，不启动正常业务后台。
pub fn run_desktop_widget_probe(
    report: std::path::PathBuf,
    packages: [std::path::PathBuf; 2],
) -> Result<(), String> {
    desktop_probe::run(report, Some(packages))
}

use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use store::{err, now_ms, AppResult, Store};
use tauri::{
    ipc::{InvokeBody, Request},
    tray::{MouseButton, MouseButtonState, TrayIconEvent},
    AppHandle, Emitter, Manager, State,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
#[cfg(not(windows))]
use tauri_plugin_notification::NotificationExt;
use uuid::Uuid;

const MAIN_TRAY: &str = "souldesk-main";

#[derive(Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum AppTheme {
    #[default]
    Ink,
    #[serde(rename = "ice-light")]
    IceLight,
    Pine,
    Oat,
    Iris,
}

impl AppTheme {
    const KEY: &str = "app_theme";

    fn id(self) -> &'static str {
        match self {
            Self::Ink => "ink",
            Self::IceLight => "ice-light",
            Self::Pine => "pine",
            Self::Oat => "oat",
            Self::Iris => "iris",
        }
    }

    fn stored(store: &Store) -> Self {
        let value = store.preference(Self::KEY);
        [Self::Ink, Self::IceLight, Self::Pine, Self::Oat, Self::Iris]
            .into_iter()
            .find(|theme| Some(theme.id()) == value.as_deref())
            .unwrap_or_default()
    }

    fn chrome(self) -> tauri::Theme {
        match self {
            Self::Ink | Self::Pine => tauri::Theme::Dark,
            Self::IceLight | Self::Oat | Self::Iris => tauri::Theme::Light,
        }
    }
}

/// 仅供显式命令行恢复验收使用；不是组件网关能力。
pub async fn run_recovery_probe(report: std::path::PathBuf) -> Result<(), String> {
    recovery::run(report, false).await
}
pub async fn run_sleep_probe(report: std::path::PathBuf) -> Result<(), String> {
    recovery::run(report, true).await
}
pub struct Core {
    ai: ai_host::Service,
    assistant: assistant_host::Service,
    jobs: job_host::Service,
    clipboard: clipboard_host::Service,
    business: business::Service,
    market_role: market_runtime::Role,
    coin_live: coin_live::Service,
    coin_index: coin::IndexCache,
    content_cache: content_cache::Service,
    website_verification: website_verification::State,
    hardware: hardware::Service,
    keyboard: keyboard_host::Service,
    reminders: reminder_host::Service,
    water: water::Service,
    labor: labor::Service,
    todo: todo::Service,
    pomodoro: pomodoro::Service,
    pomodoro_windows: pomodoro_window_native::Coordinator,
    store: Arc<Store>,
    sessions: widgets::Sessions,
    visibility: Mutex<windows::Visibility>,
    window_actions: Mutex<()>,
    quitting: AtomicBool,
    ticks: AtomicU64,
    active_tasks: AtomicU64,
    last_error: Arc<Mutex<Option<String>>>,
    verification: Option<verification::Config>,
    panel_shortcut_registered: AtomicBool,
    shortcut_hits: AtomicU64,
    shortcut_trace: Mutex<Option<Value>>,
    second_launches: AtomicU64,
    edge_hits: AtomicU64,
    host_token: String,
    view_reports: Mutex<std::collections::BTreeMap<String, Value>>,
    resource_reports: Mutex<Vec<Value>>,
    installing: AtomicBool,
    trials: Mutex<std::collections::HashMap<String, std::sync::Arc<Mutex<widgets::Preview>>>>,
    geometry_versions: Mutex<std::collections::HashMap<String, u64>>,
    window_tasks: AtomicU64,
    integration: Option<integration::Config>,
    transfer_failure: Mutex<Option<String>>,
    updates: updates::State,
}
impl Core {
    fn log_error(&self, error: &str) {
        eprintln!("SoulDesk: {error}");
        if let Err(e) = self.store.log("error", error) {
            eprintln!("诊断日志保存失败：{e}");
        }
    }

    fn report(&self, error: String) {
        self.log_error(&error);
        if let Ok(mut e) = self.last_error.lock() {
            *e = Some(error);
        }
    }
}

#[tauri::command]
async fn snapshot(
    app: AppHandle,
    window: tauri::WebviewWindow,
    core: State<'_, Core>,
) -> AppResult<Value> {
    let mut value = core.store.snapshot()?;
    {
        let visibility = core.visibility.lock().map_err(err)?;
        value["hiddenAll"] = json!(visibility.hidden);
        value["panelLocked"] = json!(visibility.panel_locked);
        if window.label() == "panel" {
            let group = value["panelGroups"]["active"].as_str().map(str::to_owned);
            value["panelScroll"] = json!(visibility
                .panel_scroll
                .get(&group)
                .copied()
                .unwrap_or_default());
            value["panelTarget"] = json!(visibility.panel_target);
        }
    }
    value["panelOpen"] = json!(app.get_webview_window("panel").is_some());
    value["panelLowered"] =
        json!(value["panelOpen"] == true && !windows::scene_is_foreground(windows::panel_scene()));
    value["panelShortcutEnabled"] = json!(panel_shortcut_enabled(&core.store));
    if panel_shortcut_available(&core) {
        value["panelShortcutRegistered"] =
            json!(core.panel_shortcut_registered.load(Ordering::Relaxed));
    }
    value["panel"] = windows::panel_preferences(&app)?;
    value["panelGeometry"] = windows::panel_metrics(&app)?;
    value["panelLayouts"] = core.store.panel_layouts()?;
    value["widgetBackdrop"] =
        json!(core.store.preference("widget_backdrop").as_deref() == Some("true"));
    value["appTheme"] = json!(AppTheme::stored(&core.store).id());
    value["updates"] = updates::snapshot(&app)?;
    value["autostart"] = json!(
        if core.integration.is_none() && core.verification.is_none() {
            windows::autostart_enabled().ok()
        } else {
            None
        }
    );
    value["silentStartup"] = json!(windows::silent_startup(&core.store));
    value["windowTasks"] = json!(core.window_tasks.load(Ordering::Relaxed));
    value["trialCount"] = json!(core.trials.lock().map_err(err)?.len());
    value["diagnostics"] = json!({"ticks":core.ticks.load(Ordering::Relaxed),"tasks":core.active_tasks.load(Ordering::Relaxed),"sessions":core.sessions.0.lock().map_err(err)?.len(),"views":app.webview_windows().len(),"error":core.last_error.lock().map_err(err)?.clone(),"dataRoot":core.store.root,"shortcut":panel_shortcut(&core.store),"docked":core.visibility.lock().map_err(err)?.docked});
    Ok(value)
}
#[tauri::command]
async fn set_autostart(
    app: AppHandle,
    enabled: Option<bool>,
    silent: Option<bool>,
) -> AppResult<()> {
    let core = app.state::<Core>();
    match (enabled, silent) {
        (Some(enabled), None) if core.integration.is_none() && core.verification.is_none() => {
            windows::set_autostart(enabled)
        }
        (Some(_), None) => Err("隔离环境无法更改 Windows 开机启动项".into()),
        (None, Some(silent)) => core.store.save_preference(
            windows::SILENT_STARTUP,
            if silent { "true" } else { "false" },
        ),
        _ => Err("请一次修改一项启动设置".into()),
    }
}

#[tauri::command]
async fn import_package(app: AppHandle, request: Request<'_>) -> AppResult<Value> {
    use tauri_plugin_dialog::DialogExt;
    let result = if let InvokeBody::Raw(bytes) = request.body() {
        packages::install_bytes(&app, bytes).await?
    } else {
        let core = app.state::<Core>();
        {
            let mut visibility = core.visibility.lock().map_err(err)?;
            if visibility.dialog {
                return Err("BUSY: 请先完成当前文件对话框".into());
            }
            visibility.dialog = true;
        }
        let picker = app
            .dialog()
            .file()
            .add_filter("SoulDesk 组件包", &["zip"])
            .blocking_pick_file();
        core.visibility.lock().map_err(err)?.dialog = false;
        let Some(path) = picker else {
            return Ok(Value::Null);
        };
        let path = path.into_path().map_err(err)?;
        packages::install(&app, &path).await?
    };
    app.emit("host:changed", ()).map_err(err)?;
    serde_json::to_value(result).map_err(err)
}

#[tauri::command]
async fn create_instance(app: AppHandle, package_id: String) -> AppResult<String> {
    if app.state::<Core>().visibility.lock().map_err(err)?.hidden {
        return Err("请先恢复组件显示".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        windows::serialized(&app, |app| {
            if windows::hidden(app)? {
                return Err("请先恢复组件显示".into());
            }
            let id = widgets::create_instance_at(&app.state::<Core>().store, &package_id, "panel")?;
            windows::show_instance(app, &id)
                .map_err(|e| format!("实例已保留，可在管理中重试显示：{e}"))?;
            Ok(id)
        })
    })
    .await
    .map_err(err)?
}
#[tauri::command]
async fn manage_instance(
    app: AppHandle,
    instance: String,
    action: String,
    width: Option<u32>,
    height: Option<u32>,
) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        windows::serialized(&app, |app| {
            match action.as_str() {
                "resize" => windows::resize_instance(
                    app,
                    &instance,
                    width.ok_or("缺少宽度")?,
                    height.ok_or("缺少高度")?,
                )?,
                "remove" => windows::remove_instance(app, &instance)?,
                _ => return Err("INVALID_ARGUMENT: 未知实例操作".into()),
            }
            windows::changed(app)
        })
    })
    .await
    .map_err(err)?
}
#[tauri::command]
async fn reorder_instances(app: AppHandle, ids: Vec<String>) -> AppResult<()> {
    widgets::reorder_instances(&app.state::<Core>().store, &ids)?;
    app.emit("host:changed", ()).map_err(err)
}
#[tauri::command]
async fn unregister_package(app: AppHandle, package_id: String) -> AppResult<()> {
    widgets::unregister_package(&app.state::<Core>().store, &package_id)?;
    app.emit("host:changed", ()).map_err(err)
}
#[tauri::command]
fn assistant_open_session(
    webview: tauri::Webview,
    core: State<'_, Core>,
    instance: String,
    expected_digest: Option<String>,
) -> AppResult<Value> {
    let _native = core.store.native_actions.lock().map_err(err)?;
    if !assistant_host::shell_role(webview.label(), &instance)
        || !core.assistant.current_window(webview.label(), &instance)
        || webview.window().label()
            != webview
                .label()
                .replacen("assistant-shell-", "assistant-panel-", 1)
    {
        return Err("PRESENTATION_EXPIRED: 当前视图不是有效的助手本机壳".into());
    }
    let value = widgets::open_display_session(
        &core.store,
        &core.sessions,
        webview.label(),
        &instance,
        expected_digest.as_deref(),
    )?;
    let token = value["token"]
        .as_str()
        .ok_or("SESSION_EXPIRED: 助手会话未建立")?;
    if let Err(error) = core.assistant.bind(webview.label(), &instance, token) {
        core.sessions.0.lock().map_err(err)?.remove(token);
        return Err(error);
    }
    core.assistant.kick(webview.app_handle(), &instance);
    Ok(value)
}
#[tauri::command]
fn assistant_close_session(
    webview: tauri::Webview,
    core: State<'_, Core>,
    token: String,
) -> AppResult<()> {
    if !webview.label().starts_with("assistant-shell-") {
        return Err("CAPABILITY_DENIED: 仅助手本机壳可关闭此会话".into());
    }
    let _native = core.store.native_actions.lock().map_err(err)?;
    let mut sessions = core.sessions.0.lock().map_err(err)?;
    if sessions
        .get(&token)
        .is_some_and(|s| s.window == webview.label())
    {
        let instance = core.assistant.close_session(webview.label(), &token)?;
        sessions.remove(&token);
        if let Some(id) = instance {
            core.assistant.kick(webview.app_handle(), &id);
        }
    }
    Ok(())
}
#[tauri::command]
fn assistant_widget_request(
    webview: tauri::Webview,
    core: State<'_, Core>,
    token: String,
    method: String,
    args: Value,
) -> AppResult<Value> {
    if !webview.label().starts_with("assistant-shell-") {
        return Err("CAPABILITY_DENIED: 仅助手本机壳可调用此接口".into());
    }
    if matches!(method.as_str(), "settings.get") {
        // 沿用通用只读设置契约，仍先核对当前助手会话，不能借此访问其他能力。
        core.assistant.access(
            &core.store,
            &core.sessions,
            webview.label(),
            &token,
            "assistant.status",
            Value::Null,
        )?;
        if !(args.is_null() || args.as_object().is_some_and(|o| o.is_empty())) {
            return Err("INVALID_ARGUMENT: 读取设置不接受参数".into());
        }
        return widgets::gateway(
            &core.store,
            &core.sessions,
            webview.label(),
            &token,
            &method,
            args,
        );
    }
    let (id, value) = core.assistant.access(
        &core.store,
        &core.sessions,
        webview.label(),
        &token,
        &method,
        args,
    )?;
    if method != "assistant.status" {
        core.assistant.kick(webview.app_handle(), &id);
    }
    Ok(value)
}
#[tauri::command]
fn assistant_surface_failed(
    webview: tauri::Webview,
    core: State<'_, Core>,
    instance: String,
    _message: Option<String>,
) -> AppResult<()> {
    if !assistant_host::shell_role(webview.label(), &instance) {
        return Err("CAPABILITY_DENIED: 当前视图不是助手本机壳".into());
    }
    core.assistant.shell_failed(webview.label(), &instance)?;
    core.assistant.kick(webview.app_handle(), &instance);
    Ok(())
}
#[tauri::command]
fn job_open_session(
    webview: tauri::Webview,
    core: State<'_, Core>,
    instance: String,
    expected_digest: Option<String>,
) -> AppResult<Value> {
    let _native = core.store.native_actions.lock().map_err(err)?;
    let surface = core
        .jobs
        .surfaces
        .lock()
        .map_err(err)?
        .shell(webview.label(), &instance)
        .ok_or("PRESENTATION_EXPIRED: 当前视图不是有效AI直聘本机壳")?;
    if !job_surfaces::shell_role(webview.label(), &instance)
        || !core.jobs.current_window(webview.label(), &instance)
        || webview.window().label() != surface.parent()
    {
        return Err("CAPABILITY_DENIED: AI直聘本机壳父窗口不符".into());
    }
    let value = widgets::open_display_session(
        &core.store,
        &core.sessions,
        webview.label(),
        &instance,
        expected_digest.as_deref(),
    )?;
    let token = value["token"]
        .as_str()
        .ok_or("SESSION_EXPIRED: AI直聘会话未建立")?;
    if let Err(error) = core
        .jobs
        .bind(&core.store, webview.label(), &instance, token)
    {
        core.sessions.0.lock().map_err(err)?.remove(token);
        return Err(error);
    }
    core.jobs.kick(webview.app_handle(), &instance);
    Ok(value)
}
#[tauri::command]
fn job_close_session(
    webview: tauri::Webview,
    core: State<'_, Core>,
    token: String,
) -> AppResult<()> {
    if job_surfaces::role(webview.label()).is_none() {
        return Err("CAPABILITY_DENIED: 仅AI直聘本机壳可关闭此会话".into());
    }
    let _native = core.store.native_actions.lock().map_err(err)?;
    let mut sessions = core.sessions.0.lock().map_err(err)?;
    if let Some(instance) = sessions
        .get(&token)
        .filter(|s| s.window == webview.label())
        .map(|s| s.instance.clone())
    {
        core.jobs
            .surfaces
            .lock()
            .map_err(err)?
            .close_session(webview.label(), &token)?;
        sessions.remove(&token);
        core.jobs.kick(webview.app_handle(), &instance);
    }
    Ok(())
}
#[tauri::command]
async fn job_widget_request(
    webview: tauri::Webview,
    token: String,
    method: String,
    args: Value,
) -> AppResult<Value> {
    if job_surfaces::role(webview.label()).is_none() {
        return Err("CAPABILITY_DENIED: 仅AI直聘本机壳可调用此接口".into());
    }
    let app = webview.app_handle();
    if matches!(method.as_str(), "settings.get" | "runtime.ready") {
        let core = app.state::<Core>();
        let (info, _) = core.jobs.access(
            &core.store,
            &core.sessions,
            webview.label(),
            &token,
            "jobs.surface.info",
            Value::Null,
        )?;
        let value = widgets::gateway(
            &core.store,
            &core.sessions,
            webview.label(),
            &token,
            &method,
            args,
        )?;
        if method == "runtime.ready" {
            core.jobs
                .surfaces
                .lock()
                .map_err(err)?
                .ready(webview.label(), &token)?;
            if let Some(i) = info["instanceId"].as_str() {
                core.jobs.kick(app, i);
            }
        }
        return Ok(value);
    }
    if !method.starts_with("jobs.") {
        return Err("CAPABILITY_DENIED: AI直聘本机壳只开放固定业务和只读设置".into());
    }
    job_host::gateway(app, webview.label(), &token, &method, args).await
}
#[tauri::command]
fn job_surface_failed(
    webview: tauri::Webview,
    core: State<'_, Core>,
    instance: String,
    _message: Option<String>,
) -> AppResult<()> {
    if !job_surfaces::shell_role(webview.label(), &instance) {
        return Err("CAPABILITY_DENIED: 当前视图不是AI直聘本机壳".into());
    }
    core.jobs
        .shell_failed(&core.store, webview.label(), &instance)?;
    core.jobs.kick(webview.app_handle(), &instance);
    Ok(())
}
#[tauri::command]
fn open_session(
    window: tauri::WebviewWindow,
    core: State<'_, Core>,
    instance: String,
    expected_digest: Option<String>,
) -> AppResult<Value> {
    if let Some(value) = packages::session(&core, window.label(), &instance)? {
        return Ok(value);
    }
    if reminder_host::surface_role(window.label(), &instance).is_some()
        && !core.reminders.current_window(window.label(), &instance)
    {
        return Err("SESSION_EXPIRED: 专用显示已被关闭或替换".into());
    }
    if pomodoro_window::role(window.label(), &instance)
        && !core
            .pomodoro_windows
            .service
            .current_window(window.label(), &instance)
    {
        return Err("PRESENTATION_EXPIRED: 番茄专用窗已关闭或替换".into());
    }
    if clipboard_surface::role(window.label(), &instance).is_some()
        && !core.clipboard.current_window(window.label(), &instance)
    {
        return Err("PRESENTATION_EXPIRED: 剪切板专用显示已关闭或替换".into());
    }
    if clipboard_tools::role(window.label(), &instance).is_some()
        && !core.ai.current_tool(&core.store, window.label(), &instance)
    {
        return Err("PRESENTATION_EXPIRED: 剪切板工具已关闭或替换".into());
    }
    let mut value = widgets::open_display_session(
        &core.store,
        &core.sessions,
        window.label(),
        &instance,
        expected_digest.as_deref(),
    )?;
    if pomodoro_window::role(window.label(), &instance) {
        let token = value["token"]
            .as_str()
            .ok_or("SESSION_EXPIRED: 专用窗会话缺失")?;
        if let Err(error) = core.pomodoro_windows.service.bind_session(
            &core.store,
            window.label(),
            &instance,
            token,
        ) {
            core.sessions.0.lock().map_err(err)?.remove(token);
            return Err(error);
        }
    }
    value["verify"] = json!(core.verification.is_some());
    if core
        .integration
        .as_ref()
        .is_some_and(|c| matches!(c.mode.as_str(), "widgetjs" | "widgetjs-resume"))
    {
        value["verify"] = json!(true);
        value["verificationMode"] = json!("widgetjs");
        if std::env::args().any(|arg| arg == "--widgetjs-check-package=convert") {
            value["verificationMode"] = json!("widgetjs-convert-render");
        }
        if std::env::args().any(|arg| arg == "--widgetjs-check-package=water-reminder") {
            value["verificationMode"] = json!("widgetjs-water");
        }
        if std::env::args().any(|arg| arg == "--widgetjs-check-package=labor") {
            value["verificationMode"] = json!("widgetjs-labor");
        }
        if std::env::args().any(|arg| arg == "--widgetjs-check-package=server") {
            value["verificationMode"] = json!("widgetjs-server");
        }
        if std::env::args().any(|arg| arg == "--widgetjs-check-package=hotspots") {
            value["verificationMode"] = json!("widgetjs-hotspots");
        }
        if std::env::args().any(|arg| arg == "--widgetjs-check-package=coins") {
            value["verificationMode"] = json!("widgetjs-coins");
        }
        if std::env::args().any(|arg| {
            matches!(
                arg.as_str(),
                "--widgetjs-check-package=photo" | "--widgetjs-check-package=sticker"
            )
        }) {
            value["verificationMode"] = json!("widgetjs-photo");
        }
    }
    Ok(value)
}
#[tauri::command]
fn close_session(
    window: tauri::WebviewWindow,
    core: State<'_, Core>,
    token: String,
) -> AppResult<()> {
    crate::component_lifecycle::close_session(&core, window.label(), &token)
}
#[tauri::command]
async fn widget_request(
    app: AppHandle,
    window: tauri::WebviewWindow,
    core: State<'_, Core>,
    token: String,
    method: String,
    args: Value,
) -> AppResult<Value> {
    use component_registry::Gateway;
    let owner = component_registry::gateway(&method);
    keyboard_host::overlay_method(window.label(), &method)?;
    reminder_host::surface_method(window.label(), &method)?;
    clipboard_surface::surface_method(window.label(), &method)?;
    pomodoro_window_native::surface_method(window.label(), &method)?;
    if owner == Gateway::PomodoroWindow {
        return pomodoro_window_native::gateway(&app, window.label(), &token, &method, args).await;
    }
    if owner == Gateway::Jobs {
        return job_host::gateway(&app, window.label(), &token, &method, args).await;
    }
    if window.label().starts_with("clipboard-tool-") {
        let core = app.state::<Core>();
        let instance = core
            .sessions
            .0
            .lock()
            .map_err(err)?
            .get(&token)
            .filter(|s| s.window == window.label() && s.preview.is_none())
            .map(|s| s.instance.clone())
            .ok_or("SESSION_EXPIRED: 工具会话无效")?;
        if !core.ai.current_tool(&core.store, window.label(), &instance) {
            return Err("PRESENTATION_EXPIRED: 工具已关闭或替换".into());
        }
    }
    if owner == Gateway::Reminders {
        return reminder_host::gateway(&app, window.label(), &token, &method, args).await;
    }
    if owner == Gateway::Keyboard {
        return keyboard_host::gateway(&app, window.label(), &token, &method, args).await;
    }
    if owner == Gateway::Assistant {
        let (id, value) = core.assistant.access(
            &core.store,
            &core.sessions,
            window.label(),
            &token,
            &method,
            args,
        )?;
        if method != "assistant.status" {
            core.assistant.kick(&app, &id);
        }
        return Ok(value);
    }
    if owner == Gateway::Details {
        let id = {
            let sessions = core.sessions.0.lock().map_err(err)?;
            let s = sessions
                .get(&token)
                .filter(|s| s.window == window.label() && s.preview.is_none())
                .ok_or("SESSION_EXPIRED: 无有效展示会话")?;
            s.instance.clone()
        };
        if method == "view.resizeMarket" {
            widgets::require_capability(
                &core.store,
                &core.sessions,
                window.label(),
                &token,
                "market.chart",
            )?;
            if window.label().starts_with("detail-")
                || args.as_object().is_none_or(|value| value.len() != 1)
            {
                return Err("INVALID_ARGUMENT: 仅主组件可选择尺寸".into());
            }
            let (width, height) = match args["size"].as_str() {
                Some("small") => (320, 380),
                Some("medium") => (430, 460),
                Some("large") => (540, 530),
                _ => return Err("INVALID_ARGUMENT: 组件尺寸无效".into()),
            };
            let handle = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                windows::serialized(&handle, |app| {
                    windows::resize_instance(app, &id, width, height)
                })
            })
            .await
            .map_err(err)??;
            return Ok(Value::Null);
        }
        let page = args["page"].as_str().unwrap_or("quotes").to_owned();
        if window.label().starts_with("pomodoro-window-") {
            if !matches!(
                page.as_str(),
                "pomodoro-scenes" | "pomodoro-settings" | "pomodoro-history"
            ) {
                return Err("CAPABILITY_DENIED: 专用窗只打开番茄钟详情".into());
            }
            core.pomodoro_windows.service.business(
                &core.store,
                &core.sessions,
                window.label(),
                &token,
                "pomodoro.get",
                Value::Null,
                now_ms(),
            )?;
        }
        if let Some(capability) = windows::detail_capability(&page)? {
            if args.as_object().is_none_or(|value| value.len() != 1) {
                return Err("INVALID_ARGUMENT: 详情仅接受固定page字段".into());
            }
            widgets::require_capability(
                &core.store,
                &core.sessions,
                window.label(),
                &token,
                capability,
            )?;
        }
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            windows::serialized(&handle, |app| windows::detail_at(app, &id, &page))
        })
        .await
        .map_err(err)??;
        return Ok(Value::Null);
    }
    if owner == Gateway::CoinLive {
        return coin_live::gateway(&app, window.label(), &token, args).await;
    }
    if owner == Gateway::Weather {
        return weather::gateway(&app, window.label(), &token, &method, args).await;
    }
    if owner == Gateway::Ai {
        return ai_host::gateway(&app, window.label(), &token, &method, args).await;
    }
    if owner == Gateway::Clipboard {
        return clipboard_host::gateway(&app, window.label(), &token, &method, args).await;
    }
    if owner == Gateway::Metrics {
        return hardware::gateway(&app, window.label(), &token, args).await;
    }
    if owner == Gateway::Server {
        return server_monitor::gateway(&app, window.label(), &token, args).await;
    }
    if owner == Gateway::Labor {
        return labor::gateway(&app, window.label(), &token, args).await;
    }
    if owner == Gateway::Water {
        return water::gateway(&app, window.label(), &token, &method, args).await;
    }
    if owner == Gateway::Todo {
        return todo::gateway(&app, window.label(), &token, &method, args).await;
    }
    if owner == Gateway::Pomodoro {
        if window.label().starts_with("pomodoro-window-") {
            return core.pomodoro_windows.service.business(
                &core.store,
                &core.sessions,
                window.label(),
                &token,
                &method,
                args,
                now_ms(),
            );
        }
        return pomodoro::gateway(&app, window.label(), &token, &method, args).await;
    }
    if owner == Gateway::Content {
        return content::gateway(&app, window.label(), &token, &method, args).await;
    }
    if owner == Gateway::Documents {
        return documents::gateway(&app, window.label(), &token, &method, args).await;
    }
    if owner == Gateway::Media {
        return media::gateway(&app, window.label(), &token, &method, args).await;
    }
    if owner == Gateway::Files {
        return launcher::gateway(&app, window.label(), &token, &method, args).await;
    }
    if owner == Gateway::Market {
        return business::gateway(&app, window.label(), &token, &method, args).await;
    }
    let reload_instance = if matches!(method.as_str(), "library.bind" | "library.create") {
        core.sessions
            .0
            .lock()
            .map_err(err)?
            .get(&token)
            .filter(|session| session.window == window.label() && session.preview.is_none())
            .map(|session| session.instance.clone())
    } else {
        None
    };
    let result = widgets::gateway(
        &core.store,
        &core.sessions,
        window.label(),
        &token,
        &method,
        args,
    );
    if result
        .as_ref()
        .is_ok_and(|value| value["reloadRequired"] == true)
    {
        if let Some(instance) = reload_instance {
            if let Err(error) = app.emit("host:reload-instance", &instance) {
                core.report(format!("数据源已切换，但其他视图重载通知失败：{error}"));
            }
        }
    }
    if method == "runtime.ready" && result.is_ok() {
        let id = core
            .sessions
            .0
            .lock()
            .map_err(err)?
            .get(&token)
            .map(|s| s.instance.clone());
        if let Some(id) = id {
            if pomodoro_window::role(window.label(), &id) {
                core.pomodoro_windows
                    .service
                    .book
                    .lock()
                    .map_err(err)?
                    .ready(window.label(), &token, core.pomodoro_windows.now())?;
                core.pomodoro_windows.kick(&app);
            } else {
                windows::ready(&app, window.label(), &id)?;
            }
        }
    }
    result
}

#[tauri::command]
async fn business_settings(
    app: AppHandle,
    action: String,
    config: Option<Value>,
) -> AppResult<Value> {
    let core = app.state::<Core>();
    match action.as_str() {
        "get" => Ok(
            json!({"config":core.store.preference("network").and_then(|v|serde_json::from_str::<Value>(&v).ok()).unwrap_or(json!({"mode":"system"})),
                "market":core.business.feed.snapshot()?,
                "monitorGaps":market_replay::history(&core.store)?}),
        ),
        "set" => {
            let config = config.ok_or("INVALID_ARGUMENT: 缺少网络配置")?;
            let network = business::resolve_network(&config)?;
            core.store.save_preference("network", &config.to_string())?;
            for feed in [&core.business.feed, &core.business.spot] {
                feed.desired.send_modify(|d| d.network = network.clone());
            }
            core.business.network_ready.store(true, Ordering::Release);
            core.business.reconnect();
            Ok(Value::Null)
        }
        "reconnect" => {
            if core.market_role.is_desktop() {
                market_runtime::request_reconnect(&core.store)?;
            }
            core.business.reconnect();
            Ok(Value::Null)
        }
        "backup" => Ok(json!(core.store.backup()?)),
        _ => Err("INVALID_ARGUMENT: 未知网络操作".into()),
    }
}

#[tauri::command]
async fn window_action(
    app: AppHandle,
    window: tauri::WebviewWindow,
    action: String,
    instance: Option<String>,
    theme: Option<AppTheme>,
) -> AppResult<()> {
    let label = window.label();
    let allowed = if matches!(action.as_str(), "lock-panel" | "unlock-panel") {
        label == "panel" && instance.is_none()
    } else if matches!(
        action.as_str(),
        "enable-widget-backdrop"
            | "disable-widget-backdrop"
            | "set-app-theme"
            | "desktop-grid"
            | "desktop-grid-restore"
    ) {
        label == "main" && instance.is_none()
    } else if label == "main" {
        action != "resize-panel"
    } else if label == "edge" {
        ["edge-hover", "edge-activate"].contains(&action.as_str())
    } else if label.starts_with("detail-") {
        action == "close-detail"
    } else if label == "panel" {
        [
            "show",
            "settings",
            "collapse",
            "edge-activate",
            "float",
            "hide",
            "remove",
            "detail",
            "resize-panel",
        ]
        .contains(&action.as_str())
            && instance.as_ref().is_none_or(|id| {
                windows::instance(&app, id).is_ok_and(|i| i["location"] == "panel")
            })
    } else if let Some(id) = label.strip_prefix("float-") {
        ["show", "pin", "hide", "remove", "embed", "detail"].contains(&action.as_str())
            && instance.as_deref().is_none_or(|given| given == id)
    } else {
        false
    };
    if !allowed {
        return Err("CAPABILITY_DENIED: 当前窗口无此展示权限".into());
    }
    if matches!(action.as_str(), "desktop-grid" | "desktop-grid-restore") {
        return tauri::async_runtime::spawn_blocking(move || {
            desktop_grid::launch(&app, action == "desktop-grid-restore")
        })
        .await
        .map_err(|error| error.to_string())?;
    }
    let scene = windows::panel_scene();
    if action == "outside-panel" && !windows::scene_is_foreground(scene) {
        return Ok(());
    }
    if action == "hide-all" {
        app.state::<Core>().assistant.cancel_all()?;
    }
    if action == "remove" {
        if let Some(id) = &instance {
            app.state::<Core>().assistant.cancel(id)?;
            app.state::<Core>().assistant.kick(&app, id);
        }
    }
    tauri::async_runtime::spawn_blocking(move || {
        windows::serialized(&app, |app| {
            let id = || instance.as_deref().ok_or("缺少实例".to_owned());
            match action.as_str() {
                "show" => windows::component_library(app),
                "settings" => windows::settings(app),
                "hide-all" => windows::hide_all(app),
                "restore-all" => windows::restore(app),
                "toggle" => windows::toggle(app),
                "collapse" => windows::collapse(app),
                "outside-panel" => windows::dismiss_from_outside(app, scene),
                "lock-panel" | "unlock-panel" => {
                    windows::set_panel_locked(app, action == "lock-panel")
                }
                "enable-widget-backdrop" | "disable-widget-backdrop" => {
                    app.state::<Core>().store.save_preference(
                        "widget_backdrop",
                        if action == "enable-widget-backdrop" {
                            "true"
                        } else {
                            "false"
                        },
                    )?;
                    windows::changed(app)
                }
                "set-app-theme" => {
                    let theme = theme.ok_or("INVALID_ARGUMENT: 缺少应用主题")?;
                    let core = app.state::<Core>();
                    core.store.save_preference(AppTheme::KEY, theme.id())?;
                    if let Err(error) = window.set_theme(Some(theme.chrome())) {
                        core.log_error(&format!("应用标题栏切换主题失败：{error}"));
                    }
                    if let Err(error) = windows::changed(app) {
                        core.log_error(&format!("应用主题同步事件失败：{error}"));
                    }
                    Ok(())
                }
                "resize-panel" => windows::resize_panel_width(app),
                "pin" => windows::pin(app, id()?),
                "hide" => windows::hide_instance(app, id()?),
                "remove" => windows::remove_instance(app, id()?),
                "restore" => windows::restore_instance(app, id()?),
                "float" => windows::move_to(app, id()?, "desktop"),
                "embed" => windows::move_to(app, id()?, "panel"),
                "detail" => windows::detail(app, id()?),
                "close-detail" => windows::destroy_window(app, window.label()),
                "edge-hover" | "edge-activate" => {
                    let opened = if action == "edge-hover" {
                        windows::show_panel_on_hover(app)?
                    } else {
                        let foreground = windows::scene_is_foreground(scene);
                        let source = if foreground { "panel" } else { "edge" };
                        if scene != windows::panel_scene()
                            || window.label() != source
                            || windows::hidden(app)?
                            || !window.is_visible().map_err(err)?
                        {
                            return Ok(());
                        }
                        windows::toggle(app)?;
                        !foreground
                    };
                    if opened {
                        app.state::<Core>()
                            .edge_hits
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(())
                }
                "destroy" => windows::destroy_views(app),
                "repair" => windows::repair(app),
                _ => Err("未知窗口操作".into()),
            }
        })
    })
    .await
    .map_err(err)?
}
#[tauri::command]
fn panel_view(
    window: tauri::WebviewWindow,
    core: State<'_, Core>,
    view_id: String,
    update: windows::PanelViewUpdate,
) -> AppResult<()> {
    if window.label() != "panel" {
        return Err("CAPABILITY_DENIED: 仅面板可以更新视口".into());
    }
    // 建窗线程等待宿主页时不占用此命令所需的锁。
    core.visibility
        .lock()
        .map_err(err)?
        .update_panel_view(&view_id, update)
}
#[tauri::command]
async fn drag_panel_edge(app: AppHandle, window: tauri::WebviewWindow) -> AppResult<bool> {
    if !matches!(window.label(), "panel" | "edge") {
        return Err("CAPABILITY_DENIED: 当前窗口无挂件拖动权限".into());
    }
    // 展开动作仍在建窗或滑入时，先记录按住挂件的实际鼠标位置。
    let Some(press) = desktop_native::panel_edge_press()? else {
        return Ok(false);
    };
    tauri::async_runtime::spawn_blocking(move || {
        windows::serialized(&app, |app| windows::drag_panel_edge(app, &window, press))
    })
    .await
    .map_err(err)?
}

#[tauri::command]
async fn set_panel(
    app: AppHandle,
    ratio: Option<f64>,
    tint: Option<f64>,
    slide_in_ms: Option<u32>,
    slide_out_ms: Option<u32>,
    desktop_double_click: Option<bool>,
    edge_style: Option<panel::EdgeStyle>,
) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        windows::serialized(&app, |app| {
            let mut settings: panel::Preferences =
                serde_json::from_value(windows::panel_preferences(app)?).map_err(err)?;
            if let Some(value) = ratio {
                settings.ratio = value;
            }
            if let Some(value) = tint {
                settings.tint = value;
            }
            if let Some(value) = slide_in_ms {
                settings.slide_in_ms = value;
            }
            if let Some(value) = slide_out_ms {
                settings.slide_out_ms = value;
            }
            if let Some(value) = desktop_double_click {
                settings.desktop_double_click = value;
            }
            if let Some(value) = edge_style {
                settings.edge_style = value;
            }
            windows::save_panel_settings(app, settings)
        })
    })
    .await
    .map_err(err)?
}
#[tauri::command]
async fn manage_panel_group(
    app: AppHandle,
    window: tauri::WebviewWindow,
    change: panel::GroupChange,
) -> AppResult<()> {
    if !["main", "panel"].contains(&window.label()) {
        return Err("CAPABILITY_DENIED: 仅管理与面板可修改分组".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        windows::serialized(&app, |app| windows::change_panel_groups(app, change))
    })
    .await
    .map_err(err)?
}

#[tauri::command]
async fn save_panel_layout(
    app: AppHandle,
    window: tauri::WebviewWindow,
    instance: String,
    layout: panel::Layout,
) -> AppResult<()> {
    if !["main", "panel"].contains(&window.label()) {
        return Err("CAPABILITY_DENIED: 仅管理与面板可调整面板布局".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        windows::serialized(&app, |app| {
            app.state::<Core>()
                .store
                .save_panel_layout(&instance, layout)?;
            windows::changed(app)
        })
    })
    .await
    .map_err(err)?
}

#[tauri::command]
async fn app_update(
    app: AppHandle,
    action: String,
    enabled: Option<bool>,
    version: Option<String>,
) -> AppResult<Value> {
    updates::command(app, &action, enabled, version.as_deref()).await
}
#[tauri::command]
async fn surface_activate(window: tauri::WebviewWindow) -> AppResult<Value> {
    if !window.label().starts_with("float-") {
        return Ok(Value::Null);
    }
    #[cfg(windows)]
    {
        return tauri::async_runtime::spawn_blocking(move || {
            desktop_native::activate_clicked(&window)
        })
        .await
        .map_err(err)?;
    }
    #[cfg(not(windows))]
    {
        Err("当前平台不支持桌面输入适配".into())
    }
}
#[tauri::command]
async fn surface_pointer(window: tauri::WebviewWindow, resize: bool) -> AppResult<()> {
    let id = window
        .label()
        .strip_prefix("float-")
        .ok_or("CAPABILITY_DENIED: 仅桌面组件允许拖动")?
        .to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        windows::serialized(window.app_handle(), |app| {
            let (_, _, mw, mh) = windows::dimensions(app, &id)?;
            #[cfg(windows)]
            desktop_native::pointer_move(&window, resize, mw, mh)?;
            windows::save_geometry(&window)
        })
    })
    .await
    .map_err(err)?
}
#[tauri::command]
fn surface_failed(
    window: tauri::WebviewWindow,
    app: AppHandle,
    instance: String,
    message: String,
) -> AppResult<()> {
    let label = window.label();
    if pomodoro_window::role(label, &instance) {
        return app
            .state::<Core>()
            .pomodoro_windows
            .failed(&app, label, &instance);
    }
    if label != "panel"
        && label != format!("float-{instance}")
        && label != format!("detail-{instance}")
        && label != format!("keyboard-{instance}")
        && reminder_host::surface_role(label, &instance).is_none()
        && clipboard_surface::role(label, &instance).is_none()
        && clipboard_tools::role(label, &instance).is_none()
    {
        return Err("CAPABILITY_DENIED: 无效展示来源".into());
    }
    windows::failed(&app, label, &instance, message.chars().take(1000).collect())
}

fn panel_shortcut_available(core: &Core) -> bool {
    core.integration
        .as_ref()
        .is_none_or(|config| config.panel_shortcut_enabled())
}

fn panel_shortcut_enabled(store: &Store) -> bool {
    store.preference("panel_shortcut_enabled").as_deref() != Some("false")
}

pub(crate) fn panel_shortcut(store: &Store) -> String {
    store
        .preference("shortcut")
        .unwrap_or_else(|| "Shift+Space".into())
}

fn register_panel_shortcut(app: &AppHandle, value: &str) -> AppResult<()> {
    let shortcuts = app.global_shortcut();
    if shortcuts.is_registered(value) {
        return Err(format!("快捷键 {value} 已被 SoulDesk 的其他功能占用"));
    }
    shortcuts.register(value).map_err(|error| {
        app.state::<Core>()
            .log_error(&format!("面板快捷键 {value} 注册失败：{error}"));
        if matches!(&error, tauri_plugin_global_shortcut::Error::GlobalHotkey(reason)
            if reason.starts_with("HotKey already registered:"))
        {
            format!("快捷键 {value} 已被占用，请更换键位")
        } else {
            format!("无法注册快捷键 {value}，请检查键位后重试")
        }
    })
}

#[tauri::command]
fn set_shortcut(app: AppHandle, value: Option<String>, enabled: Option<bool>) -> AppResult<()> {
    let core = app.state::<Core>();
    let old = panel_shortcut(&core.store);
    let available = panel_shortcut_available(&core);
    let was_enabled = panel_shortcut_enabled(&core.store);
    let registered = core.panel_shortcut_registered.load(Ordering::Relaxed);
    let shortcut = app.global_shortcut();
    if let Some(enabled) = enabled {
        if available && enabled && !registered {
            register_panel_shortcut(&app, &old)?;
        } else if !enabled && registered {
            shortcut.unregister(old.as_str()).map_err(err)?;
        }
        if enabled == was_enabled {
            core.panel_shortcut_registered
                .store(available && enabled, Ordering::Relaxed);
            return Ok(());
        }
        if let Err(error) = core.store.save_preference(
            "panel_shortcut_enabled",
            if enabled { "true" } else { "false" },
        ) {
            if available && enabled && !registered {
                let _ = shortcut.unregister(old.as_str());
            } else if !enabled && registered && register_panel_shortcut(&app, &old).is_err() {
                core.panel_shortcut_registered
                    .store(false, Ordering::Relaxed);
            }
            return Err(error);
        }
        core.panel_shortcut_registered
            .store(available && enabled, Ordering::Relaxed);
        return Ok(());
    }
    let Some(value) = value else {
        return Err("缺少快捷键".into());
    };
    let key = value
        .parse::<tauri_plugin_global_shortcut::Shortcut>()
        .map_err(|_| "快捷键格式无效，例如 Shift+Space".to_string())?;
    let same_key = old
        .parse::<tauri_plugin_global_shortcut::Shortcut>()
        .is_ok_and(|old| old == key);
    if !same_key && key.mods.is_empty() {
        return Err("请按住 Ctrl、Alt、Shift 或 Win，再按一个主键".into());
    }
    let register = available && was_enabled && (!registered || !same_key);
    if register {
        register_panel_shortcut(&app, &value).map_err(|error| {
            if registered {
                format!("{error}。已保留原键")
            } else {
                error
            }
        })?;
    }
    if old != value {
        if let Err(error) = core.store.save_preference("shortcut", &value) {
            if register {
                let _ = shortcut.unregister(key);
            }
            return Err(error);
        }
    }
    if register {
        core.panel_shortcut_registered
            .store(true, Ordering::Relaxed);
        if registered {
            if let Err(e) = shortcut.unregister(old.as_str()) {
                core.report(format!("旧快捷键释放失败：{e}"));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn independent_sound() -> AppResult<()> {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_NODEFAULT, SND_SYNC};
    let name: Vec<u16> = "SystemNotification".encode_utf16().chain(Some(0)).collect();
    if unsafe {
        PlaySoundW(
            name.as_ptr(),
            std::ptr::null_mut(),
            SND_ALIAS | SND_SYNC | SND_NODEFAULT,
        )
    } == 0
    {
        Err("系统声音调用失败".into())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn independent_sound() -> AppResult<()> {
    Err("当前平台未实现声音适配器".into())
}

#[cfg(windows)]
fn deliver_notification(app: &AppHandle) -> AppResult<&'static str> {
    if !reminder_store::Channel::Windows.enabled_in(&app.state::<Core>().store)? {
        return Ok("suppressed");
    }
    // 复用 Tauri 维护的同一 WinRT 实现，等待系统调用返回，避免插件吞掉发送错误。
    tauri_winrt_notification::Toast::new(&app.config().identifier)
        .title("SoulDesk · 后台测试")
        .text1("核心后台仍在执行；此事件与组件页面无关。")
        .sound(None)
        .show()
        .map_err(err)?;
    Ok("sent")
}

#[cfg(not(windows))]
fn deliver_notification(app: &AppHandle) -> AppResult<&'static str> {
    if !reminder_store::Channel::Windows.enabled_in(&app.state::<Core>().store)? {
        return Ok("suppressed");
    }
    app.notification()
        .builder()
        .title("SoulDesk · 后台测试")
        .body("核心后台仍在执行。")
        .show()
        .map_err(err)?;
    Ok("unknown: 已交给通知插件，系统结果未返回")
}

#[tauri::command]
async fn schedule_test(
    app: AppHandle,
    notification: bool,
    sound: bool,
    delay: u64,
) -> AppResult<String> {
    if !(1..=60).contains(&delay) {
        return Err("延时范围为 1～60 秒".into());
    }
    let core = app.state::<Core>();
    let id = Uuid::new_v4().to_string();
    core.store
        .db
        .lock()
        .map_err(err)?
        .execute(
            "INSERT INTO test_events(id,created,notification,sound) VALUES(?1,?2,?3,?4)",
            rusqlite::params![
                id,
                now_ms(),
                if notification {
                    "pending"
                } else {
                    "suppressed"
                },
                if sound { "pending" } else { "suppressed" }
            ],
        )
        .map_err(err)?;
    core.active_tasks.fetch_add(1, Ordering::Relaxed);
    let task_id = id.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(delay)).await;
        let core = app.state::<Core>();
        let send = async {
            core.store
                .db
                .lock()
                .map_err(err)?
                .execute(
                    "UPDATE test_events SET notification=?1,sound=?2 WHERE id=?3",
                    rusqlite::params![
                        if notification {
                            "sending"
                        } else {
                            "suppressed"
                        },
                        if sound { "sending" } else { "suppressed" },
                        task_id
                    ],
                )
                .map_err(err)?;
            let n = if notification {
                let notification_app = app.clone();
                match tauri::async_runtime::spawn_blocking(move || {
                    deliver_notification(&notification_app)
                })
                .await
                .map_err(err)?
                {
                    Ok(state) => state.to_owned(),
                    Err(e) => format!("failed: {e}"),
                }
            } else {
                "suppressed".into()
            };
            let s = if sound {
                match tauri::async_runtime::spawn_blocking(independent_sound)
                    .await
                    .map_err(err)?
                {
                    Ok(_) => "sent".to_owned(),
                    Err(e) => format!("failed: {e}"),
                }
            } else {
                "suppressed".into()
            };
            core.store.db.lock().map_err(err)?.execute("UPDATE test_events SET completed=?1,notification=?2,sound=?3,view_count=?4 WHERE id=?5",rusqlite::params![now_ms(),n,s,app.webview_windows().len() as u64,task_id]).map_err(err)?;
            Ok(())
        };
        if let Err(e) = send.await {
            core.report(e);
        }
        core.active_tasks.fetch_sub(1, Ordering::Relaxed);
        let _ = app.emit("host:changed", ());
    });
    Ok(id)
}

fn tray_panel_should_toggle(event: &TrayIconEvent, skip_release: &AtomicBool) -> bool {
    match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Down,
            ..
        } => {
            // 双击的尾部释放可能落在托盘外，下一次按下重新开始。
            skip_release.store(false, Ordering::Relaxed);
            false
        }
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => {
            // Windows 在第二次按下时送达双击，第一次释放已完成切换。
            skip_release.store(true, Ordering::Relaxed);
            false
        }
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } => !skip_release.swap(false, Ordering::Relaxed),
        _ => false,
    }
}

type FeedChannels = (
    tokio::sync::watch::Receiver<market::Desired>,
    tokio::sync::mpsc::Sender<market::Input>,
    tokio::sync::mpsc::Receiver<market::Input>,
);

fn manage_core(
    app: &mut tauri::App,
    store: Store,
    integration: Option<integration::Config>,
    verification: Option<verification::Config>,
    market_role: market_runtime::Role,
) -> [FeedChannels; 2] {
    let config = store
        .preference("network")
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or(json!({"mode":"system"}));
    let network = business::resolve_network(&config);
    let initial_error = network.as_ref().err().cloned();
    let network_ready = network.is_ok();
    let network = network.unwrap_or(network::NetworkConfig::Direct);
    let (feed, market_rx, market_tx, market_inputs) = market::Feed::new(network.clone());
    let (spot, spot_rx, spot_tx, spot_inputs) = market::Feed::for_market(network, "spot");
    let hidden = store.preference("hidden_all").as_deref() == Some("true");
    let panel_locked = store.preference("panel_locked").as_deref() == Some("true");
    app.manage(Core {
        ai: Default::default(),
        assistant: Default::default(),
        jobs: Default::default(),
        clipboard: Default::default(),
        business: business::Service {
            feed,
            spot,
            charts: Default::default(),
            subscriptions: Default::default(),
            network_ready: AtomicBool::new(network_ready),
        },
        market_role,
        store: Arc::new(store),
        sessions: Default::default(),
        visibility: Mutex::new(windows::Visibility {
            hidden,
            panel_locked,
            ..Default::default()
        }),
        window_actions: Mutex::new(()),
        quitting: AtomicBool::new(false),
        ticks: AtomicU64::new(0),
        active_tasks: AtomicU64::new(0),
        last_error: Arc::new(Mutex::new(initial_error)),
        verification,
        panel_shortcut_registered: AtomicBool::new(false),
        shortcut_hits: AtomicU64::new(0),
        shortcut_trace: Mutex::new(None),
        second_launches: AtomicU64::new(0),
        edge_hits: AtomicU64::new(0),
        host_token: Uuid::new_v4().to_string(),
        view_reports: Mutex::new(Default::default()),
        resource_reports: Mutex::new(Vec::new()),
        installing: AtomicBool::new(false),
        trials: Mutex::new(Default::default()),
        geometry_versions: Mutex::new(Default::default()),
        window_tasks: AtomicU64::new(0),
        integration,
        transfer_failure: Mutex::new(None),
        updates: Default::default(),
        coin_live: Default::default(),
        coin_index: Default::default(),
        content_cache: Default::default(),
        website_verification: Default::default(),
        hardware: Default::default(),
        keyboard: Default::default(),
        reminders: Default::default(),
        water: Default::default(),
        labor: Default::default(),
        todo: Default::default(),
        pomodoro: Default::default(),
        pomodoro_windows: Default::default(),
    });
    [
        (market_rx, market_tx, market_inputs),
        (spot_rx, spot_tx, spot_inputs),
    ]
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::{
        menu::{Menu, MenuItem},
        tray::TrayIconBuilder,
    };
    let args = std::env::args().collect::<Vec<_>>();
    let integration = integration::parse(&args)?;
    if let Some(config) = &integration {
        integration::prepare(config)?;
    }
    let verification = verification::parse(&args)?;
    let root = if let Some(config) = &integration {
        config.root.clone()
    } else {
        match &verification {
            Some(config) => config.root.clone(),
            None => app.path().app_local_data_dir()?,
        }
    };
    let split_market = verification.is_none()
        && integration
            .as_ref()
            .is_none_or(|config| matches!(config.mode.as_str(), "manual" | "market-isolation"));
    let store = Store::open_desktop(root, split_market)?;
    reminder_store::migrate_notification_channel(&store)?;
    let shortcut = panel_shortcut(&store);
    let hidden = store.preference("hidden_all").as_deref() == Some("true");
    let market_role = if split_market {
        market_runtime::Role::Desktop
    } else {
        market_runtime::Role::InProcess
    };
    let channels = manage_core(app, store, integration, verification, market_role);
    if split_market {
        market_runtime::start_desktop(app.handle())?;
    }
    let show = MenuItem::with_id(app, "show", "添加组件", true, None::<&str>)?;
    let restore = MenuItem::with_id(app, "restore", "恢复组件", true, None::<&str>)?;
    let panel = MenuItem::with_id(app, "panel", "切换组件面板", true, None::<&str>)?;
    let assistant = MenuItem::with_id(app, "assistant", "网页 AI 助手", true, None::<&str>)?;
    let island_visibility = MenuItem::with_id(
        app,
        "island-visibility",
        "切换灵动通知显隐",
        true,
        None::<&str>,
    )?;
    let island_mute =
        MenuItem::with_id(app, "island-mute", "切换灵动通知静音", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "隐藏全部", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let desktop_grid =
        MenuItem::with_id(app, "desktop-grid", "桌面格子", cfg!(windows), None::<&str>)?;
    let desktop_grid_restore = MenuItem::with_id(
        app,
        "desktop-grid-restore",
        "恢复 Windows 桌面图标",
        cfg!(windows),
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "退出 SoulDesk", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &settings,
            &show,
            &panel,
            &assistant,
            &desktop_grid,
            &desktop_grid_restore,
            &island_visibility,
            &island_mute,
            &restore,
            &hide,
            &quit,
        ],
    )?;
    let tray_pixels: &[u8; 32 * 32 * 4] = include_bytes!("../icons/tray.rgba");
    let skip_tray_release = AtomicBool::new(false);
    TrayIconBuilder::with_id(MAIN_TRAY)
        .icon(tauri::image::Image::new(tray_pixels, 32, 32))
        .tooltip("SoulDesk")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(move |tray, event| {
            if tray_panel_should_toggle(&event, &skip_tray_release) {
                windows::dispatch(tray.app_handle(), windows::toggle);
            }
        })
        .on_menu_event(|app, event| {
            let action = event.id.as_ref().to_owned();
            if app
                .state::<Core>()
                .pomodoro_windows
                .menu_event(app, &action)
            {
                return;
            }
            let island_control = match action.as_str() {
                "island-visibility" => Some(reminder_host::IslandControl::Visibility),
                "island-mute" => Some(reminder_host::IslandControl::Mute),
                _ => None,
            };
            if let Some(control) = island_control {
                control.dispatch(app, None);
                return;
            }
            if matches!(action.as_str(), "desktop-grid" | "desktop-grid-restore") {
                let app = app.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Err(error) = desktop_grid::launch(&app, action == "desktop-grid-restore")
                    {
                        app.state::<Core>().log_error(&error);
                    }
                });
                return;
            }
            if action == "quit" {
                let _ = app.state::<Core>().pomodoro_windows.service.stop();
                app.state::<Core>().jobs.stop_memory();
                let _ = app.state::<Core>().assistant.stop();
                windows::dispatch(app, windows::quit);
            } else {
                if action == "hide" {
                    let _ = app.state::<Core>().assistant.cancel_all();
                }
                windows::dispatch(app, move |app| match action.as_str() {
                    "show" => windows::component_library(app),
                    "settings" => windows::settings(app),
                    "hide" => windows::hide_all(app),
                    "restore" => windows::restore(app),
                    "panel" => windows::toggle(app),
                    "assistant" => app.state::<Core>().assistant.tray(app),
                    _ => Ok(()),
                });
            }
        })
        .build(app)?;
    let core = app.state::<Core>();
    if panel_shortcut_available(&core) && panel_shortcut_enabled(&core.store) {
        let registered = register_panel_shortcut(app.handle(), &shortcut).is_ok();
        core.panel_shortcut_registered
            .store(registered, Ordering::Relaxed);
    }
    let normal_run = core.integration.is_none() && core.verification.is_none();
    if normal_run {
        if let Err(error) = core.reminders.register_island_shortcuts(app.handle()) {
            core.log_error(&error);
        }
    }
    #[cfg(windows)]
    if normal_run
        || core
            .integration
            .as_ref()
            .is_some_and(|c| c.mode == "widgetjs-manual")
    {
        match desktop_input::start(
            app.handle(),
            normal_run
                && windows::panel_preferences(app.handle())?["desktopDoubleClick"]
                    .as_bool()
                    .unwrap_or(true),
        ) {
            Ok(listener) => {
                app.manage(listener);
            }
            Err(error) => app.state::<Core>().report(error),
        }
    }
    if normal_run {
        updates::start(app.handle());
    }
    // 正常启动恢复桌面场景；显式隔离历史验收继续由其入口拥有。
    if !core
        .integration
        .as_ref()
        .is_some_and(|c| c.mode == "grid-preview")
    {
        core.keyboard.start(app.handle())?;
        core.clipboard.start(app.handle())?;
    }
    if app.state::<Core>().verification.is_none()
        && !hidden
        && !market_runtime::is_check(app.handle())
    {
        windows::dispatch(app.handle(), windows::restore);
    }
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(1));
        let mut monitor_job: Option<tauri::async_runtime::JoinHandle<()>> = None;
        loop {
            timer.tick().await;
            if handle.state::<Core>().quitting.load(Ordering::Relaxed) {
                break;
            }
            crate::component_lifecycle::tick(&handle);
            let tick = handle.state::<Core>().ticks.fetch_add(1, Ordering::Relaxed);
            // 巡检读取的是当前状态，窗口线程阻塞时无需补跑已经错过的轮次。
            if tick % 2 == 0
                && monitor_job
                    .as_ref()
                    .is_none_or(|job| job.inner().is_finished())
            {
                monitor_job = Some(windows::dispatch(&handle, windows::monitor_changes));
            }
        }
    });
    if args.iter().any(|a| a == "--show") {
        windows::show(app.handle(), true)?;
    } else if normal_run && !windows::silent_startup(&core.store) {
        windows::dispatch(app.handle(), |app| windows::show(app, false));
    }
    alert_notice::activate_argument(app.handle(), std::env::args().skip(1));
    verification::start(app.handle());
    integration::start(app.handle());
    if app
        .state::<Core>()
        .integration
        .as_ref()
        .is_none_or(|c| !c.standalone_check() || c.market_check())
        && app
            .state::<Core>()
            .verification
            .as_ref()
            .is_none_or(|c| c.g2)
    {
        let core = app.state::<Core>();
        for (feed, (rx, tx, inputs)) in [&core.business.feed, &core.business.spot]
            .into_iter()
            .zip(channels)
        {
            business::start_feed(app.handle(), feed.clone(), rx, tx, inputs);
        }
        business::start(app.handle());
    }
    Ok(())
}

fn prevent_exit(quitting: bool, code: Option<i32>) -> bool {
    // 关闭最后一个窗口产生的隐式退出不能抢先结束事件循环；
    // 只接受本程序已授权并通过 app.exit 发出的显式退出。
    !quitting || code.is_none()
}

#[cfg(test)]
mod tray_panel_tests {
    use super::*;

    fn click(button: MouseButton, button_state: MouseButtonState) -> TrayIconEvent {
        TrayIconEvent::Click {
            id: MAIN_TRAY.into(),
            position: tauri::PhysicalPosition::new(0.0, 0.0),
            rect: tauri::Rect::default(),
            button,
            button_state,
        }
    }

    fn double_click(button: MouseButton) -> TrayIconEvent {
        TrayIconEvent::DoubleClick {
            id: MAIN_TRAY.into(),
            position: tauri::PhysicalPosition::new(0.0, 0.0),
            rect: tauri::Rect::default(),
            button,
        }
    }

    #[test]
    fn tray_panel_double_click_keeps_only_the_first_release() {
        let skip_release = AtomicBool::new(false);
        let down = click(MouseButton::Left, MouseButtonState::Down);
        let up = click(MouseButton::Left, MouseButtonState::Up);
        assert!(!tray_panel_should_toggle(&down, &skip_release));
        assert!(tray_panel_should_toggle(&up, &skip_release));
        assert!(!tray_panel_should_toggle(
            &double_click(MouseButton::Left),
            &skip_release
        ));
        assert!(!tray_panel_should_toggle(&up, &skip_release));
        assert!(!tray_panel_should_toggle(&down, &skip_release));
        assert!(tray_panel_should_toggle(&up, &skip_release));
    }

    #[test]
    fn tray_panel_next_press_resets_an_unfinished_double_click() {
        let skip_release = AtomicBool::new(false);
        assert!(!tray_panel_should_toggle(
            &double_click(MouseButton::Left),
            &skip_release
        ));
        assert!(!tray_panel_should_toggle(
            &click(MouseButton::Right, MouseButtonState::Up),
            &skip_release
        ));
        assert!(!tray_panel_should_toggle(
            &click(MouseButton::Left, MouseButtonState::Down),
            &skip_release
        ));
        assert!(!tray_panel_should_toggle(
            &double_click(MouseButton::Right),
            &skip_release
        ));
        assert!(tray_panel_should_toggle(
            &click(MouseButton::Left, MouseButtonState::Up),
            &skip_release
        ));
    }
}

#[cfg(test)]
mod exit_tests {
    use super::prevent_exit;

    #[test]
    fn closing_last_window_does_not_overtake_explicit_exit() {
        assert!(prevent_exit(false, None));
        assert!(prevent_exit(true, None));
        assert!(prevent_exit(false, Some(0)));
        assert!(!prevent_exit(true, Some(0)));
        assert!(!prevent_exit(true, Some(1)));
    }
}

pub fn run() {
    let args = std::env::args().collect::<Vec<_>>();
    if let Some(result) = market_runtime::run_worker(&args) {
        if let Err(error) = result {
            eprintln!("SoulDesk 行情进程：{error}");
            std::process::exit(1);
        }
        return;
    }
    let integration = integration::parse(&args);
    // 体验版直接启动同一份格子引擎，且只接受带标记的测试目录。
    if cfg!(feature = "desktop-grid-preview") {
        let Ok(Some(config)) = integration.as_ref() else {
            std::process::exit(2);
        };
        if config.mode != "grid-preview" {
            std::process::exit(2);
        }
        if let Err(error) = desktop_grid::preview(&config.root) {
            let _ = std::fs::write(config.root.join("preview-startup-error.txt"), error);
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    verification::capture_start();
    let panel_check = integration.is_ok_and(|c| c.is_some_and(|c| c.standalone_check()));
    let mut builder = tauri::Builder::default();
    // 专项检查必须先通过独立路径校验，且不注册全局快捷键、桌面监听或更新定时器。
    if !panel_check {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _| {
            app.state::<Core>()
                .second_launches
                .fetch_add(1, Ordering::Relaxed);
            if args.iter().any(|a| a == "--show") {
                windows::dispatch(app, |app| windows::show(app, true));
            }
            alert_notice::activate_argument(app, args.into_iter());
        }));
    }
    builder
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        let core = app.state::<Core>();
                        if let Some(spec) = core.assistant.shortcut_route(shortcut.id()) {
                            let handle = app.clone();
                            tauri::async_runtime::spawn_blocking(move || {
                                let _ = handle
                                    .state::<Core>()
                                    .assistant
                                    .shortcut_activated(&handle, &spec);
                            });
                            return;
                        }
                        match core.clipboard.shortcut_route(&core.store, shortcut.id()) {
                            clipboard_shortcuts::Route::Ignore => return,
                            clipboard_shortcuts::Route::Clipboard(spec) => {
                                let target = clipboard_host::shortcut_target(&core.store, &spec);
                                windows::dispatch(app, move |app| {
                                    app.state::<Core>()
                                        .clipboard
                                        .shortcut_activated(app, &spec, target)
                                });
                                return;
                            }
                            clipboard_shortcuts::Route::Main => {
                                if !panel_shortcut_available(&core)
                                    || !panel_shortcut_enabled(&core.store)
                                {
                                    return;
                                }
                            }
                        }
                        windows::dispatch(app, |app| {
                            let before = app.webview_windows().iter().any(|(label, w)| {
                                label != "edge" && w.is_visible().unwrap_or(false)
                            });
                            windows::toggle_locked_panel(app)?;
                            let after = app.webview_windows().iter().any(|(label, w)| {
                                label != "edge" && w.is_visible().unwrap_or(false)
                            });
                            *app.state::<Core>().shortcut_trace.lock().map_err(err)? =
                                Some(json!({"beforeVisible":before,"afterVisible":after}));
                            app.state::<Core>()
                                .shortcut_hits
                                .fetch_add(1, Ordering::Relaxed);
                            Ok(())
                        });
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .register_uri_scheme_protocol("widget", |ctx, request| {
            let result = widgets::resource(
                &ctx.app_handle().state::<Core>().store,
                request.uri().path(),
            );
            let (status, data, mime) = match result {
                Ok((b, m)) => (200, b, m),
                Err(_) => (
                    404,
                    b"component resource unavailable".to_vec(),
                    "text/plain".into(),
                ),
            };
            let core = ctx.app_handle().state::<Core>();
            if core.verification.is_some() {
                if let Ok(mut reports) = core.resource_reports.lock() {
                    if reports.len() < 64 {
                        reports.push(
                            json!({"uri":request.uri().to_string(),"status":status,"mime":mime}),
                        );
                    }
                }
            }
            tauri::http::Response::builder()
                .status(status)
                .header("Content-Type", mime)
                .header("X-Content-Type-Options", "nosniff")
                .header(
                    "Content-Security-Policy",
                    widgets::resource_csp(&core.store, request.uri().path()),
                )
                .header("Cache-Control", "no-store")
                .body(data)
                .unwrap()
        })
        .invoke_handler(|invoke| {
            let allowed = invoke
                .message
                .webview_ref()
                .try_state::<Core>()
                .is_some_and(|core| {
                    invoke
                        .message
                        .headers()
                        .get("x-souldesk-host")
                        .and_then(|h| h.to_str().ok())
                        == Some(core.host_token.as_str())
                });
            if !allowed {
                invoke
                    .resolver
                    .reject("CAPABILITY_DENIED: 只允许宿主顶层容器调用");
                return true;
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                snapshot,
                import_package,
                official_gallery::official_catalog,
                official_gallery::install_official,
                create_instance,
                manage_instance,
                reorder_instances,
                unregister_package,
                open_session,
                close_session,
                widget_request,
                assistant_open_session,
                assistant_close_session,
                assistant_widget_request,
                assistant_surface_failed,
                job_open_session,
                job_close_session,
                job_widget_request,
                job_surface_failed,
                window_action,
                set_shortcut,
                set_autostart,
                schedule_test,
                business_settings,
                set_panel,
                drag_panel_edge,
                panel_view,
                save_panel_layout,
                manage_panel_group,
                app_update,
                surface_activate,
                surface_pointer,
                surface_failed,
                reminder_host::island_surface,
                reminder_host::notification_settings,
                verification::report_view,
                packages::trial_failed
            ];
            handler(invoke)
        })
        .manage(official_gallery::Gallery::default())
        .setup(setup)
        .on_window_event(|window, event| {
            let app = window.app_handle();
            if app.try_state::<Core>().is_some_and(|core| {
                core.pomodoro_windows
                    .window_event(app, window.label(), event)
            }) {
                return;
            }
            #[cfg(windows)]
            if app.try_state::<Core>().is_some()
                && job_surfaces::native::window_event(app, window.label(), event)
            {
                return;
            }
            if app
                .try_state::<Core>()
                .is_some_and(|core| core.assistant.window_event(app, window.label(), event))
            {
                return;
            }
            match event {
                tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                    windows::queue_geometry_save(window)
                }
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let label = window.label().to_owned();
                    app.state::<Core>().reminders.closed(&label);
                    app.state::<Core>().clipboard.closed(&label);
                    let core = app.state::<Core>();
                    let _ = core.ai.close_tool(&core.store, &label);
                    windows::dispatch(app, move |app| {
                        if let Some(id) = label.strip_prefix("float-") {
                            windows::hide_instance(app, id)
                        } else if label == "panel" {
                            windows::collapse(app)
                        } else {
                            if let Some(w) = app.get_webview_window(&label) {
                                windows::save_geometry(&w)?;
                            }
                            windows::release(app, &label)?;
                            windows::destroy_window(app, &label)
                        }
                    });
                }
                tauri::WindowEvent::Focused(focused)
                    if window.label().starts_with("clipboard-") =>
                {
                    app.state::<Core>()
                        .clipboard
                        .focus_window(window.label(), *focused);
                }
                tauri::WindowEvent::Destroyed => {
                    if let Some(core) = app.try_state::<Core>() {
                        core.reminders.closed(window.label());
                        core.clipboard.closed(window.label());
                        let _ = core.ai.close_tool(&core.store, window.label());
                        if let Ok(mut values) = core.geometry_versions.lock() {
                            values.remove(window.label());
                        }
                        if let Ok(mut sessions) = core.sessions.0.lock() {
                            sessions.retain(|_, s| s.window != window.label());
                        }
                    }
                }
                _ => {}
            }
        })
        .build(tauri::generate_context!())
        .expect("SoulDesk 初始化失败；请保留原数据并查看诊断")
        .run(|app, event| match event {
            tauri::RunEvent::ExitRequested { api, code, .. } => {
                let prevent =
                    prevent_exit(app.state::<Core>().quitting.load(Ordering::Relaxed), code);
                integration::trace_lifecycle(
                    app,
                    json!({"event":"exit-requested","code":code,"prevented":prevent}),
                );
                if prevent {
                    api.prevent_exit();
                }
            }
            tauri::RunEvent::Exit => {
                let _ = app.state::<Core>().pomodoro_windows.service.stop();
                let _ = app.state::<Core>().pomodoro_windows.cleanup(app);
                app.state::<Core>().jobs.stop_memory();
                integration::trace_lifecycle(app, json!({"event":"event-loop-exit"}))
            }
            _ => {}
        });
    if panel_check {
        eprintln!("SoulDesk 隔离事件循环已结束");
    }
}
