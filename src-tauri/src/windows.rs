use crate::{
    desktop_geometry::WorkArea,
    panel::{EdgePosition, EdgeStyle, Preferences},
    store::{err, AppResult},
    Core,
};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

#[derive(Default)]
pub struct Visibility {
    pub hidden: bool,
    pub panel_locked: bool,
    pub(crate) edge_hover_block: Option<std::time::Instant>,
    pub docked: bool,
    pub dialog: bool,
    pub monitors: String,
    pub ready: std::collections::HashMap<String, std::time::Instant>,
    pub failed: std::collections::HashMap<String, String>,
    pub(crate) panel_view: Option<PanelView>,
    pub panel_target: Option<String>,
    pub panel_scroll: std::collections::HashMap<Option<String>, PanelScroll>,
}

pub(crate) struct PanelView {
    id: String,
    ready: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PanelScroll {
    left: f64,
    top: f64,
}

#[derive(serde::Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum PanelViewUpdate {
    Ready,
    Scroll {
        group: Option<String>,
        position: PanelScroll,
    },
}

impl Visibility {
    pub fn update_panel_view(&mut self, id: &str, update: PanelViewUpdate) -> AppResult<()> {
        let Some(view) = self.panel_view.as_mut().filter(|view| view.id == id) else {
            return Ok(());
        };
        match update {
            PanelViewUpdate::Ready => view.ready = true,
            PanelViewUpdate::Scroll { group, position } => {
                if ![position.left, position.top]
                    .iter()
                    .all(|value| value.is_finite() && *value >= 0.)
                {
                    return Err("面板滚动位置无效".into());
                }
                self.panel_scroll.insert(group, position);
            }
        }
        Ok(())
    }
}

// 最低位表示面板在前台。代次隔离旧点击和旧收起计时。
static PANEL_SCENE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn panel_scene() -> u64 {
    PANEL_SCENE.load(Ordering::Acquire)
}

pub(crate) fn scene_is_foreground(scene: u64) -> bool {
    scene & 1 != 0
}

fn set_panel_foreground(foreground: bool) {
    let next = (panel_scene().wrapping_add(2) & !1) | u64::from(foreground);
    PANEL_SCENE.store(next, Ordering::Release);
}

const PANEL_DISMISS_DELAY: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Default)]
enum PanelPresence {
    #[default]
    Unseen,
    Present,
    Left(Instant),
}

impl PanelPresence {
    fn update(&mut self, present: bool, now: Instant) -> bool {
        if present {
            *self = Self::Present;
        } else if matches!(self, Self::Present) {
            *self = Self::Left(now);
        }
        matches!(self, Self::Left(since) if now.duration_since(*since) >= PANEL_DISMISS_DELAY)
    }
}

fn watch_panel_dismissal(w: &WebviewWindow, pointer_started_inside: bool) -> AppResult<()> {
    let app = w.app_handle().clone();
    let raw = native::handle(w)?;
    let scene = panel_scene();
    let mut pointer = PanelPresence::default();
    let mut focus = PanelPresence::default();
    pointer.update(pointer_started_inside, Instant::now());
    tauri::async_runtime::spawn(async move {
        while panel_scene() == scene && native::owned_alive(raw) {
            if let Ok((inside, focused)) = native::panel_presence(raw) {
                let now = Instant::now();
                let mouse_elapsed = pointer.update(inside, now);
                let focus_elapsed = focus.update(focused, now);
                if mouse_elapsed || focus_elapsed {
                    let handle = app.clone();
                    let result = tauri::async_runtime::spawn_blocking(move || {
                        serialized(&handle, |app| {
                            if panel_scene() != scene || !native::owned_alive(raw) {
                                return Ok(true);
                            }
                            let (inside, focused) = native::panel_presence(raw)?;
                            if (mouse_elapsed && !inside) || (focus_elapsed && !focused) {
                                collapse(app)?;
                                return Ok(true);
                            }
                            Ok(false)
                        })
                    })
                    .await
                    .map_err(err)
                    .and_then(|result| result);
                    match result {
                        Ok(true) => break,
                        Ok(false) => (),
                        Err(error) => {
                            app.state::<Core>().report(error);
                            break;
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    Ok(())
}

pub fn serialized<T>(
    app: &AppHandle,
    action: impl FnOnce(&AppHandle) -> AppResult<T>,
) -> AppResult<T> {
    let core = app.state::<Core>();
    let _guard = core.window_actions.lock().map_err(err)?;
    action(app)
}

// WebView2 建窗必须离开同步 IPC 和系统事件线程；同一时刻只执行一个窗口动作。
pub fn dispatch(
    app: &AppHandle,
    action: impl FnOnce(&AppHandle) -> AppResult<()> + Send + 'static,
) -> tauri::async_runtime::JoinHandle<()> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = serialized(&handle, action) {
            handle.state::<Core>().report(error);
        }
    })
}

fn html_drop_for_window(store: &crate::store::Store, label: &str) -> AppResult<bool> {
    if label != "panel" && !label.starts_with("float-") {
        return Ok(false);
    }
    let db = store.db.lock().map_err(err)?;
    let mut statement = db.prepare("SELECT p.manifest FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE ('float-'||i.id)=?1 OR (?1='panel' AND i.location='panel' AND i.visible=1)").map_err(err)?;
    let mut rows = statement.query([label]).map_err(err)?;
    while let Some(row) = rows.next().map_err(err)? {
        let manifest: crate::widgets::Manifest =
            serde_json::from_str(&row.get::<_, String>(0).map_err(err)?).map_err(err)?;
        if manifest
            .capabilities
            .iter()
            .any(|value| matches!(value.as_str(), "documents.user" | "documents.drop"))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn build(
    app: &AppHandle,
    label: &str,
    url: String,
    width: f64,
    height: f64,
) -> AppResult<WebviewWindow> {
    let keyboard = label.starts_with("keyboard-");
    let notice = label.starts_with("island-");
    let rest = label.starts_with("break-");
    let clipboard_tool = label.starts_with("clipboard-tool-");
    let clipboard = label.starts_with("clipboard-") && !clipboard_tool;
    let surface =
        label.starts_with("float-") || label == "panel" || keyboard || notice || rest || clipboard;
    let core = app.state::<Core>();
    let theme = if label == "main" {
        crate::AppTheme::stored(&core.store).chrome()
    } else {
        tauri::Theme::Dark
    };
    // 管理窗口的 ZIP 导入和支持文档拖放的组件使用 WebView2 的网页拖放。
    let html_drop = label == "main" || html_drop_for_window(&core.store, label)?;
    let create = |profile: &str| -> tauri::Result<WebviewWindow> {
        // 隔离运行连 WebView 缓存也写入本次目录，不接触正常用户数据。
        let directory = core.store.root.join("webview").join(profile);
        std::fs::create_dir_all(&directory)?;
        let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.as_str().into()))
            .title(match label {
                "main" => "SoulDesk · 组件管理",
                "panel" => "SoulDesk · 组件面板",
                _ => "SoulDesk · 组件",
            })
            .inner_size(width, height)
            .min_inner_size(120., 80.)
            .visible(false)
            .focused(false)
            .theme(Some(theme))
            .data_directory(directory)
            .initialization_script(host_script(app))
            .initialization_script_for_all_frames("if(window!==window.top)document.addEventListener('pointerdown',event=>{if(event.isTrusted&&event.button===0){const origin=new URLSearchParams(location.search).get('hostOrigin');if(origin)parent.postMessage({type:'souldesk:user-pointer',protocol:1},origin);}},true);");
        if html_drop {
            builder = builder.disable_drag_drop_handler();
        }
        #[cfg(windows)]
        if label == "main" {
            builder = builder.drag_and_drop(false);
        }
        if surface {
            builder = builder
                .decorations(false)
                .resizable(false)
                .shadow(false)
                .skip_taskbar(true)
                .transparent(true)
                .background_color(tauri::window::Color(0, 0, 0, 0));
        }
        if label == "panel" {
            builder = builder.always_on_top(true);
        }
        if notice {
            builder = builder
                .always_on_top(true)
                .focusable(false)
                .min_inner_size(120., 1.);
        }
        if rest {
            builder = builder.always_on_top(true).focusable(true);
        }
        if keyboard {
            builder = builder.always_on_top(true).focusable(false);
        }
        if clipboard {
            builder = builder
                .always_on_top(true)
                .focusable(true)
                .min_inner_size(120., 1.);
        }
        builder.build()
    };
    let shared_notice = notice && app.get_webview_window("edge").is_some();
    let result = create(if shared_notice {
        "edge"
    } else if notice {
        "notification-overlay"
    } else {
        label
    });
    #[cfg(windows)]
    let result = match result {
        Err(tauri::Error::Runtime(error))
            if shared_notice && error.to_string().contains("HRESULT(0x8007139F)") =>
        {
            create("notification-overlay")
        }
        Err(tauri::Error::Runtime(error))
            if label == "main" && error.to_string().contains("HRESULT(0x8007139F)") =>
        {
            // WebView2 拒绝复用现有环境时，组件管理可使用单独的缓存目录。
            let window = create("main-recovery").map_err(|retry| {
                format!("组件管理 WebView2 初始化失败：{error}；备用目录初始化失败：{retry}")
            })?;
            core.log_error(&format!("组件管理已通过备用 WebView2 目录打开：{error}"));
            Ok(window)
        }
        result => result,
    };
    let w = result.map_err(err)?;
    if surface {
        native::prepare(&w)?;
    }
    if label == "panel" {
        if let Err(e) = native::persistent_acrylic(&w) {
            let _ = w.destroy();
            return Err(e);
        }
    }
    if label == "main" {
        restore_geometry(&w)?;
    }
    Ok(w)
}

pub fn save_geometry(w: &WebviewWindow) -> AppResult<()> {
    if w.label() != "main" && !w.label().starts_with("float-") {
        return Ok(());
    }
    if w.is_minimized().map_err(err)? || w.is_maximized().map_err(err)? {
        return Ok(());
    }
    let scale = w.scale_factor().map_err(err)?;
    let (x, y, width, height) = if w.label().starts_with("float-") {
        let r = native::describe(w);
        let r = &r["rect"];
        (
            r["left"].as_i64().ok_or("无法读取桌面位置")? as f64,
            r["top"].as_i64().ok_or("无法读取桌面位置")? as f64,
            r["width"].as_u64().ok_or("无法读取桌面尺寸")? as f64,
            r["height"].as_u64().ok_or("无法读取桌面尺寸")? as f64,
        )
    } else {
        let p = w.outer_position().map_err(err)?;
        let sz = w.inner_size().map_err(err)?;
        (p.x as f64, p.y as f64, sz.width as f64, sz.height as f64)
    };
    w.state::<Core>().store.save_window_layout(w.label(),&serde_json::json!({"x":x/scale,"y":y/scale,"width":width/scale,"height":height/scale,"scale":scale,"pinned":if w.label().starts_with("float-"){native::describe(w)["topmost"]==true}else{false}}))
}
fn restore_geometry(w: &WebviewWindow) -> AppResult<()> {
    if let Some(g) = w.state::<Core>().store.window_layout(w.label())? {
        let num = |key: &str| {
            g[key]
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or("保存的窗口几何无效")
        };
        let (width, height, scale) = (num("width")?, num("height")?, num("scale")?);
        if !(80.0..=16384.).contains(&width)
            || !(80.0..=16384.).contains(&height)
            || !(0.5..=8.).contains(&scale)
        {
            return Err("保存的窗口尺寸无效".into());
        }
        w.set_size(LogicalSize::new(width, height)).map_err(err)?;
        w.set_position(tauri::PhysicalPosition::new(
            (num("x")? * scale) as i32,
            (num("y")? * scale) as i32,
        ))
        .map_err(err)?;
    }
    clamp(w)
}

pub fn queue_geometry_save(window: &tauri::Window) {
    if window.label() != "main" && !window.label().starts_with("float-") {
        return;
    }
    let app = window.app_handle().clone();
    let label = window.label().to_owned();
    let generation = {
        let core = app.state::<Core>();
        let Ok(mut values) = core.geometry_versions.lock() else {
            return;
        };
        let value = values.entry(label.clone()).or_insert(0);
        *value += 1;
        *value
    };
    app.state::<Core>()
        .window_tasks
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(350)).await;
        dispatch(&app, move |app| {
            let result = (|| -> AppResult<()> {
                let current = app
                    .state::<Core>()
                    .geometry_versions
                    .lock()
                    .map_err(err)?
                    .get(&label)
                    .copied();
                if current == Some(generation) {
                    if let Some(window) = app.get_webview_window(&label) {
                        save_geometry(&window)?;
                    }
                }
                Ok(())
            })();
            app.state::<Core>()
                .window_tasks
                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            result
        });
    });
}

pub fn save_all_geometry(app: &AppHandle) -> AppResult<()> {
    for window in app.webview_windows().values() {
        save_geometry(window)?;
    }
    Ok(())
}

pub(crate) fn quit(app: &AppHandle) -> AppResult<()> {
    crate::component_lifecycle::before_exit(app);
    save_all_geometry(app)?;
    crate::component_lifecycle::after_geometry_saved(app);
    crate::integration::trace_lifecycle(
        app,
        serde_json::json!({"event":"explicit-exit-start","code":0,"action":"tray-quit"}),
    );
    app.exit(0);
    crate::integration::trace_lifecycle(
        app,
        serde_json::json!({"event":"explicit-exit-enqueued","code":0}),
    );
    Ok(())
}

// 只能从后台命令/窗口协调工作线程调用；destroy() 返回时管理器可能仍持有旧句柄。
pub fn destroy_window(app: &AppHandle, label: &str) -> AppResult<()> {
    if let Some(window) = app.get_webview_window(label) {
        window.destroy().map_err(err)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while app.get_webview_window(label).is_some() {
            if std::time::Instant::now() >= deadline {
                return Err(format!("窗口 {label} 销毁确认超时，未复用旧句柄"));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    Ok(())
}

pub(crate) const SILENT_STARTUP: &str = "silent_startup";

pub(crate) fn silent_startup(store: &crate::store::Store) -> bool {
    store.preference(SILENT_STARTUP).as_deref() != Some("false")
}

#[cfg(windows)]
pub fn autostart_enabled() -> AppResult<bool> {
    use winreg::{
        enums::{HKEY_CURRENT_USER, KEY_READ},
        RegKey,
    };
    let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
        KEY_READ,
    ) {
        Ok(key) => key,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(err(e)),
    };
    match key.get_value::<String, _>("SoulDesk") {
        Ok(value) => Ok(value == autostart_command()?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(err(e)),
    }
}
#[cfg(windows)]
fn autostart_command() -> AppResult<String> {
    Ok(format!(
        "\"{}\"",
        std::env::current_exe()
            .map_err(err)?
            .to_str()
            .ok_or("程序路径编码无效")?
    ))
}
#[cfg(windows)]
pub fn set_autostart(enabled: bool) -> AppResult<()> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(err)?;
    if enabled {
        key.set_value("SoulDesk", &autostart_command()?)
            .map_err(err)
    } else {
        match key.delete_value("SoulDesk") {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(err(e)),
        }
    }
}
#[cfg(not(windows))]
pub fn autostart_enabled() -> AppResult<bool> {
    Err("当前平台未实现登录启动".into())
}
#[cfg(not(windows))]
pub fn set_autostart(_: bool) -> AppResult<()> {
    Err("当前平台未实现登录启动".into())
}

pub(crate) fn host_script(app: &AppHandle) -> String {
    let core = app.state::<Core>();
    let mut origins = vec![
        "http://tauri.localhost".to_owned(),
        "https://tauri.localhost".to_owned(),
        "tauri://localhost".to_owned(),
    ];
    if let Some(url) = &app.config().build.dev_url {
        origins.push(url.origin().ascii_serialization());
    }
    let theme = crate::AppTheme::stored(&core.store).id();
    format!("if(window.top===window&&{}.includes(location.origin)){{Object.defineProperties(window,{{__SOULDESK_HOST_TOKEN__:{{value:{}}},__SOULDESK_APP_THEME__:{{value:{theme:?}}}}});}}",serde_json::to_string(&origins).unwrap(),serde_json::to_string(&core.host_token).unwrap())
}
pub fn main_window(app: &AppHandle) -> AppResult<WebviewWindow> {
    if let Some(w) = app.get_webview_window("main") {
        return Ok(w);
    }
    let w = build(app, "main", "index.html".into(), 1000., 740.)?;
    clamp(&w)?;
    Ok(w)
}
/// 打开管理不解除隐藏全部，也不显示面板或独立组件。
pub fn show(app: &AppHandle, focus: bool) -> AppResult<()> {
    let w = main_window(app)?;
    clamp(&w)?;
    w.show().map_err(err)?;
    if focus {
        w.set_focus().map_err(err)?;
    }
    Ok(())
}
pub fn settings(app: &AppHandle) -> AppResult<()> {
    if app.get_webview_window("main").is_none() {
        build(app, "main", "index.html?tab=checks".into(), 1000., 740.)?;
    }
    show(app, true)?;
    app.emit_to("main", "host:navigate", "checks").map_err(err)
}

pub(crate) fn notification_history(app: &AppHandle, id: &str) -> AppResult<()> {
    if app.get_webview_window("main").is_none() {
        build(
            app,
            "main",
            format!("index.html?tab=checks&notificationHistory={id}"),
            1000.,
            740.,
        )?;
    }
    let window = main_window(app)?;
    if window.is_minimized().map_err(err)? {
        window.unminimize().map_err(err)?;
    }
    show(app, true)?;
    app.emit_to("main", "host:notification-history", id)
        .map_err(err)
}

pub fn instance(app: &AppHandle, id: &str) -> AppResult<serde_json::Value> {
    app.state::<Core>().store.snapshot()?["instances"]
        .as_array()
        .ok_or("实例列表无效")?
        .iter()
        .find(|i| i["id"] == id)
        .cloned()
        .ok_or("实例不存在".into())
}
fn instances(app: &AppHandle) -> AppResult<Vec<serde_json::Value>> {
    Ok(app.state::<Core>().store.snapshot()?["instances"]
        .as_array()
        .ok_or("实例列表无效")?
        .clone())
}
fn active_panel_instances(app: &AppHandle) -> AppResult<Vec<serde_json::Value>> {
    let groups = app.state::<Core>().store.panel_groups()?;
    Ok(instances(app)?
        .into_iter()
        .filter(|i| {
            i["visible"] == true
                && i["location"] == "panel"
                && i["id"].as_str().is_some_and(|id| groups.contains(id))
        })
        .collect())
}
pub(crate) fn hidden(app: &AppHandle) -> AppResult<bool> {
    Ok(app.state::<Core>().visibility.lock().map_err(err)?.hidden)
}
pub fn changed(app: &AppHandle) -> AppResult<()> {
    app.emit("host:changed", ()).map_err(err)
}

pub fn release(app: &AppHandle, label: &str) -> AppResult<()> {
    release_surface(app, label, None)
}
fn take_surface_sessions(
    sessions: &mut std::collections::HashMap<String, crate::widgets::Session>,
    label: &str,
    instance: Option<&str>,
) -> std::collections::HashSet<String> {
    let removed = sessions
        .iter()
        .filter(|(_, session)| {
            session.window == label && instance.is_none_or(|id| session.instance == id)
        })
        .map(|(token, _)| token.clone())
        .collect::<std::collections::HashSet<_>>();
    sessions.retain(|token, _| !removed.contains(token));
    removed
}
fn release_surface(app: &AppHandle, label: &str, instance: Option<&str>) -> AppResult<()> {
    let core = app.state::<Core>();
    let mut sessions = core.sessions.0.lock().map_err(err)?;
    let removed = take_surface_sessions(&mut sessions, label, instance);
    core.coin_live.release_owners(&removed)?;
    core.business
        .subscriptions
        .lock()
        .map_err(err)?
        .retain(|_, s| !removed.contains(&s.owner));
    let mut state = core.visibility.lock().map_err(err)?;
    let key = instance.map(|id| format!("{label}/{id}"));
    let keep = |candidate: &String| {
        key.as_ref().map_or_else(
            || !candidate.starts_with(&format!("{label}/")),
            |key| candidate != key,
        )
    };
    state.ready.retain(|key, _| keep(key));
    state.failed.retain(|key, _| keep(key));
    if instance.is_none() && label == "panel" {
        state.panel_view = None;
    }
    Ok(())
}
fn dispose(app: &AppHandle, label: &str) -> AppResult<()> {
    release(app, label)?;
    destroy_window(app, label)
}
pub(crate) fn keyboard_close(app: &AppHandle, id: &str) -> AppResult<()> {
    dispose(app, &format!("keyboard-{id}"))
}
pub(crate) fn clipboard_close(app: &AppHandle, label: &str) -> AppResult<()> {
    if !label.starts_with("clipboard-") {
        return Err("CAPABILITY_DENIED: 非剪切板窗口".into());
    }
    app.state::<Core>().clipboard.closed(label);
    let core = app.state::<Core>();
    core.ai.close_tool(&core.store, label)?;
    dispose(app, label)
}
pub(crate) fn clipboard_tool_show(
    app: &AppHandle,
    p: &crate::clipboard_tools::Tool,
) -> AppResult<()> {
    let core = app.state::<Core>();
    if !core.ai.current_tool(&core.store, &p.label, &p.instance) {
        return Err("PRESENTATION_EXPIRED: 工具已撤销".into());
    }
    let result = (|| {
        let (width, height, page) = match p.kind {
            crate::clipboard_tools::Kind::Settings => (560., 560., "clipboard-settings"),
            crate::clipboard_tools::Kind::Mindmap => (820., 680., "clipboard-ai-mindmap"),
            crate::clipboard_tools::Kind::Translate => (720., 560., "clipboard-ai-translate"),
        };
        let w = build(
            app,
            &p.label,
            format!("index.html?clipboardPopup={}&page={page}", p.instance),
            width,
            height,
        )?;
        w.center().map_err(err)?;
        wait_ready(app, &p.label, &p.instance)?;
        if !core.ai.current_tool(&core.store, &p.label, &p.instance) {
            return Err("PRESENTATION_EXPIRED: 工具已撤销".into());
        }
        native::show(&w)?;
        w.set_focus().map_err(err)
    })();
    if result.is_err() {
        let _ = clipboard_close(app, &p.label);
    }
    result
}
pub(crate) fn clipboard_show(
    app: &AppHandle,
    p: &crate::clipboard_surface::Popup,
) -> AppResult<()> {
    use crate::clipboard_surface::Kind;
    let core = app.state::<Core>();
    if !core.clipboard.current_window(&p.label, &p.instance) || hidden(app)? {
        return Err("PRESENTATION_EXPIRED: 剪切板窗口已撤销".into());
    }
    if app.get_webview_window(&p.label).is_some() {
        return Ok(());
    }
    let result = (|| {
        let (width, height, page) = if p.kind == Kind::History {
            (500., 400., "clipboard-history")
        } else {
            (700., 54., "clipboard-search")
        };
        let w = build(
            app,
            &p.label,
            format!("index.html?clipboardPopup={}&page={page}", p.instance),
            width,
            height,
        )?;
        #[cfg(windows)]
        let owner = w.hwnd().map_err(err)?.0 as usize;
        #[cfg(not(windows))]
        let owner = 0;
        core.clipboard.attach_window(&core.store, p, owner)?;
        let a = area(app, true)?;
        let scale = a.scale_factor;
        let width = (width * scale).round().min(a.width as f64).max(1.) as u32;
        let height = (height * scale).round().min(a.height as f64).max(1.) as u32;
        let x = a.left + (a.width as i32 - width as i32) / 2;
        let y = if p.kind == Kind::History {
            a.top
                + (a.height as i32 - height as i32 - (a.height as f64 * 0.05).round() as i32).max(0)
        } else {
            a.top
        };
        w.set_position(tauri::PhysicalPosition::new(x, y))
            .map_err(err)?;
        w.set_size(tauri::PhysicalSize::new(width, height))
            .map_err(err)?;
        wait_ready(app, &p.label, &p.instance)?;
        core.clipboard.show_window(&core.store, p, || {
            native::show(&w)?;
            if p.kind == Kind::History {
                w.set_focus().map_err(err)?;
            }
            Ok(())
        })
    })();
    if result.is_err() {
        core.clipboard.closed(&p.label);
        let _ = dispose(app, &p.label);
    }
    result
}
pub(crate) fn reminder_reconcile(
    app: &AppHandle,
    surface: crate::reminder_engine::Surface,
    target: Option<crate::reminder_engine::Target>,
) -> AppResult<()> {
    use crate::reminder_engine::Surface;
    let core = app.state::<Core>();
    if core.reminders.target(surface) != target {
        return Ok(());
    }
    if surface == Surface::Notice {
        return island_reconcile(app, target);
    }
    // 最新代次清理同类旧窗，过期清理任务即使被跳过也不会留下旧显示层。
    for label in app
        .webview_windows()
        .keys()
        .filter(|label| label.starts_with("break-"))
        .cloned()
        .collect::<Vec<_>>()
    {
        dispose(app, &label)?;
    }
    let Some(target) = target else {
        return Ok(());
    };
    if core.reminders.target(surface).as_ref() != Some(&target) || hidden(app)? {
        return Ok(());
    }
    let label = target.rest_label();
    let result = (|| {
        let w = build(
            app,
            &label,
            format!("index.html?breakOverlay={}", target.owner),
            350.,
            240.,
        )?;
        let area = area(app, false)?;
        w.set_position(tauri::PhysicalPosition::new(area.left, area.top))
            .map_err(err)?;
        w.set_fullscreen(true).map_err(err)?;
        wait_ready(app, &label, &target.owner)?;
        core.reminders.show_target(app, &target, &label, || {
            w.show().map_err(err)?;
            w.set_focus().map_err(err)
        })
    })();
    if let Err(error) = result {
        if core.reminders.target(surface).as_ref() == Some(&target) {
            core.reminders.failed(app, &target, error.clone());
        }
        let _ = dispose(app, &label);
        if error.starts_with("SESSION_EXPIRED:") {
            return Ok(());
        }
        return Err(error);
    }
    Ok(())
}

const ISLAND_WIDTH: u32 = 350;
const ISLAND_POSITION: &str = "notification.island.position";

#[derive(serde::Serialize, serde::Deserialize)]
struct IslandPosition {
    monitor: Option<String>,
    monitor_origin: (i32, i32),
    left: f64,
    top: f64,
}

fn island_geometry(
    area: WorkArea,
    height: u32,
    offset: Option<(f64, f64)>,
) -> (i32, i32, u32, u32) {
    let scaled = |size: u32| (f64::from(size) * area.scale_factor).round().max(1.) as u32;
    let width = scaled(ISLAND_WIDTH).min(area.width);
    let height = scaled(height).min(area.height);
    let max_x = f64::from(area.width - width);
    let max_y = f64::from(area.height - height);
    let (x, y) = offset
        .map(|(x, y)| (x * area.scale_factor, y * area.scale_factor))
        .unwrap_or((max_x / 2., 12. * area.scale_factor));
    (
        area.left + x.clamp(0., max_x).round() as i32,
        area.top + y.clamp(0., max_y).round() as i32,
        width,
        height,
    )
}

fn saved_island_geometry(app: &AppHandle, height: u32) -> AppResult<(i32, i32, u32, u32)> {
    let core = app.state::<Core>();
    if let Some(position) = core
        .store
        .preference(ISLAND_POSITION)
        .and_then(|value| serde_json::from_str::<IslandPosition>(&value).ok())
    {
        if let Some(monitor) = app
            .available_monitors()
            .map_err(err)?
            .into_iter()
            .find(|monitor| {
                if let Some(name) = &position.monitor {
                    monitor.name() == Some(name)
                } else {
                    (monitor.position().x, monitor.position().y) == position.monitor_origin
                }
            })
        {
            return Ok(island_geometry(
                work_area(&monitor),
                height,
                Some((position.left, position.top)),
            ));
        }
    }
    Ok(island_geometry(area(app, false)?, height, None))
}

pub(crate) fn drag_island(
    app: &AppHandle,
    window: &WebviewWindow,
    press: native::EdgePress,
) -> AppResult<bool> {
    use crate::reminder_engine::Surface;
    let core = app.state::<Core>();
    if hidden(app)? {
        return Ok(true);
    }
    let Some(target) = core.reminders.target(Surface::Notice) else {
        return Ok(true);
    };
    let initial = window.outer_position().map_err(err)?;
    let origin = window
        .current_monitor()
        .map_err(err)?
        .or(app.primary_monitor().map_err(err)?)
        .ok_or("未找到可用显示器")?;
    let mut monitor = origin.clone();
    let initial_scale = origin.scale_factor();
    let mut position = None;
    let attempt = native::drag_panel_edge(window, press, initial_scale, |start, pointer| {
        if let Some(next) = app
            .monitor_from_point(f64::from(pointer.0), f64::from(pointer.1))
            .map_err(err)?
        {
            monitor = next;
        }
        let area = work_area(&monitor);
        let attach_x = f64::from(start.0 - initial.x) / initial_scale;
        let attach_y = f64::from(start.1 - initial.y) / initial_scale;
        let left = f64::from(pointer.0 - area.left) / area.scale_factor - attach_x;
        let top = f64::from(pointer.1 - area.top) / area.scale_factor - attach_y;
        let (x, y, width, height) = island_geometry(area, target.height, Some((left, top)));
        native::place_docked(window, x, y, width, height, false)?;
        position = Some(IslandPosition {
            monitor: monitor.name().cloned(),
            monitor_origin: (monitor.position().x, monitor.position().y),
            left: f64::from(x - area.left) / area.scale_factor,
            top: f64::from(y - area.top) / area.scale_factor,
        });
        Ok(())
    });
    let dropped = matches!(&attempt, Ok(native::EdgeDrag::Dropped));
    let result = match attempt {
        Ok(native::EdgeDrag::Click) => return Ok(false),
        Ok(native::EdgeDrag::Dropped) => match position.as_ref() {
            Some(position) => core.store.save_preference(
                ISLAND_POSITION,
                &serde_json::to_string(position).map_err(err)?,
            ),
            None => Ok(()),
        },
        Ok(native::EdgeDrag::Cancelled) => Ok(()),
        Err(error) => Err(error),
    };
    if position.is_some() && (!dropped || result.is_err()) {
        let (x, y, width, height) = saved_island_geometry(app, target.height)?;
        native::place_docked(window, x, y, width, height, false)?;
    }
    result.map(|()| true)
}

fn island_reconcile(
    app: &AppHandle,
    target: Option<crate::reminder_engine::Target>,
) -> AppResult<()> {
    use crate::{reminder_engine::Surface, reminder_host::IslandWindow};
    let core = app.state::<Core>();
    let service = &core.reminders;
    let Some(target) = target else {
        let label = service
            .island_window
            .lock()
            .map_err(err)?
            .as_ref()
            .map(|view| view.label.clone());
        if let Some(label) = label {
            if let Some(w) = app.get_webview_window(&label) {
                native::hide(&w)?;
                app.emit_to(&label, "host:island-content", None::<serde_json::Value>)
                    .map_err(err)?;
            }
            if let Some(view) = service.island_window.lock().map_err(err)?.as_mut() {
                view.idle_since.get_or_insert_with(Instant::now);
            }
        }
        return release_idle_island(app);
    };
    if hidden(app)? {
        return Ok(());
    }
    let (label, window) = {
        let mut cached = service.island_window.lock().map_err(err)?;
        let existing = cached
            .as_ref()
            .and_then(|view| app.get_webview_window(&view.label));
        let label = existing
            .as_ref()
            .map(|window| window.label().to_owned())
            .unwrap_or_else(|| format!("island-host-{}", uuid::Uuid::new_v4()));
        *cached = Some(IslandWindow {
            label: label.clone(),
            idle_since: None,
        });
        (label, existing)
    };
    let active = || service.target(Surface::Notice).as_ref() == Some(&target);
    let result = (|| {
        let w = if let Some(w) = window {
            w
        } else {
            build(
                app,
                &label,
                "island.html".into(),
                f64::from(ISLAND_WIDTH),
                f64::from(target.height),
            )?
        };
        wait_ready_while(app, &label, &target.owner, Duration::ZERO, active)?;
        let mut snapshot = service.notice_snapshot(&target)?;
        let show_window = snapshot["hidden"] != true;
        let click_through = snapshot["opacity"] == 0;
        snapshot["marketAudioMuted"] =
            serde_json::json!(core.integration.as_ref().is_some_and(|c| c.market_check()));
        app.emit_to(&label, "host:island-content", snapshot)
            .map_err(err)?;
        wait_ready_while(app, &label, &target.event, Duration::ZERO, active)?;
        let (x, y, width, height) = saved_island_geometry(app, target.height)?;
        service.show_target(app, &target, &label, || {
            w.set_ignore_cursor_events(click_through).map_err(err)?;
            if w.outer_position().map_err(err)? != tauri::PhysicalPosition::new(x, y)
                || w.outer_size().map_err(err)? != tauri::PhysicalSize::new(width, height)
            {
                native::place_docked(&w, x, y, width, height, false)?;
            }
            let visible = w.is_visible().map_err(err)?;
            if show_window && !visible {
                native::show(&w)?;
            } else if !show_window && visible {
                native::hide(&w)?;
            }
            Ok(())
        })
    })();
    {
        let key = format!("{label}/{}", target.event);
        let mut visibility = core.visibility.lock().map_err(err)?;
        visibility.ready.remove(&key);
        visibility.failed.remove(&key);
    }
    if let Err(error) = result {
        if error.starts_with("SESSION_EXPIRED:") {
            return Ok(());
        }
        if active() {
            service.failed(app, &target, error.clone());
        }
        service.island_window.lock().map_err(err)?.take();
        let _ = dispose(app, &label);
        return Err(error);
    }
    Ok(())
}

pub(crate) fn release_idle_island(app: &AppHandle) -> AppResult<()> {
    use crate::{reminder_engine::Surface, reminder_store::Channel};
    let core = app.state::<Core>();
    if core.reminders.target(Surface::Notice).is_some() {
        return Ok(());
    }
    let enabled = Channel::Island.enabled_in(&core.store)?;
    let mut cached = core.reminders.island_window.lock().map_err(err)?;
    if cached
        .as_ref()
        .is_some_and(|view| !enabled || view.expired(Instant::now()))
    {
        let label = cached.take().expect("已检查胶囊窗口").label;
        drop(cached);
        dispose(app, &label)?;
    }
    Ok(())
}
pub(crate) fn keyboard_close_all(app: &AppHandle) -> AppResult<()> {
    for label in app
        .webview_windows()
        .keys()
        .filter(|label| label.starts_with("keyboard-"))
        .cloned()
        .collect::<Vec<_>>()
    {
        dispose(app, &label)?;
    }
    Ok(())
}
pub(crate) fn keyboard_create(app: &AppHandle, id: &str, generation: u64) -> AppResult<()> {
    let core = app.state::<Core>();
    if core.quitting.load(std::sync::atomic::Ordering::Acquire)
        || hidden(app)?
        || !core.keyboard.current(id, generation)
    {
        return Ok(());
    }
    let label = format!("keyboard-{id}");
    let result = (|| {
        let w = build(
            app,
            &label,
            format!("index.html?keyboardOverlay={id}"),
            450.,
            240.,
        )?;
        w.set_ignore_cursor_events(true).map_err(err)?;
        w.set_focusable(false).map_err(err)?;
        wait_ready(app, &label, id)?;
        if !core.keyboard.current(id, generation) || hidden(app)? {
            dispose(app, &label)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = dispose(app, &label);
    }
    result
}
pub(crate) fn keyboard_visibility(app: &AppHandle, id: &str, generation: u64) -> AppResult<()> {
    let core = app.state::<Core>();
    if !core.keyboard.current(id, generation) {
        return Ok(());
    }
    let w = app
        .get_webview_window(&format!("keyboard-{id}"))
        .ok_or("键盘演示叠加层不存在")?;
    if core.quitting.load(std::sync::atomic::Ordering::Acquire)
        || hidden(app)?
        || !core.keyboard.wants_show(id, generation)
    {
        return native::hide(&w);
    }
    let area = area(app, false)?;
    let size = w.outer_size().map_err(err)?;
    w.set_position(tauri::PhysicalPosition::new(
        area.left + (area.width as i32 - size.width as i32) / 2,
        area.top + area.height as i32 - size.height as i32,
    ))
    .map_err(err)?;
    w.set_always_on_top(true).map_err(err)?;
    w.set_ignore_cursor_events(true).map_err(err)?;
    if core.keyboard.wants_show(id, generation) && !hidden(app)? {
        native::show(&w)?;
    }
    Ok(())
}
pub fn ready(app: &AppHandle, label: &str, id: &str) -> AppResult<()> {
    app.state::<Core>()
        .visibility
        .lock()
        .map_err(err)?
        .ready
        .insert(format!("{label}/{id}"), std::time::Instant::now());
    Ok(())
}
pub fn failed(app: &AppHandle, label: &str, id: &str, error: String) -> AppResult<()> {
    app.state::<Core>()
        .visibility
        .lock()
        .map_err(err)?
        .failed
        .insert(format!("{label}/{id}"), error);
    Ok(())
}
fn wait_ready(app: &AppHandle, label: &str, id: &str) -> AppResult<()> {
    wait_ready_while(app, label, id, Duration::from_millis(800), || true)
}
fn wait_ready_while(
    app: &AppHandle,
    label: &str,
    id: &str,
    observe: Duration,
    active: impl Fn() -> bool,
) -> AppResult<()> {
    let key = format!("{label}/{id}");
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    loop {
        if !active() {
            return Err("SESSION_EXPIRED: 显示任务已被替换".into());
        }
        {
            let core = app.state::<Core>();
            let v = core.visibility.lock().map_err(err)?;
            if let Some(e) = v.failed.get(&key) {
                return Err(format!("LOAD_FAILED: {e}"));
            }
            if v.ready.get(&key).is_some_and(|t| t.elapsed() >= observe) {
                return Ok(());
            }
        }
        if app.get_webview_window(label).is_none() {
            return Err("LOAD_FAILED: 目标窗口已关闭".into());
        }
        if std::time::Instant::now() >= deadline {
            return Err("LOAD_FAILED: 目标组件握手超时".into());
        }
        std::thread::sleep(Duration::from_millis(if observe.is_zero() {
            8
        } else {
            30
        }));
    }
}
fn wait_panel_shell(app: &AppHandle) -> AppResult<()> {
    let core = app.state::<Core>();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        let ready = core
            .visibility
            .lock()
            .map_err(err)?
            .panel_view
            .as_ref()
            .is_some_and(|view| view.ready);
        if ready {
            return Ok(());
        }
        if app.get_webview_window("panel").is_none() {
            return Err("面板已关闭".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // 宿主页未及时报告时仍显示窗口，保留加载状态和收起入口。
    Ok(())
}
pub(crate) fn area(
    app: &AppHandle,
    at_pointer: bool,
) -> AppResult<crate::desktop_geometry::WorkArea> {
    let monitor = if at_pointer {
        let p = app.cursor_position().map_err(err)?;
        app.monitor_from_point(p.x, p.y).map_err(err)?
    } else {
        app.get_webview_window("panel")
            .and_then(|w| w.current_monitor().ok().flatten())
    };
    monitor_area(app, monitor)
}
fn monitor_area(app: &AppHandle, monitor: Option<tauri::Monitor>) -> AppResult<WorkArea> {
    let m = monitor
        .or(app.primary_monitor().map_err(err)?)
        .ok_or("未找到可用显示器")?;
    Ok(work_area(&m))
}
fn work_area(m: &tauri::Monitor) -> WorkArea {
    let a = m.work_area();
    WorkArea {
        left: a.position.x,
        top: a.position.y,
        width: a.size.width,
        height: a.size.height,
        scale_factor: m.scale_factor(),
    }
}
fn dock_monitor(app: &AppHandle, settings: &Preferences) -> AppResult<tauri::Monitor> {
    if let Some(name) = &settings.edge_monitor {
        if let Some(monitor) = app
            .available_monitors()
            .map_err(err)?
            .into_iter()
            .find(|m| m.name() == Some(name))
        {
            return Ok(monitor);
        }
    }
    app.primary_monitor()
        .map_err(err)?
        .ok_or_else(|| "未找到可用显示器".into())
}
fn panel_area(app: &AppHandle) -> AppResult<WorkArea> {
    Ok(work_area(&dock_monitor(app, &read_panel_settings(app)?)?))
}
fn read_panel_settings(app: &AppHandle) -> AppResult<Preferences> {
    let raw = app.state::<Core>().store.preference("panel");
    let value = raw
        .map(|s| serde_json::from_str::<Preferences>(&s).map_err(err))
        .transpose()?
        .unwrap_or_default();
    value.validate()?;
    Ok(value)
}
pub fn panel_preferences(app: &AppHandle) -> AppResult<serde_json::Value> {
    serde_json::to_value(read_panel_settings(app)?).map_err(err)
}
pub fn panel_metrics(app: &AppHandle) -> AppResult<serde_json::Value> {
    if let Some(w) = app.get_webview_window("panel") {
        let area = panel_area(app)?;
        let r = native::describe(&w);
        return Ok(
            serde_json::json!({"effective_ratio":r["rect"]["width"].as_f64().unwrap_or(0.)/area.width as f64,"rect":r["rect"]}),
        );
    }
    Ok(serde_json::Value::Null)
}
pub fn save_panel(app: &AppHandle, ratio: f64, tint: f64) -> AppResult<()> {
    let mut settings = read_panel_settings(app)?;
    settings.ratio = ratio;
    settings.tint = tint;
    save_panel_settings(app, settings)
}
pub fn save_panel_settings(app: &AppHandle, settings: Preferences) -> AppResult<()> {
    settings.validate()?;
    let previous = read_panel_settings(app)?;
    let edge_changed = previous.edge != settings.edge
        || previous.edge_style != settings.edge_style
        || previous.edge_monitor != settings.edge_monitor;
    let layout = |settings: &Preferences| -> AppResult<()> {
        if edge_changed || previous.ratio != settings.ratio {
            layout_panel(app)?;
        }
        if edge_changed {
            sync_edge(app, settings)?;
        }
        Ok(())
    };
    app.state::<Core>()
        .store
        .save_preference("panel", &serde_json::to_string(&settings).map_err(err)?)?;
    if let Err(e) = layout(&settings) {
        app.state::<Core>()
            .store
            .save_preference("panel", &serde_json::to_string(&previous).map_err(err)?)?;
        if let Err(restore) = layout_panel(app).and_then(|_| sync_edge(app, &previous)) {
            return Err(format!("{e}；恢复面板几何失败：{restore}"));
        }
        return Err(e);
    }
    #[cfg(windows)]
    if app.state::<Core>().integration.is_none() && app.state::<Core>().verification.is_none() {
        crate::desktop_input::set_enabled(settings.desktop_double_click);
    }
    changed(app)
}

pub fn change_panel_groups(app: &AppHandle, change: crate::panel::GroupChange) -> AppResult<()> {
    let core = app.state::<Core>();
    let before = core.store.panel_groups()?;
    let area = match &change {
        crate::panel::GroupChange::Move { instance, group }
            if before.membership.get(instance) != group.as_ref() =>
        {
            Some(panel_placement_area(app)?)
        }
        _ => None,
    };
    let after = core.store.change_panel_groups(change, area)?;
    core.visibility
        .lock()
        .map_err(err)?
        .panel_scroll
        .retain(|group, _| {
            group
                .as_ref()
                .is_none_or(|id| after.groups.iter().any(|group| &group.id == id))
        });
    for instance in instances(app)? {
        if let Some(id) = instance["id"].as_str() {
            if before.contains(id) != after.contains(id) {
                release_surface(app, "panel", Some(id))?;
            }
        }
    }
    changed(app)?;
    layout_panel(app)
}

fn panel_placement_area(app: &AppHandle) -> AppResult<crate::panel::PlacementArea> {
    let settings = read_panel_settings(app)?;
    Ok(crate::panel::PlacementArea {
        work_area: work_area(&dock_monitor(app, &settings)?),
        ratio: settings.ratio,
    })
}

fn panel_min_width(app: &AppHandle) -> AppResult<f64> {
    let minimum = active_panel_instances(app)?
        .iter()
        .filter_map(|i| i["id"].as_str())
        .map(|id| dimensions(app, id).map(|(_, _, min, _)| min))
        .collect::<AppResult<Vec<_>>>()?
        .into_iter()
        .fold(0f64, f64::max);
    Ok(crate::panel::minimum_panel_width(minimum))
}

pub fn resize_panel_width(app: &AppHandle) -> AppResult<()> {
    if hidden(app)? {
        return Err("请先恢复组件显示".into());
    }
    let window = app.get_webview_window("panel").ok_or("面板已关闭")?;
    let area = panel_area(app)?;
    let minimum = panel_min_width(app)?;
    let result = (|| {
        let mut settings = read_panel_settings(app)?;
        if let Some(geometry) =
            native::resize_panel_width(&window, area, minimum, settings.edge.side)?
        {
            settings.ratio = geometry.effective_ratio;
            save_panel_settings(app, settings)
        } else {
            layout_panel(app)
        }
    })();
    if let Err(error) = result {
        if let Err(restore) = layout_panel(app) {
            return Err(format!("{error}；恢复原面板宽度失败：{restore}"));
        }
        return Err(error);
    }
    Ok(())
}

fn layout_panel(app: &AppHandle) -> AppResult<()> {
    if let Some(w) = app.get_webview_window("panel") {
        let a = panel_area(app)?;
        place_panel(app, &w, panel_geometry(app, a)?)?;
    }
    Ok(())
}
fn panel_geometry(
    app: &AppHandle,
    area: crate::desktop_geometry::WorkArea,
) -> AppResult<crate::desktop_geometry::PanelGeometry> {
    let settings = read_panel_settings(app)?;
    let mut geometry =
        crate::desktop_geometry::panel_geometry(area, settings.ratio, panel_min_width(app)?)?;
    geometry.left = settings.edge.side.window_left(area, geometry.width);
    Ok(geometry)
}
fn place_panel(
    app: &AppHandle,
    window: &WebviewWindow,
    geometry: crate::desktop_geometry::PanelGeometry,
) -> AppResult<()> {
    native::place_docked(
        window,
        geometry.left,
        geometry.top,
        geometry.width,
        geometry.height,
        false,
    )?;
    app.emit_to("panel", "host:panel-geometry", geometry)
        .map_err(err)
}
pub fn dimensions(app: &AppHandle, id: &str) -> AppResult<(f64, f64, f64, f64)> {
    let i = instance(app, id)?;
    let snapshot = app.state::<Core>().store.snapshot()?;
    let p = snapshot["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == i["packageId"])
        .ok_or("组件包不存在")?;
    let n = |v: &serde_json::Value| v.as_f64().ok_or("组件尺寸无效");
    Ok((
        n(i.get("width")
            .filter(|v| !v.is_null())
            .unwrap_or(&p["defaultSize"]["width"]))?,
        n(i.get("height")
            .filter(|v| !v.is_null())
            .unwrap_or(&p["defaultSize"]["height"]))?,
        n(&p["minSize"]["width"])?,
        n(&p["minSize"]["height"])?,
    ))
}
fn desktop_window(app: &AppHandle, id: &str) -> AppResult<WebviewWindow> {
    let label = format!("float-{id}");
    if let Some(w) = app.get_webview_window(&label) {
        return Ok(w);
    }
    let (width, height, _, _) = dimensions(app, id)?;
    let g = app.state::<Core>().store.window_layout(&label)?;
    let a = area(app, false)?;
    let (x, y, width, height) = if let Some(g) = g {
        let old_scale = g["scale"]
            .as_f64()
            .filter(|s| s.is_finite() && *s > 0.)
            .ok_or("保存缩放无效")?;
        let x = g["x"].as_f64().ok_or("保存位置无效")? * old_scale;
        let y = g["y"].as_f64().ok_or("保存位置无效")? * old_scale;
        let m = app
            .monitor_from_point(x, y)
            .map_err(err)?
            .or(app.primary_monitor().map_err(err)?)
            .ok_or("未找到显示器")?;
        let wa = m.work_area();
        let w = (g["width"].as_f64().unwrap_or(width) * m.scale_factor())
            .round()
            .min(wa.size.width as f64)
            .max(120.);
        let h = (g["height"].as_f64().unwrap_or(height) * m.scale_factor())
            .round()
            .min(wa.size.height as f64)
            .max(80.);
        (
            x.clamp(
                wa.position.x as f64,
                (wa.position.x as f64 + wa.size.width as f64 - w).max(wa.position.x as f64),
            ),
            y.clamp(
                wa.position.y as f64,
                (wa.position.y as f64 + wa.size.height as f64 - h).max(wa.position.y as f64),
            ),
            w,
            h,
        )
    } else {
        let width = (width * a.scale_factor).min(a.width as f64).round() as u32;
        let height = (height * a.scale_factor).min(a.height as f64).round() as u32;
        let occupied = app
            .webview_windows()
            .values()
            .filter(|w| w.label().starts_with("float-"))
            .map(|w| {
                let r = native::describe(w);
                let r = &r["rect"];
                (
                    r["left"].as_i64().unwrap_or(0) as i32,
                    r["top"].as_i64().unwrap_or(0) as i32,
                    r["width"].as_u64().unwrap_or(0) as u32,
                    r["height"].as_u64().unwrap_or(0) as u32,
                )
            })
            .collect::<Vec<_>>();
        let (x, y) = crate::desktop_geometry::initial_position(a, width, height, &occupied);
        (x as f64, y as f64, width as f64, height as f64)
    };
    let w = build(
        app,
        &label,
        format!("index.html?instance={id}"),
        width / a.scale_factor,
        height / a.scale_factor,
    )?;
    // 附着前先设定物理尺寸；attach 沿用已经验证的无焦点显示路径。
    native::place(&w, x as i32, y as i32, width as u32, height as u32, false)?;
    if let Err(e) = native::attach(&w, x as i32, y as i32) {
        let _ = dispose(app, &label);
        return Err(e);
    }
    if instance(app, id)?["pinned"] == true {
        native::toggle_pin(&w)?;
    }
    Ok(w)
}
pub fn show_instance(app: &AppHandle, id: &str) -> AppResult<()> {
    if hidden(app)? {
        return Err("请先恢复组件显示".into());
    }
    let i = instance(app, id)?;
    if i["location"] == "panel" {
        let core = app.state::<Core>();
        if app.get_webview_window("panel").is_none() || !core.store.panel_groups()?.contains(id) {
            // 面板收起或目标不在当前组时，首次显示由面板视口负责。
            return changed(app);
        }
        // 当前组已经打开时原位加载目标，保留面板的分组及窗口层级。
        core.visibility.lock().map_err(err)?.panel_target = Some(id.into());
        let result = changed(app).and_then(|_| wait_ready(app, "panel", id));
        core.visibility.lock().map_err(err)?.panel_target = None;
        result?;
    } else {
        let w = desktop_window(app, id)?;
        if let Err(e) = wait_ready(app, w.label(), id) {
            let _ = dispose(app, w.label());
            return Err(e);
        }
        native::show(&w)?;
    }
    Ok(())
}

pub(crate) fn open_notification_source(app: &AppHandle, id: &str) -> AppResult<()> {
    let source = instance(app, id)?;
    let was_hidden = source["visible"] != true;
    if was_hidden {
        restore_instance(app, id)?;
    }
    if source["location"] == "panel" {
        let groups = app.state::<Core>().store.panel_groups()?;
        let group = groups.membership.get(id).cloned();
        if group != groups.active {
            change_panel_groups(app, crate::panel::GroupChange::Select { group })?;
        }
        show_panel(app, true)?;
        show_instance(app, id)?;
        app.emit_to("panel", "host:reveal-instance", id)
            .map_err(err)
    } else if was_hidden {
        Ok(())
    } else {
        show_instance(app, id)
    }
}
pub fn show_panel_on_hover(app: &AppHandle) -> AppResult<bool> {
    let Some(edge) = app.get_webview_window("edge") else {
        return Ok(false);
    };
    if hidden(app)? || scene_is_foreground(panel_scene()) || !edge.is_visible().map_err(err)? {
        return Ok(false);
    }
    let blocked = app
        .state::<Core>()
        .visibility
        .lock()
        .map_err(err)?
        .edge_hover_block
        .is_some();
    if blocked || app.get_webview_window("panel").is_some() || !native::hover_hits(&edge)? {
        return Ok(false);
    }
    show_panel(app, false)?;
    Ok(true)
}

fn set_panel_layer(window: &WebviewWindow, foreground: bool, focus: bool) -> AppResult<()> {
    if let Err(error) = native::set_panel_layer(window, foreground, focus) {
        return match native::set_panel_layer(window, !foreground, false) {
            Ok(()) => Err(error),
            Err(restore) => Err(format!("{error}；恢复面板层级失败：{restore}")),
        };
    }
    set_panel_foreground(foreground);
    Ok(())
}

pub fn show_panel(app: &AppHandle, focus: bool) -> AppResult<()> {
    if hidden(app)? {
        return Ok(());
    }
    let created = app.get_webview_window("panel").is_none();
    let area = panel_area(app)?;
    let view_id = created.then(|| uuid::Uuid::new_v4().to_string());
    {
        let core = app.state::<Core>();
        let mut v = core.visibility.lock().map_err(err)?;
        if let Some(id) = &view_id {
            v.panel_view = Some(PanelView {
                id: id.clone(),
                ready: false,
            });
        }
    }
    let geometry = panel_geometry(app, area)?;
    let w = if let Some(id) = view_id {
        build(
            app,
            "panel",
            format!("index.html?panel=1&view={id}"),
            f64::from(geometry.width) / area.scale_factor,
            f64::from(geometry.height) / area.scale_factor,
        )?
    } else {
        app.get_webview_window("panel").ok_or("面板已关闭")?
    };
    place_panel(app, &w, geometry)?;
    let pointer_started_inside = native::pointer_inside(&w).unwrap_or(false);
    let entering_foreground = created || !scene_is_foreground(panel_scene());
    if created {
        wait_panel_shell(app)?;
        let settings = read_panel_settings(app)?;
        if let Err(e) = native::slide_panel(&w, true, settings.slide_in_ms, settings.edge.side) {
            app.state::<Core>().report(e);
        }
        native::maintain_panel_layer(&w, true, focus)?;
        set_panel_foreground(true);
    } else if !scene_is_foreground(panel_scene()) {
        set_panel_layer(&w, true, focus)?;
    } else {
        native::maintain_panel_layer(&w, true, focus)?;
        native::show(&w)?;
    }
    if focus {
        w.set_focus().map_err(err)?;
    }
    if let Some(edge) = app.get_webview_window("edge") {
        native::hide(&edge)?;
    }
    if entering_foreground && !panel_locked(app)? {
        watch_panel_dismissal(&w, pointer_started_inside)?;
    }
    changed(app)
}
pub fn collapse(app: &AppHandle) -> AppResult<()> {
    let Some(w) = app.get_webview_window("panel") else {
        return Ok(());
    };
    let settings = read_panel_settings(app)?;
    if panel_locked(app)? {
        if !scene_is_foreground(panel_scene()) {
            return Ok(());
        }
        set_panel_layer(&w, false, false)?;
    } else {
        if let Err(e) = native::slide_panel(&w, false, settings.slide_out_ms, settings.edge.side) {
            app.state::<Core>().report(e);
        }
        dispose(app, "panel")?;
        set_panel_foreground(false);
    }
    if !hidden(app)? {
        let edge = edge_window(app)?;
        place_edge(
            &edge,
            work_area(&dock_monitor(app, &settings)?),
            settings.edge_style,
            settings.edge,
        )?;
        emit_edge(app, "edge", settings.edge_style, settings.edge)?;
        native::show(&edge)?;
        block_hover_until_exit(app, &edge)?;
    }
    changed(app)
}
fn block_hover_until_exit(app: &AppHandle, edge: &WebviewWindow) -> AppResult<()> {
    let block = native::pointer_inside(edge)?.then(std::time::Instant::now);
    app.state::<Core>()
        .visibility
        .lock()
        .map_err(err)?
        .edge_hover_block = block;
    if let Some(block) = block {
        let edge = edge.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let inside = edge.is_visible().unwrap_or(false)
                    && native::pointer_inside(&edge).unwrap_or(false);
                let core = edge.state::<Core>();
                if let Ok(mut state) = core.visibility.lock() {
                    if state.edge_hover_block != Some(block) {
                        break;
                    }
                    if !inside || state.hidden {
                        state.edge_hover_block = None;
                        break;
                    }
                } else {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
    }
    Ok(())
}
fn panel_locked(app: &AppHandle) -> AppResult<bool> {
    Ok(app
        .state::<Core>()
        .visibility
        .lock()
        .map_err(err)?
        .panel_locked)
}

fn save_panel_lock(app: &AppHandle, locked: bool) -> AppResult<()> {
    let core = app.state::<Core>();
    core.store
        .save_preference("panel_locked", if locked { "true" } else { "false" })?;
    core.visibility.lock().map_err(err)?.panel_locked = locked;
    Ok(())
}

pub fn set_panel_locked(app: &AppHandle, locked: bool) -> AppResult<()> {
    if panel_locked(app)? == locked {
        return Ok(());
    }
    let panel = app.get_webview_window("panel");
    if !locked && panel.is_some() && !scene_is_foreground(panel_scene()) {
        show_panel(app, true)?;
    }
    save_panel_lock(app, locked)?;
    if let Some(panel) = panel {
        if scene_is_foreground(panel_scene()) {
            set_panel_foreground(true);
            if !locked {
                watch_panel_dismissal(&panel, false)?;
            }
        }
    }
    changed(app)
}
pub fn edge_window(app: &AppHandle) -> AppResult<WebviewWindow> {
    if let Some(w) = app.get_webview_window("edge") {
        return Ok(w);
    }
    let settings = read_panel_settings(app)?;
    let style = serde_json::json!(settings.edge_style);
    let side = serde_json::json!(settings.edge.side);
    let url = format!(
        "index.html?edge=1&style={}&side={}",
        style.as_str().unwrap(),
        side.as_str().unwrap()
    );
    let (width, height) = settings.edge_style.size();
    let w = WebviewWindowBuilder::new(app, "edge", WebviewUrl::App(url.into()))
        .title("SoulDesk · 侧边入口")
        .inner_size(width, height)
        .min_inner_size(1., 1.)
        .transparent(true)
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .shadow(false)
        .decorations(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .visible(false)
        .focused(false)
        .initialization_script(host_script(app))
        .build()
        .map_err(err)?;
    native::prepare(&w)?;
    place_edge(
        &w,
        work_area(&dock_monitor(app, &settings)?),
        settings.edge_style,
        settings.edge,
    )?;
    Ok(w)
}
fn position_edge(app: &AppHandle, w: &WebviewWindow) -> AppResult<()> {
    let settings = read_panel_settings(app)?;
    place_edge(
        w,
        work_area(&dock_monitor(app, &settings)?),
        settings.edge_style,
        settings.edge,
    )
}
fn edge_size(area: WorkArea, style: EdgeStyle) -> (u32, u32) {
    let (width, height) = style.size();
    let pixels = |logical: f64, limit: u32| {
        (logical * area.scale_factor)
            .round()
            .clamp(1., f64::from(limit.max(1))) as u32
    };
    (pixels(width, area.width), pixels(height, area.height))
}
fn place_edge(
    w: &WebviewWindow,
    area: WorkArea,
    style: EdgeStyle,
    position: EdgePosition,
) -> AppResult<()> {
    let (width, height) = edge_size(area, style);
    native::place_docked(
        w,
        position.side.window_left(area, width),
        position.top(area, height),
        width,
        height,
        false,
    )
}
fn emit_edge(
    app: &AppHandle,
    label: &str,
    style: EdgeStyle,
    position: EdgePosition,
) -> AppResult<()> {
    app.emit_to(
        label,
        "host:panel-edge",
        serde_json::json!({"style": style, "position": position}),
    )
    .map_err(err)
}
fn sync_edge(app: &AppHandle, settings: &Preferences) -> AppResult<()> {
    if let Some(w) = app.get_webview_window("edge") {
        place_edge(
            &w,
            work_area(&dock_monitor(app, settings)?),
            settings.edge_style,
            settings.edge,
        )?;
        emit_edge(app, "edge", settings.edge_style, settings.edge)?;
    }
    if app.get_webview_window("panel").is_some() {
        emit_edge(app, "panel", settings.edge_style, settings.edge)?;
    }
    Ok(())
}

pub fn drag_panel_edge(
    app: &AppHandle,
    source: &WebviewWindow,
    press: native::EdgePress,
) -> AppResult<bool> {
    let panel = app.get_webview_window("panel");
    // 置底时独立挂件仍可拖动，已开始滑入时则由面板上的挂件接手。
    let window = if source.label() == "edge" && !source.is_visible().map_err(err)? {
        panel.as_ref().unwrap_or(source)
    } else {
        source
    };
    if hidden(app)? || !window.is_visible().map_err(err)? {
        return Ok(false);
    }
    let mut settings = read_panel_settings(app)?;
    let original = settings.edge;
    let origin = dock_monitor(app, &settings)?;
    let origin_area = work_area(&origin);
    let (_, origin_height) = edge_size(origin_area, settings.edge_style);
    let monitors = app.available_monitors().map_err(err)?;
    let geometry = if panel.is_some() {
        Some((settings.ratio, panel_min_width(app)?))
    } else {
        None
    };
    let mut current = original;
    let mut target = &origin;
    let attempt =
        native::drag_panel_edge(window, press, origin_area.scale_factor, |start, pointer| {
            let next_monitor = monitors
                .iter()
                .find(|monitor| {
                    let p = monitor.position();
                    let size = monitor.size();
                    (0..i64::from(size.width)).contains(&(i64::from(pointer.0) - i64::from(p.x)))
                        && (0..i64::from(size.height))
                            .contains(&(i64::from(pointer.1) - i64::from(p.y)))
                })
                .unwrap_or(target);
            let monitor_changed = target.position() != next_monitor.position();
            let area = work_area(next_monitor);
            let (_, height) = edge_size(area, settings.edge_style);
            let grab_ratio = ((f64::from(start.1)
                - f64::from(original.top(origin_area, origin_height)))
                / f64::from(origin_height))
            .clamp(0., 1.);
            let next = original.dragged(area, height, pointer, grab_ratio);
            if monitor_changed || next != current {
                let move_panel = monitor_changed || current.side != next.side;
                target = next_monitor;
                current = next;
                if let (Some(panel), Some((ratio, minimum))) = (&panel, geometry) {
                    if move_panel {
                        let mut geometry =
                            crate::desktop_geometry::panel_geometry(area, ratio, minimum)?;
                        geometry.left = next.side.window_left(area, geometry.width);
                        place_panel(app, panel, geometry)?;
                    }
                    emit_edge(app, "panel", settings.edge_style, next)?;
                }
                if window.label() == "edge" {
                    place_edge(window, area, settings.edge_style, next)?;
                    emit_edge(app, "edge", settings.edge_style, next)?;
                }
            }
            Ok(())
        });
    let committed = matches!(&attempt, Ok(native::EdgeDrag::Dropped));
    let result = match attempt {
        Ok(native::EdgeDrag::Dropped)
            if current != original || target.position() != origin.position() =>
        {
            settings.edge = current;
            settings.edge_monitor = target.name().cloned();
            save_panel_settings(app, settings).map(|()| true)
        }
        other => other.map(|outcome| !matches!(outcome, native::EdgeDrag::Click)),
    };
    if (!committed || result.is_err())
        && (current != original || target.position() != origin.position())
    {
        // 保存失败或取消，恢复最后一次持久化的位置。
        layout_panel(app)?;
        sync_edge(app, &read_panel_settings(app)?)?;
    }
    if window.label() == "edge" && !matches!(&result, Ok(false)) {
        block_hover_until_exit(app, window)?;
    }
    result
}

pub fn remove_instance(app: &AppHandle, id: &str) -> AppResult<()> {
    hide_instance(app, id)?;
    crate::component_lifecycle::remove_instance(app, id)?;
    changed(app)
}

pub fn hide_instance(app: &AppHandle, id: &str) -> AppResult<()> {
    let i = instance(app, id)?;
    app.state::<Core>().store.presentation(
        id,
        i["location"].as_str().unwrap(),
        false,
        i["pinned"] == true,
    )?;
    crate::component_lifecycle::after_instance_hidden(app, id)?;
    save_instance_geometry(app, id)?;
    dispose(app, &format!("float-{id}"))?;
    dispose(app, &format!("detail-{id}"))?;
    if i["location"] == "panel" {
        // 前端按稳定实例标识移除一个组件；不销毁同面板其他实例的页面或连接。
        release_surface(app, "panel", Some(id))?;
    }
    changed(app)
}
fn save_instance_geometry(app: &AppHandle, id: &str) -> AppResult<()> {
    if let Some(w) = app.get_webview_window(&format!("float-{id}")) {
        save_geometry(&w)?;
    }
    Ok(())
}
/// 已持久归属和临时窗口共同回滚；目标完成 SDK ready 后才算切换成功。
pub fn move_to(app: &AppHandle, id: &str, location: &str) -> AppResult<()> {
    if hidden(app)? {
        return Err("请先恢复组件显示".into());
    }
    if !["desktop", "panel"].contains(&location) {
        return Err("未知展示位置".into());
    }
    let before = instance(app, id)?;
    let old = before["location"].as_str().unwrap();
    if old == location && before["visible"] == true {
        return show_instance(app, id);
    }
    let area = (old == "desktop" && location == "panel")
        .then(|| panel_placement_area(app))
        .transpose()?;
    let core = app.state::<Core>();
    let pinned = before["pinned"] == true;
    let panel_open = app.get_webview_window("panel").is_some();
    let resize_for_embed = area.is_some() && panel_open && core.store.panel_groups()?.contains(id);
    save_instance_geometry(app, id)?;
    dispose(app, &format!("float-{id}"))?;
    dispose(app, &format!("detail-{id}"))?;
    if old == "panel" || location == "panel" {
        release_surface(app, "panel", Some(id))?;
    }
    let mut placement = None;
    let result: AppResult<()> = (|| {
        if let Some(area) = area {
            placement = Some(core.store.embed_in_panel(id, pinned, area)?);
            if resize_for_embed {
                layout_panel(app)?;
            }
        } else {
            core.store.presentation(id, location, true, pinned)?;
        }
        changed(app)?;
        show_instance(app, id)?;
        if app.state::<Core>().integration.is_some() {
            let core = app.state::<Core>();
            let mut fault = core.transfer_failure.lock().map_err(err)?;
            if fault.as_deref() == Some(&format!("{id}/{location}")) {
                *fault = None;
                return Err("受控失败：目标已完成实际 SDK 挂载".into());
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        dispose(app, &format!("float-{id}"))?;
        if panel_open {
            release_surface(app, "panel", Some(id))?;
        }
        let restore_panel_geometry = resize_for_embed && placement.is_some();
        if let Some(placement) = placement {
            core.store
                .rollback_panel_embed(id, before["visible"] == true, pinned, placement)?;
        } else {
            core.store
                .presentation(id, old, before["visible"] == true, pinned)?;
        }
        changed(app)?;
        // 状态快速回滚时前端快照可能合并；仅重建目标iframe，其他面板实例保留。
        if panel_open {
            app.emit("host:reload-instance", id).map_err(err)?;
        }
        let restore: AppResult<()> = (|| {
            if before["visible"] == true
                && (old == "desktop"
                    || panel_open && app.state::<Core>().store.panel_groups()?.contains(id))
            {
                show_instance(app, id)?;
            }
            if restore_panel_geometry {
                layout_panel(app)?;
            }
            Ok(())
        })();
        return match restore {
            Ok(()) => Err(format!("{error}；已恢复原展示")),
            Err(e) => Err(format!(
                "{error}；恢复原展示也失败：{e}，配置仍保留，请从管理重试"
            )),
        };
    }
    Ok(())
}
pub fn move_instance(app: &AppHandle, id: &str, floating: bool) -> AppResult<()> {
    move_to(app, id, if floating { "desktop" } else { "panel" })
}
pub fn restore_instance(app: &AppHandle, id: &str) -> AppResult<()> {
    let i = instance(app, id)?;
    move_to(app, id, i["location"].as_str().unwrap())
}
pub fn pin(app: &AppHandle, id: &str) -> AppResult<()> {
    let i = instance(app, id)?;
    if i["location"] != "desktop" || i["visible"] != true {
        return Err("只有可见桌面组件可以置顶".into());
    }
    let w = app
        .get_webview_window(&format!("float-{id}"))
        .ok_or("组件窗口尚未恢复")?;
    let pinned = native::toggle_pin(&w)?;
    if let Err(e) = app
        .state::<Core>()
        .store
        .presentation(id, "desktop", true, pinned)
    {
        native::toggle_pin(&w)?;
        return Err(e);
    }
    changed(app)
}
pub fn detail(app: &AppHandle, id: &str) -> AppResult<()> {
    detail_at(app, id, "quotes")
}
pub fn detail_capability(page: &str) -> AppResult<Option<&'static str>> {
    match page {
        "quotes" | "rules" | "history" => Ok(None),
        "pomodoro-scenes" | "pomodoro-settings" | "pomodoro-history" => Ok(Some("focus.pomodoro")),
        page if page.starts_with("market-chart:") => {
            let mut fields = page["market-chart:".len()..].split(':');
            let market = fields.next().unwrap_or_default();
            let symbol = fields.next().ok_or("行情详情缺少标的")?;
            let price_kind = match fields.next() {
                None | Some("last") => crate::alerts::PriceKind::Last,
                Some("mark") => crate::alerts::PriceKind::Mark,
                _ => return Err("行情详情价格来源无效".into()),
            };
            if fields.next().is_some() {
                return Err("行情详情参数无效".into());
            }
            let key = crate::alerts::MarketKey {
                market: market.into(),
                ..crate::alerts::MarketKey::binance(symbol, price_kind)
            };
            key.validate()?;
            Ok(Some("market.chart"))
        }
        _ => Err("未知详情页".into()),
    }
}
pub fn detail_at(app: &AppHandle, id: &str, page: &str) -> AppResult<()> {
    detail_capability(page)?;
    if hidden(app)? || instance(app, id)?["visible"] != true {
        return Err("请先显示组件".into());
    }
    show_detail(app, id, page)
}

pub(crate) fn open_market_alert_chart(
    app: &AppHandle,
    key: &crate::alerts::MarketKey,
) -> AppResult<()> {
    key.validate()?;
    let Some(instance) = instances(app)?
        .into_iter()
        .filter(|entry| entry["packageId"] == crate::alert_notice::MARKET_PACKAGE)
        .max_by_key(|entry| entry["visible"] == true)
    else {
        show(app, true)?;
        return Err("找不到行情组件实例，请从组件库添加后打开 K 线".into());
    };
    let id = instance["id"].as_str().ok_or("行情组件实例无效")?;
    let price = match key.price_kind {
        crate::alerts::PriceKind::Last => "last",
        crate::alerts::PriceKind::Mark => "mark",
    };
    show_detail(
        app,
        id,
        &format!("market-chart:{}:{}:{price}", key.market, key.symbol),
    )
}

fn show_detail(app: &AppHandle, id: &str, page: &str) -> AppResult<()> {
    let label = format!("detail-{id}");
    let w = if let Some(w) = app.get_webview_window(&label) {
        w
    } else {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("detail", id)
            .append_pair("page", page)
            .finish();
        let (width, height) = if page.starts_with("market-chart:") {
            (1120., 760.)
        } else {
            (820., 740.)
        };
        build(app, &label, format!("index.html?{query}"), width, height)?
    };
    clamp(&w)?;
    w.show().map_err(err)?;
    w.set_focus().map_err(err)?;
    app.emit_to(&label, "host:detail-page", page).map_err(err)
}
pub fn hide_all(app: &AppHandle) -> AppResult<()> {
    app.state::<Core>()
        .jobs
        .hide_all(&app.state::<Core>().store)?;
    app.state::<Core>().jobs.cleanup(app)?;
    app.state::<Core>().assistant.cancel_all()?;
    app.state::<Core>().assistant.cleanup(app)?;
    save_all_geometry(app)?;
    app.state::<Core>()
        .store
        .save_preference("hidden_all", "true")?;
    app.state::<Core>().pomodoro_windows.suspend_all()?;
    {
        let core = app.state::<Core>();
        let mut v = core.visibility.lock().map_err(err)?;
        v.hidden = true;
    }
    set_panel_foreground(false);
    for label in app
        .webview_windows()
        .keys()
        .filter(|l| !l.starts_with("trial-"))
        .cloned()
        .collect::<Vec<_>>()
    {
        dispose(app, &label)?;
    }
    Ok(())
}
pub fn restore(app: &AppHandle) -> AppResult<()> {
    app.state::<Core>()
        .store
        .save_preference("hidden_all", "false")?;
    {
        let core = app.state::<Core>();
        let mut v = core.visibility.lock().map_err(err)?;
        v.hidden = false;
    }
    let mut errors = Vec::new();
    for i in instances(app)?
        .iter()
        .filter(|i| i["visible"] == true && i["location"] == "desktop")
    {
        if let Err(e) = show_instance(app, i["id"].as_str().unwrap()) {
            errors.push(e);
        }
    }
    if !scene_is_foreground(panel_scene()) {
        native::show(&edge_window(app)?)?;
    }
    changed(app)?;
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}
pub fn toggle(app: &AppHandle) -> AppResult<()> {
    if hidden(app)? {
        restore(app)?;
        return show_panel(app, true);
    }
    if !scene_is_foreground(panel_scene()) {
        return show_panel(app, true);
    }
    // 锁定面板可能仍被宿主记为前台，但 Windows 已取消实际置顶。
    if panel_locked(app)?
        && app
            .get_webview_window("panel")
            .as_ref()
            .map(native::panel_is_topmost)
            .transpose()?
            == Some(false)
    {
        return show_panel(app, true);
    }
    collapse(app)
}

pub fn toggle_locked_panel(app: &AppHandle) -> AppResult<()> {
    let open = app.get_webview_window("panel").is_some();
    let was_locked = panel_locked(app)?;
    let locked = !open;
    if locked != was_locked {
        save_panel_lock(app, locked)?;
    }
    let result = if open { collapse(app) } else { toggle(app) };
    if let Err(error) = result {
        if locked != was_locked && app.get_webview_window("panel").is_some() == open {
            if let Err(restore) = save_panel_lock(app, was_locked) {
                return Err(format!("{error}；恢复面板锁定失败：{restore}"));
            }
        }
        return Err(error);
    }
    Ok(())
}

pub(crate) fn dismiss_from_outside(app: &AppHandle, scene: u64) -> AppResult<()> {
    if scene_is_foreground(scene) && panel_scene() == scene && !hidden(app)? {
        collapse(app)?;
    }
    Ok(())
}
pub fn clamp(w: &WebviewWindow) -> AppResult<()> {
    if w.label().starts_with("float-") {
        return native::clamp_desktop(w);
    }
    if let Some(m) = w
        .current_monitor()
        .map_err(err)?
        .or(w.primary_monitor().map_err(err)?)
    {
        let a = m.work_area();
        let sz = w.outer_size().map_err(err)?;
        let p = w.outer_position().map_err(err)?;
        if sz.width > a.size.width || sz.height > a.size.height {
            w.set_size(tauri::PhysicalSize::new(
                sz.width.min(a.size.width),
                sz.height.min(a.size.height),
            ))
            .map_err(err)?;
        }
        let sz = w.outer_size().map_err(err)?;
        w.set_position(tauri::PhysicalPosition::new(
            p.x.clamp(
                a.position.x,
                a.position.x + a.size.width.saturating_sub(sz.width) as i32,
            ),
            p.y.clamp(
                a.position.y,
                a.position.y + a.size.height.saturating_sub(sz.height) as i32,
            ),
        ))
        .map_err(err)?;
    }
    Ok(())
}
pub fn repair(app: &AppHandle) -> AppResult<()> {
    layout_panel(app)?;
    for w in app.webview_windows().values() {
        // 专用窗允许收起到工作区外；几何和缩放由其协调器处理。
        if w.label().starts_with("pomodoro-window-") {
            continue;
        }
        if !matches!(w.label(), "panel" | "edge") {
            clamp(w)?;
        }
        if w.label().starts_with("float-") {
            let i = instance(app, w.label().trim_start_matches("float-"))?;
            if native::check_attachment(w, i["pinned"] == true).is_err() {
                let r = native::describe(w);
                native::attach(
                    w,
                    r["rect"]["left"].as_i64().unwrap_or(0) as i32,
                    r["rect"]["top"].as_i64().unwrap_or(0) as i32,
                )?;
                if i["pinned"] == true {
                    native::toggle_pin(w)?;
                }
            }
        }
    }
    if let Some(w) = app.get_webview_window("edge") {
        position_edge(app, &w)?;
    }
    Ok(())
}
pub fn destroy_views(app: &AppHandle) -> AppResult<()> {
    hide_all(app)
}

#[cfg(test)]
mod document_drop_tests {
    use super::*;

    #[test]
    fn island_geometry_places_default_near_top_and_keeps_saved_offsets_in_view() {
        let area = WorkArea {
            left: -1600,
            top: 30,
            width: 1600,
            height: 900,
            scale_factor: 1.25,
        };
        assert_eq!(island_geometry(area, 72, None), (-1019, 45, 438, 90));
        assert_eq!(
            island_geometry(area, 48, Some((50., 30.))),
            (-1537, 68, 438, 60)
        );
        assert_eq!(
            island_geometry(area, 800, Some((50., 300.))),
            (-1537, 30, 438, 900)
        );
    }

    #[test]
    fn panel_waits_three_seconds_after_leaving_and_resets_on_return() {
        let start = Instant::now();
        let at = |milliseconds| start + Duration::from_millis(milliseconds);
        let mut presence = PanelPresence::default();
        assert!(!presence.update(false, at(0)));
        assert!(!presence.update(false, at(3_000)));
        assert!(!presence.update(true, at(4_000)));
        assert!(!presence.update(false, at(4_100)));
        assert!(!presence.update(false, at(7_099)));
        assert!(!presence.update(true, at(7_100)));
        assert!(!presence.update(false, at(7_200)));
        assert!(presence.update(false, at(10_200)));
    }

    #[test]
    fn panel_scroll_survives_window_recreation_and_stale_view_cannot_overwrite_it() {
        let view = |id: &str| {
            Some(PanelView {
                id: id.into(),
                ready: false,
            })
        };
        let mut state = Visibility {
            panel_view: view("first"),
            ..Default::default()
        };
        let position = PanelScroll {
            left: 10.,
            top: 340.,
        };
        let group = Some("work".to_owned());
        state
            .update_panel_view(
                "first",
                PanelViewUpdate::Scroll {
                    group: group.clone(),
                    position,
                },
            )
            .unwrap();
        state.panel_view = view("second");
        state
            .update_panel_view("first", PanelViewUpdate::Ready)
            .unwrap();
        state
            .update_panel_view(
                "first",
                PanelViewUpdate::Scroll {
                    group: group.clone(),
                    position: PanelScroll::default(),
                },
            )
            .unwrap();
        assert!(!state.panel_view.as_ref().unwrap().ready);
        assert_eq!(state.panel_scroll[&group], position);
        state
            .update_panel_view("second", PanelViewUpdate::Ready)
            .unwrap();
        assert!(state.panel_view.unwrap().ready);
        assert!(state.ready.is_empty());
    }

    #[test]
    fn hiding_one_panel_instance_preserves_other_session_tokens_and_windows() {
        let session = |instance: &str, window: &str| crate::widgets::Session {
            instance: instance.into(),
            window: window.into(),
            shared_cursor: None,
            preview: None,
        };
        let mut sessions = std::collections::HashMap::from([
            ("hidden-panel".into(), session("first", "panel")),
            ("other-panel".into(), session("second", "panel")),
            ("first-detail".into(), session("first", "detail-first")),
            ("similar-id".into(), session("first-2", "panel")),
        ]);
        assert_eq!(
            take_surface_sessions(&mut sessions, "panel", Some("first")),
            std::collections::HashSet::from(["hidden-panel".into()])
        );
        assert_eq!(sessions.len(), 3);
        assert_eq!(sessions["other-panel"].instance, "second");
        assert!(sessions.contains_key("first-detail"));
        assert!(sessions.contains_key("similar-id"));
        assert_eq!(take_surface_sessions(&mut sessions, "panel", None).len(), 2);
        assert!(sessions.contains_key("first-detail"));
    }

    #[test]
    fn detail_pages_are_fixed_and_pomodoro_requires_its_capability() {
        for page in ["pomodoro-scenes", "pomodoro-settings", "pomodoro-history"] {
            assert_eq!(detail_capability(page).unwrap(), Some("focus.pomodoro"));
        }
        assert_eq!(detail_capability("quotes").unwrap(), None);
        for page in [
            "https://example.com",
            "../pomodoro-scenes",
            "pomodoro-scenes?instance=other",
            "unknown",
        ] {
            assert!(detail_capability(page).is_err());
        }
    }

    #[test]
    fn html_drop_only_applies_to_formal_windows_with_document_components() {
        let store = crate::store::Store::open(
            std::env::temp_dir().join(format!("souldesk-drop-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        for package in ["local.fixture.surface", "local.souldesk.widgetjs.convert"] {
            let staged = crate::widgets::stage_bytes(
                &store,
                crate::presets::find(package)
                    .unwrap()
                    .unwrap()
                    .archive()
                    .unwrap(),
            )
            .unwrap();
            crate::widgets::activate(&store, &staged).unwrap();
        }
        let clock = crate::widgets::create_instance(&store, "local.fixture.surface").unwrap();
        let document =
            crate::widgets::create_instance(&store, "local.souldesk.widgetjs.convert").unwrap();
        assert!(!html_drop_for_window(&store, "main").unwrap());
        assert!(!html_drop_for_window(&store, "trial-documents").unwrap());
        assert!(!html_drop_for_window(&store, &format!("float-{clock}")).unwrap());
        assert!(html_drop_for_window(&store, &format!("float-{document}")).unwrap());
        assert!(!html_drop_for_window(&store, "panel").unwrap());
        store.presentation(&document, "panel", true, false).unwrap();
        assert!(html_drop_for_window(&store, "panel").unwrap());
        store
            .presentation(&document, "panel", false, false)
            .unwrap();
        assert!(!html_drop_for_window(&store, "panel").unwrap());
    }
}

#[cfg(windows)]
use crate::desktop_native as native;

/// 工作区/缩放变化时才重排；桌面附着失效会报告并尝试恢复，不改成浮窗。
pub fn monitor_changes(app: &AppHandle) -> AppResult<()> {
    let signature = serde_json::to_string(
        &app.available_monitors()
            .map_err(err)?
            .iter()
            .map(|m| {
                let a = m.work_area();
                serde_json::json!([
                    m.name(),
                    a.position.x,
                    a.position.y,
                    a.size.width,
                    a.size.height,
                    m.scale_factor()
                ])
            })
            .collect::<Vec<_>>(),
    )
    .map_err(err)?;
    let changed = {
        let core = app.state::<Core>();
        let mut v = core.visibility.lock().map_err(err)?;
        let changed = v.monitors != signature;
        v.monitors = signature;
        changed
    };
    if changed {
        app.state::<Core>().assistant.monitor_changed(app)?;
        repair(app)?;
    }
    if let Some(panel) = app.get_webview_window("panel") {
        if let Err(error) =
            native::maintain_panel_layer(&panel, scene_is_foreground(panel_scene()), false)
        {
            app.state::<Core>().report(error);
        }
    }
    for w in app
        .webview_windows()
        .values()
        .filter(|w| w.label().starts_with("float-"))
    {
        let id = w.label().trim_start_matches("float-");
        let i = instance(app, id)?;
        if let Err(e) = native::check_attachment(w, i["pinned"] == true) {
            app.state::<Core>().report(e);
            repair(app)?;
        }
    }
    Ok(())
}

pub fn resize_instance(app: &AppHandle, id: &str, width: u32, height: u32) -> AppResult<()> {
    let before = instance(app, id)?;
    if before["location"] == "panel" {
        let layouts = app.state::<Core>().store.panel_layouts()?;
        let old = &layouts[id];
        app.state::<Core>().store.save_panel_layout(
            id,
            crate::panel::Layout {
                x: old["x"].as_f64().unwrap_or(0.0),
                y: old["y"].as_f64().unwrap_or(0.0),
                width: width as f64,
                height: height as f64,
            },
        )?;
        return changed(app);
    }
    crate::widgets::set_instance_size(&app.state::<Core>().store, id, width, height)?;
    let result: AppResult<()> = (|| {
        if let Some(w) = app.get_webview_window(&format!("float-{id}")) {
            let r = native::describe(&w);
            let scale = w.scale_factor().map_err(err)?;
            native::place(
                &w,
                r["rect"]["left"].as_i64().unwrap_or(0) as i32,
                r["rect"]["top"].as_i64().unwrap_or(0) as i32,
                (width as f64 * scale).round() as u32,
                (height as f64 * scale).round() as u32,
                false,
            )?;
            clamp(&w)?;
            save_geometry(&w)?;
        }
        Ok(())
    })();
    if result.is_err() {
        app.state::<Core>()
            .store
            .db
            .lock()
            .map_err(err)?
            .execute(
                "UPDATE widget_instances SET width=?1,height=?2 WHERE id=?3",
                rusqlite::params![before["width"].as_u64(), before["height"].as_u64(), id],
            )
            .map_err(err)?;
    }
    result
}
