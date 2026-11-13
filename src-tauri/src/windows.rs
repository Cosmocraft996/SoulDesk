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
    pub docked: bool,
    pub epoch: u64,
    pub dialog: bool,
    pub monitors: String,
    pub ready: std::collections::HashMap<String, std::time::Instant>,
    pub editing: bool,
    pub panel_manual: bool,
    pub interacting: bool,
    pub menus: std::collections::HashSet<String>,
    pub failed: std::collections::HashMap<String, String>,
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
    let surface = label.starts_with("float-") || label == "panel";
    let mut builder = WebviewWindowBuilder::new(app,label,WebviewUrl::App(url.into()))
        .title(if label=="main" {"SoulDesk · 组件管理"} else if label=="panel" {"SoulDesk · 组件面板"} else {"SoulDesk · 组件"})
        .inner_size(width,height).min_inner_size(120.,80.).visible(false).focused(false)
        .theme(Some(tauri::Theme::Dark)).initialization_script(host_script(app))
        .initialization_script_for_all_frames("if(window!==window.top)document.addEventListener('pointerdown',event=>{if(event.isTrusted&&event.button===0){const origin=new URLSearchParams(location.search).get('hostOrigin');if(origin)parent.postMessage({type:'souldesk:user-pointer',protocol:1},origin);}},true);document.addEventListener('focusin',event=>{if(window!==window.top&&event.isTrusted){const origin=new URLSearchParams(location.search).get('hostOrigin');if(origin)parent.postMessage({type:'souldesk:editing',protocol:1,editing:!!event.target.closest?.('input,textarea,select,[contenteditable=true]')},origin);}},true);document.addEventListener('focusout',()=>{if(window!==window.top){const origin=new URLSearchParams(location.search).get('hostOrigin');if(origin)parent.postMessage({type:'souldesk:editing',protocol:1,editing:false},origin);}},true);");
    // 隔离运行连 WebView 缓存也写入本次目录，不接触正常用户数据。
    let directory = app.state::<Core>().store.root.join("webview").join(label);
    std::fs::create_dir_all(&directory).map_err(err)?;
    builder = builder.data_directory(directory);
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
    let w = builder.build().map_err(err)?;
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
    format!("if(window.top===window&&{}.includes(location.origin)){{Object.defineProperty(window,'__SOULDESK_HOST_TOKEN__',{{value:{}}});}}",serde_json::to_string(&origins).unwrap(),serde_json::to_string(&core.host_token).unwrap())
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
pub(crate) fn hidden(app: &AppHandle) -> AppResult<bool> {
    Ok(app.state::<Core>().visibility.lock().map_err(err)?.hidden)
}
pub fn changed(app: &AppHandle) -> AppResult<()> {
    app.emit("host:changed", ()).map_err(err)
}

pub fn release(app: &AppHandle, label: &str) -> AppResult<()> {
    let core = app.state::<Core>();
    let mut sessions = core.sessions.0.lock().map_err(err)?;
    let removed = sessions
        .iter()
        .filter(|(_, s)| s.window == label)
        .map(|(t, _)| t.clone())
        .collect::<std::collections::HashSet<_>>();
    sessions.retain(|t, _| !removed.contains(t));
    core.business
        .subscriptions
        .lock()
        .map_err(err)?
        .retain(|_, s| !removed.contains(&s.owner));
    let mut state = core.visibility.lock().map_err(err)?;
    state
        .ready
        .retain(|k, _| !k.starts_with(&format!("{label}/")));
    state
        .failed
        .retain(|k, _| !k.starts_with(&format!("{label}/")));
    Ok(())
}
fn dispose(app: &AppHandle, label: &str) -> AppResult<()> {
    release(app, label)?;
    destroy_window(app, label)
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
    let key = format!("{label}/{id}");
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    loop {
        {
            let core = app.state::<Core>();
            let v = core.visibility.lock().map_err(err)?;
            if let Some(e) = v.failed.get(&key) {
                return Err(format!("LOAD_FAILED: {e}"));
            }
            if v.ready
                .get(&key)
                .is_some_and(|t| t.elapsed() >= Duration::from_millis(800))
            {
                return Ok(());
            }
        }
        if app.get_webview_window(label).is_none() {
            return Err("LOAD_FAILED: 目标窗口已关闭".into());
        }
        if std::time::Instant::now() >= deadline {
            return Err("LOAD_FAILED: 目标组件握手超时".into());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}
fn area(app: &AppHandle, at_pointer: bool) -> AppResult<crate::desktop_geometry::WorkArea> {
    let monitor = if at_pointer {
        let p = app.cursor_position().map_err(err)?;
        app.monitor_from_point(p.x, p.y).map_err(err)?
    } else {
        app.get_webview_window("panel")
            .and_then(|w| w.current_monitor().ok().flatten())
    };
    let m = monitor
        .or(app.primary_monitor().map_err(err)?)
        .ok_or("未找到可用显示器")?;
    let a = m.work_area();
    Ok(crate::desktop_geometry::WorkArea {
        left: a.position.x,
        top: a.position.y,
        width: a.size.width,
        height: a.size.height,
        scale_factor: m.scale_factor(),
    })
}
pub fn panel_preferences(app: &AppHandle) -> AppResult<serde_json::Value> {
    let raw = app.state::<Core>().store.preference("panel");
    let value = raw
        .map(|s| serde_json::from_str::<crate::panel::Preferences>(&s).map_err(err))
        .transpose()?
        .unwrap_or_default();
    value.validate()?;
    serde_json::to_value(value).map_err(err)
}
pub fn panel_metrics(app: &AppHandle) -> AppResult<serde_json::Value> {
    if let Some(w) = app.get_webview_window("panel") {
        let area = area(app, false)?;
        let r = native::describe(&w);
        return Ok(
            serde_json::json!({"effective_ratio":r["rect"]["width"].as_f64().unwrap_or(0.)/area.width as f64,"rect":r["rect"]}),
        );
    }
    Ok(serde_json::Value::Null)
}
pub fn save_panel(app: &AppHandle, ratio: f64, tint: f64) -> AppResult<()> {
    let mut settings: crate::panel::Preferences =
        serde_json::from_value(panel_preferences(app)?).map_err(err)?;
    settings.ratio = ratio;
    settings.tint = tint;
    save_panel_settings(app, settings)
}
pub fn save_panel_settings(app: &AppHandle, settings: crate::panel::Preferences) -> AppResult<()> {
    settings.validate()?;
    let previous = panel_preferences(app)?;
    app.state::<Core>()
        .store
        .save_preference("panel", &serde_json::to_string(&settings).map_err(err)?)?;
    if let Err(e) = layout_panel(app, false) {
        app.state::<Core>()
            .store
            .save_preference("panel", &previous.to_string())?;
        if let Err(restore) = layout_panel(app, false) {
            return Err(format!("{e}；恢复面板几何失败：{restore}"));
        }
        return Err(e);
    }
    #[cfg(windows)]
    crate::desktop_input::set_enabled(settings.desktop_double_click);
    changed(app)
}
fn layout_panel(app: &AppHandle, at_pointer: bool) -> AppResult<()> {
    if let Some(w) = app.get_webview_window("panel") {
        let a = area(app, at_pointer)?;
        let ratio = panel_preferences(app)?["ratio"].as_f64().unwrap();
        let min = instances(app)?
            .iter()
            .filter(|i| i["visible"] == true && i["location"] == "panel")
            .filter_map(|i| i["id"].as_str())
            .map(|id| dimensions(app, id).map(|(_, _, min, _)| min))
            .collect::<AppResult<Vec<_>>>()?
            .into_iter()
            .fold(280f64, f64::max);
        let g = crate::desktop_geometry::panel_geometry(a, ratio, min + 32.)?;
        native::place(&w, g.left, g.top, g.width, g.height, false)?;
        app.emit_to("panel", "host:panel-geometry", g)
            .map_err(err)?;
    }
    Ok(())
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
        show_panel(app, false)?;
        wait_ready(app, "panel", id)?;
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
pub fn show_panel(app: &AppHandle, focus: bool) -> AppResult<()> {
    if hidden(app)? {
        return Ok(());
    }
    let created = app.get_webview_window("panel").is_none();
    if created {
        build(app, "panel", "index.html?panel=1".into(), 480., 700.)?;
        let core = app.state::<Core>();
        let mut v = core.visibility.lock().map_err(err)?;
        v.editing = false;
        v.interacting = false;
        v.menus.clear();
        v.panel_manual = focus;
    } else if focus {
        app.state::<Core>()
            .visibility
            .lock()
            .map_err(err)?
            .panel_manual = true;
    }
    layout_panel(app, true)?;
    let w = app.get_webview_window("panel").ok_or("面板已关闭")?;
    if created {
        let duration = panel_preferences(app)?["slideInMs"].as_u64().unwrap_or(250) as u32;
        if let Err(e) = native::slide_panel(&w, true, duration) {
            app.state::<Core>().report(e);
        }
    } else {
        native::show(&w)?;
    }
    if focus {
        w.set_focus().map_err(err)?;
    }
    if let Some(edge) = app.get_webview_window("edge") {
        native::hide(&edge)?;
    }
    changed(app)
}
pub fn collapse(app: &AppHandle) -> AppResult<()> {
    if let Some(w) = app.get_webview_window("panel") {
        let duration = panel_preferences(app)?["slideOutMs"]
            .as_u64()
            .unwrap_or(250) as u32;
        if let Err(e) = native::slide_panel(&w, false, duration) {
            app.state::<Core>().report(e);
        }
    }
    dispose(app, "panel")?;
    if !hidden(app)? {
        native::show(&edge_window(app)?)?;
    }
    changed(app)
}
pub fn edge_window(app: &AppHandle) -> AppResult<WebviewWindow> {
    if let Some(w) = app.get_webview_window("edge") {
        return Ok(w);
    }
    let w = WebviewWindowBuilder::new(app, "edge", WebviewUrl::App("index.html?edge=1".into()))
        .title("SoulDesk · 侧边入口")
        .inner_size(12., 100.)
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
    position_edge(app, &w)?;
    Ok(w)
}
fn position_edge(app: &AppHandle, w: &WebviewWindow) -> AppResult<()> {
    let a = area(app, false)?;
    let width = (12. * a.scale_factor).round().max(1.) as u32;
    let height = (100. * a.scale_factor).round().min(a.height as f64) as u32;
    native::place(
        w,
        a.left + a.width as i32 - width as i32,
        a.top + (a.height as i32 - height as i32) / 2,
        width,
        height,
        false,
    )
}

pub fn hide_instance(app: &AppHandle, id: &str) -> AppResult<()> {
    let i = instance(app, id)?;
    app.state::<Core>().store.presentation(
        id,
        i["location"].as_str().unwrap(),
        false,
        i["pinned"] == true,
    )?;
    save_instance_geometry(app, id)?;
    dispose(app, &format!("float-{id}"))?;
    dispose(app, &format!("detail-{id}"))?;
    if i["location"] == "panel" {
        let open = app.get_webview_window("panel").is_some();
        dispose(app, "panel")?;
        if open {
            show_panel(app, false)?;
        }
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
    let panel_open = app.get_webview_window("panel").is_some();
    save_instance_geometry(app, id)?;
    dispose(app, &format!("float-{id}"))?;
    dispose(app, &format!("detail-{id}"))?;
    if old == "panel" || location == "panel" {
        dispose(app, "panel")?;
    }
    let result: AppResult<()> = (|| {
        app.state::<Core>()
            .store
            .presentation(id, location, true, before["pinned"] == true)?;
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
        if panel_open && location == "desktop" {
            show_panel(app, false)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        dispose(app, &format!("float-{id}"))?;
        dispose(app, "panel")?;
        app.state::<Core>().store.presentation(
            id,
            old,
            before["visible"] == true,
            before["pinned"] == true,
        )?;
        changed(app)?;
        let restore: AppResult<()> = (|| {
            if before["visible"] == true && (old == "desktop" || panel_open) {
                show_instance(app, id)?;
            }
            if panel_open {
                show_panel(app, false)?;
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
pub fn detail_at(app: &AppHandle, id: &str, page: &str) -> AppResult<()> {
    if !["quotes", "rules", "history"].contains(&page) {
        return Err("未知详情页".into());
    }
    if hidden(app)? || instance(app, id)?["visible"] != true {
        return Err("请先显示组件".into());
    }
    let label = format!("detail-{id}");
    let w = if let Some(w) = app.get_webview_window(&label) {
        w
    } else {
        build(
            app,
            &label,
            format!("index.html?detail={id}&page={page}"),
            820.,
            740.,
        )?
    };
    clamp(&w)?;
    w.show().map_err(err)?;
    w.set_focus().map_err(err)?;
    app.emit_to(&label, "host:detail-page", page).map_err(err)
}
pub fn hide_all(app: &AppHandle) -> AppResult<()> {
    save_all_geometry(app)?;
    app.state::<Core>()
        .store
        .save_preference("hidden_all", "true")?;
    {
        let core = app.state::<Core>();
        let mut v = core.visibility.lock().map_err(err)?;
        v.hidden = true;
        v.epoch += 1;
    }
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
        v.epoch += 1;
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
    native::show(&edge_window(app)?)?;
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
    if app.get_webview_window("panel").is_some() {
        collapse(app)
    } else {
        show_panel(app, true)
    }
}
pub fn auto_collapse(app: &AppHandle) {
    let epoch = {
        let core = app.state::<Core>();
        let Ok(mut v) = core.visibility.lock() else {
            return;
        };
        v.epoch += 1;
        v.epoch
    };
    dispatch(app, move |app| {
        {
            let core = app.state::<Core>();
            let v = core.visibility.lock().map_err(err)?;
            let detail = app
                .webview_windows()
                .keys()
                .any(|l| l.starts_with("detail-"));
            let inside = app
                .get_webview_window("panel")
                .is_some_and(|w| native::pointer_inside(&w).unwrap_or(true));
            if v.epoch != epoch
                || v.hidden
                || !crate::panel::can_auto_collapse(
                    v.panel_manual,
                    v.editing,
                    v.interacting,
                    !v.menus.is_empty(),
                    v.dialog,
                    detail,
                    inside,
                )
            {
                return Ok(());
            }
        }
        collapse(app)
    });
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
    layout_panel(app, false)?;
    for w in app.webview_windows().values() {
        if w.label() != "panel" {
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
        repair(app)?;
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
