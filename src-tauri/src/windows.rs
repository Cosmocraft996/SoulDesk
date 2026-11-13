use crate::{
    store::{err, AppResult},
    Core,
};
use std::time::Duration;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

#[derive(Default)]
pub struct Visibility {
    pub hidden: bool,
    pub scene: Vec<String>,
    pub docked: bool,
    pub epoch: u64,
    pub dialog: bool,
}

pub fn serialized(
    app: &AppHandle,
    action: impl FnOnce(&AppHandle) -> AppResult<()>,
) -> AppResult<()> {
    let core = app.state::<Core>();
    let _guard = core.window_actions.lock().map_err(err)?;
    action(app)
}

// WebView2 建窗必须离开同步 IPC 和系统事件线程；同一时刻只执行一个窗口动作。
pub fn dispatch(
    app: &AppHandle,
    action: impl FnOnce(&AppHandle) -> AppResult<()> + Send + 'static,
) {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = serialized(&handle, action) {
            handle.state::<Core>().report(error);
        }
    });
}

pub(crate) fn build(
    app: &AppHandle,
    label: &str,
    url: String,
    width: f64,
    height: f64,
) -> AppResult<WebviewWindow> {
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
        .title(if label == "main" {
            "SoulDesk · 工作台"
        } else {
            "SoulDesk · 组件"
        })
        .inner_size(width, height)
        .min_inner_size(360., 240.)
        .visible(false)
        .focused(false)
        .initialization_script(host_script(app))
        .build()
        .map_err(err)?;
    if label == "main" || label.starts_with("float-") {
        restore_geometry(&window)?;
    }
    Ok(window)
}

pub fn save_geometry(window: &WebviewWindow) -> AppResult<()> {
    if window.label() != "main" && !window.label().starts_with("float-") {
        return Ok(());
    }
    if window.is_minimized().map_err(err)? || window.is_maximized().map_err(err)? {
        return Ok(());
    }
    let scale = window.scale_factor().map_err(err)?;
    let position = window.outer_position().map_err(err)?;
    let size = window.inner_size().map_err(err)?;
    window.state::<Core>().store.save_window_layout(window.label(),&serde_json::json!({"x":position.x as f64/scale,"y":position.y as f64/scale,"width":size.width as f64/scale,"height":size.height as f64/scale,"scale":scale,"pinned":window.is_always_on_top().map_err(err)?}))
}

fn restore_geometry(window: &WebviewWindow) -> AppResult<()> {
    let Some(layout) = window.state::<Core>().store.window_layout(window.label())? else {
        return Ok(());
    };
    let number = |key: &str| {
        layout[key]
            .as_f64()
            .ok_or_else(|| format!("保存的窗口布局 {key} 无效"))
    };
    let width = number("width")?;
    let height = number("height")?;
    let scale = number("scale")?;
    if !(80.0..=16_384.0).contains(&width)
        || !(80.0..=16_384.0).contains(&height)
        || !(0.5..=8.0).contains(&scale)
    {
        return Err("保存的窗口尺寸无效".into());
    }
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(err)?;
    window
        .set_position(tauri::PhysicalPosition::new(
            (number("x")? * scale) as i32,
            (number("y")? * scale) as i32,
        ))
        .map_err(err)?;
    if window.label().starts_with("float-") {
        window
            .set_always_on_top(layout["pinned"].as_bool().unwrap_or(false))
            .map_err(err)?;
    }
    clamp(window)
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

fn float_window(app: &AppHandle, instance: &str) -> AppResult<WebviewWindow> {
    let label = format!("float-{instance}");
    let window = if let Some(window) = app.get_webview_window(&label) {
        window
    } else {
        build(
            app,
            &label,
            format!("index.html?instance={instance}"),
            420.,
            340.,
        )?
    };
    window
        .set_min_size(Some(LogicalSize::new(320., 200.)))
        .map_err(err)?;
    clamp(&window)?;
    Ok(window)
}

fn host_script(app: &AppHandle) -> String {
    let core = app.state::<Core>();
    let mut origins = vec![
        "http://tauri.localhost".to_owned(),
        "https://tauri.localhost".to_owned(),
        "tauri://localhost".to_owned(),
    ];
    if let Some(url) = &app.config().build.dev_url {
        origins.push(url.origin().ascii_serialization());
    }
    format!(
        "if(window.top===window&&{}.includes(location.origin)){{Object.defineProperty(window,'__SOULDESK_HOST_TOKEN__',{{value:{}}});}}",
        serde_json::to_string(&origins).unwrap(),serde_json::to_string(&core.host_token).unwrap()
    )
}

pub fn main_window(app: &AppHandle) -> AppResult<WebviewWindow> {
    if let Some(w) = app.get_webview_window("main") {
        return Ok(w);
    }
    let w = build(app, "main", "index.html".into(), 1060., 760.)?;
    clamp(&w)?;
    Ok(w)
}

pub fn settings(app: &AppHandle) -> AppResult<()> {
    if app.get_webview_window("main").is_none() {
        let window = build(app, "main", "index.html?tab=checks".into(), 1060., 760.)?;
        clamp(&window)?;
    }
    show(app, true)?;
    app.emit_to("main", "host:navigate", "checks")
        .map_err(err)?;
    Ok(())
}

pub fn edge_window(app: &AppHandle) -> AppResult<WebviewWindow> {
    if let Some(w) = app.get_webview_window("edge") {
        return Ok(w);
    }
    let w = WebviewWindowBuilder::new(app, "edge", WebviewUrl::App("index.html?edge=1".into()))
        .title("SoulDesk · 侧边入口")
        .inner_size(16., 120.)
        .decorations(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .visible(false)
        .focused(false)
        .initialization_script(host_script(app))
        .build()
        .map_err(err)?;
    if let Some(m) = w.primary_monitor().map_err(err)? {
        let a = m.work_area();
        let size = w.outer_size().map_err(err)?;
        w.set_position(tauri::PhysicalPosition::new(
            a.position.x + a.size.width as i32 - size.width as i32,
            a.position.y + (a.size.height as i32 - size.height as i32) / 2,
        ))
        .map_err(err)?;
    }
    Ok(w)
}

pub fn clamp(w: &WebviewWindow) -> AppResult<()> {
    if let Some(m) = w
        .current_monitor()
        .map_err(err)?
        .or(w.primary_monitor().map_err(err)?)
    {
        let area = m.work_area();
        let size = w.outer_size().map_err(err)?;
        let inner = w.inner_size().map_err(err)?;
        let p = w.outer_position().map_err(err)?;
        let width = size.width.min(area.size.width);
        let height = size.height.min(area.size.height);
        if width != size.width || height != size.height {
            w.set_size(tauri::PhysicalSize::new(
                width
                    .saturating_sub(size.width.saturating_sub(inner.width))
                    .max(1),
                height
                    .saturating_sub(size.height.saturating_sub(inner.height))
                    .max(1),
            ))
            .map_err(err)?;
        }
        let final_size = w.outer_size().map_err(err)?;
        w.set_position(tauri::PhysicalPosition::new(
            p.x.clamp(
                area.position.x,
                area.position.x + area.size.width.saturating_sub(final_size.width) as i32,
            ),
            p.y.clamp(
                area.position.y,
                area.position.y + area.size.height.saturating_sub(final_size.height) as i32,
            ),
        ))
        .map_err(err)?;
    }
    Ok(())
}

pub fn show(app: &AppHandle, focus: bool) -> AppResult<()> {
    {
        let core = app.state::<Core>();
        let mut v = core.visibility.lock().map_err(err)?;
        v.hidden = false;
        v.epoch += 1;
    }
    if let Some(edge) = app.get_webview_window("edge") {
        edge.hide().map_err(err)?;
    }
    let w = main_window(app)?;
    clamp(&w)?;
    if app.state::<Core>().visibility.lock().map_err(err)?.docked {
        if let Some(m) = w.current_monitor().map_err(err)? {
            let area = m.work_area();
            let size = w.outer_size().map_err(err)?;
            w.set_position(tauri::PhysicalPosition::new(
                area.position.x + area.size.width as i32 - size.width as i32,
                area.position.y,
            ))
            .map_err(err)?;
        }
    }
    w.show().map_err(err)?;
    let instances = app.state::<Core>().store.snapshot()?["instances"]
        .as_array()
        .unwrap()
        .clone();
    for instance in instances.iter().filter(|i| i["floating"] == true) {
        if let Some(id) = instance["id"].as_str() {
            float_window(app, id)?.show().map_err(err)?;
        }
    }
    if focus {
        w.set_focus().map_err(err)?;
    }
    Ok(())
}

pub fn hide_all(app: &AppHandle) -> AppResult<()> {
    save_all_geometry(app)?;
    let visible = app
        .webview_windows()
        .iter()
        .filter(|(label, w)| label.as_str() != "edge" && w.is_visible().unwrap_or(false))
        .map(|(l, _)| l.clone())
        .collect::<Vec<_>>();
    let core = app.state::<Core>();
    let mut v = core.visibility.lock().map_err(err)?;
    if !visible.is_empty() {
        v.scene = visible;
    }
    v.hidden = true;
    v.epoch += 1;
    drop(v);
    for w in app.webview_windows().values() {
        w.hide().map_err(err)?;
    }
    Ok(())
}

pub fn toggle(app: &AppHandle) -> AppResult<()> {
    let visible = app
        .webview_windows()
        .iter()
        .any(|(l, w)| l != "edge" && w.is_visible().unwrap_or(false));
    if visible {
        return hide_all(app);
    }
    let scene = {
        let core = app.state::<Core>();
        let mut v = core.visibility.lock().map_err(err)?;
        v.hidden = false;
        v.epoch += 1;
        v.scene.clone()
    };
    if let Some(w) = app.get_webview_window("edge") {
        w.hide().map_err(err)?;
    }
    let mut restored = false;
    for label in scene {
        if let Some(w) = app.get_webview_window(&label) {
            clamp(&w)?;
            w.show().map_err(err)?;
            w.set_focus().map_err(err)?;
            restored = true;
        }
    }
    if !restored {
        show(app, true)?;
    }
    Ok(())
}

pub fn collapse(app: &AppHandle) -> AppResult<()> {
    let core = app.state::<Core>();
    let v = core.visibility.lock().map_err(err)?;
    if v.hidden || !v.docked || v.dialog {
        return Ok(());
    }
    drop(v);
    if let Some(w) = app.get_webview_window("main") {
        w.hide().map_err(err)?;
    }
    edge_window(app)?.show().map_err(err)?;
    Ok(())
}

pub fn delayed_collapse(app: &AppHandle, focus_loss: bool) {
    let epoch = {
        let core = app.state::<Core>();
        let Ok(mut v) = core.visibility.lock() else {
            return;
        };
        v.epoch += 1;
        v.epoch
    };
    let handle = app.clone();
    handle
        .state::<Core>()
        .window_tasks
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(800)).await;
        dispatch(&handle, move |app| {
            let result = (|| -> AppResult<()> {
                let core = app.state::<Core>();
                let (valid, docked) = {
                    let v = core.visibility.lock().map_err(err)?;
                    (v.epoch == epoch && !v.hidden && !v.dialog, v.docked)
                };
                if valid {
                    if docked {
                        collapse(app)?;
                    } else if focus_loss {
                        if let Some(w) = app.get_webview_window("main") {
                            w.hide().map_err(err)?;
                        }
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

pub fn move_instance(app: &AppHandle, instance: &str, floating: bool) -> AppResult<()> {
    let core = app.state::<Core>();
    let previous = crate::widgets::set_floating(&core.store, &core.sessions, instance, floating)?;
    app.emit("host:changed", ()).map_err(err)?;
    let label = format!("float-{instance}");
    let result = (|| -> AppResult<()> {
        if floating {
            let w = float_window(app, instance)?;
            w.show().map_err(err)?;
            w.set_focus().map_err(err)?;
        } else if let Some(w) = app.get_webview_window(&label) {
            save_geometry(&w)?;
            destroy_window(app, &label)?;
        }
        Ok(())
    })();
    if result.is_err() {
        crate::widgets::set_floating(&core.store, &core.sessions, instance, previous)?;
        if floating && !previous {
            if let Some(w) = app.get_webview_window(&label) {
                let _ = w.destroy();
            }
        }
        app.emit("host:changed", ()).map_err(err)?;
    }
    result
}

pub fn destroy_views(app: &AppHandle) -> AppResult<()> {
    hide_all(app)?;
    let core = app.state::<Core>();
    core.sessions.0.lock().map_err(err)?.clear();
    for label in app.webview_windows().keys() {
        destroy_window(app, label)?;
    }
    Ok(())
}
