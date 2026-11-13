mod alert_store;
pub mod alerts;
mod business;
pub mod desktop_geometry;
#[cfg(windows)]
mod desktop_input;
#[cfg(windows)]
mod desktop_native;
mod desktop_probe;
mod desktop_widgets;
mod integration;
pub mod market;
pub mod network;
mod packages;
mod panel;
mod power;
mod presets;
mod recovery;
mod soak;
mod store;
mod updates;
mod verification;
mod view_probe;
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
        Mutex,
    },
    time::Duration,
};
use store::{err, now_ms, AppResult, Store};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
#[cfg(not(windows))]
use tauri_plugin_notification::NotificationExt;
use uuid::Uuid;

/// 仅供显式命令行恢复验收使用；不是组件网关能力。
pub async fn run_recovery_probe(report: std::path::PathBuf) -> Result<(), String> {
    recovery::run(report, false).await
}
pub async fn run_sleep_probe(report: std::path::PathBuf) -> Result<(), String> {
    recovery::run(report, true).await
}
pub struct Core {
    business: business::Service,
    store: Store,
    sessions: widgets::Sessions,
    visibility: Mutex<windows::Visibility>,
    window_actions: Mutex<()>,
    quitting: AtomicBool,
    ticks: AtomicU64,
    active_tasks: AtomicU64,
    last_error: Mutex<Option<String>>,
    verification: Option<verification::Config>,
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
    fn report(&self, error: String) {
        eprintln!("SoulDesk: {error}");
        if let Err(e) = self.store.log("error", &error) {
            eprintln!("诊断日志保存失败：{e}");
        }
        if let Ok(mut e) = self.last_error.lock() {
            *e = Some(error);
        }
    }
}

#[tauri::command]
async fn snapshot(app: AppHandle, core: State<'_, Core>) -> AppResult<Value> {
    let mut value = core.store.snapshot()?;
    value["packages"] = json!(presets::catalog(
        value["packages"].as_array().ok_or("组件目录无效")?
    )?);
    value["hiddenAll"] = json!(core.visibility.lock().map_err(err)?.hidden);
    value["panelOpen"] = json!(app.get_webview_window("panel").is_some());
    value["panel"] = windows::panel_preferences(&app)?;
    value["panelGeometry"] = windows::panel_metrics(&app)?;
    value["panelLayouts"] = core.store.panel_layouts()?;
    value["updates"] = updates::snapshot(&app)?;
    value["autostart"] = json!(windows::autostart_enabled().ok());
    value["windowTasks"] = json!(core.window_tasks.load(Ordering::Relaxed));
    value["trialCount"] = json!(core.trials.lock().map_err(err)?.len());
    value["diagnostics"] = json!({"ticks":core.ticks.load(Ordering::Relaxed),"tasks":core.active_tasks.load(Ordering::Relaxed),"sessions":core.sessions.0.lock().map_err(err)?.len(),"views":app.webview_windows().len(),"error":core.last_error.lock().map_err(err)?.clone(),"dataRoot":core.store.root,"shortcut":core.store.preference("shortcut").unwrap_or("Ctrl+Alt+Space".into()),"docked":core.visibility.lock().map_err(err)?.docked});
    Ok(value)
}
#[tauri::command]
async fn set_autostart(enabled: bool) -> AppResult<()> {
    windows::set_autostart(enabled)
}

#[tauri::command]
async fn import_package(app: AppHandle) -> AppResult<Value> {
    use tauri_plugin_dialog::DialogExt;
    {
        let core = app.state::<Core>();
        let mut v = core.visibility.lock().map_err(err)?;
        v.dialog = true;
        v.epoch += 1;
    }
    let picker = app
        .dialog()
        .file()
        .add_filter("SoulDesk 组件包", &["zip"])
        .blocking_pick_file();
    app.state::<Core>().visibility.lock().map_err(err)?.dialog = false;
    let Some(path) = picker else {
        return Ok(Value::Null);
    };
    let path = path.into_path().map_err(err)?;
    let result = packages::install(&app, &path).await?;
    app.emit("host:changed", ()).map_err(err)?;
    serde_json::to_value(result).map_err(err)
}

#[tauri::command]
async fn update_preset(app: AppHandle, package_id: String) -> AppResult<()> {
    packages::update_preset(&app, &package_id).await
}
#[tauri::command]
async fn create_instance(app: AppHandle, package_id: String) -> AppResult<String> {
    if app.state::<Core>().visibility.lock().map_err(err)?.hidden {
        return Err("请先恢复组件显示".into());
    }
    packages::ensure_preset(&app, &package_id).await?;
    let id = widgets::create_instance(&app.state::<Core>().store, &package_id)?;
    let target = id.clone();
    let shown = tauri::async_runtime::spawn_blocking(move || {
        windows::serialized(&app, |app| windows::show_instance(app, &target))
    })
    .await
    .map_err(err)?;
    if let Err(e) = shown {
        return Err(format!("实例已保留，可在管理中重试显示：{e}"));
    }
    Ok(id)
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
            let core = app.state::<Core>();
            match action.as_str() {
                "resize" => windows::resize_instance(
                    app,
                    &instance,
                    width.ok_or("缺少宽度")?,
                    height.ok_or("缺少高度")?,
                )?,
                "remove" => {
                    windows::hide_instance(app, &instance)?;
                    widgets::remove_instance(&core.store, &core.sessions, &instance)?;
                }
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
fn open_session(
    window: tauri::WebviewWindow,
    core: State<'_, Core>,
    instance: String,
    expected_digest: Option<String>,
) -> AppResult<Value> {
    if let Some(value) = packages::session(&core, window.label(), &instance)? {
        return Ok(value);
    }
    let mut value = widgets::open_display_session(
        &core.store,
        &core.sessions,
        window.label(),
        &instance,
        expected_digest.as_deref(),
    )?;
    value["verify"] = json!(core.verification.is_some());
    Ok(value)
}
#[tauri::command]
fn close_session(
    window: tauri::WebviewWindow,
    core: State<'_, Core>,
    token: String,
) -> AppResult<()> {
    let mut sessions = core.sessions.0.lock().map_err(err)?;
    if sessions
        .get(&token)
        .is_some_and(|s| s.window == window.label())
    {
        sessions.remove(&token);
    }
    Ok(())
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
    if method == "view.openDetails" {
        let id = {
            let sessions = core.sessions.0.lock().map_err(err)?;
            let s = sessions
                .get(&token)
                .filter(|s| s.window == window.label() && s.preview.is_none())
                .ok_or("SESSION_EXPIRED: 无有效展示会话")?;
            s.instance.clone()
        };
        let page = args["page"].as_str().unwrap_or("quotes").to_owned();
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            windows::serialized(&handle, |app| windows::detail_at(app, &id, &page))
        })
        .await
        .map_err(err)??;
        return Ok(Value::Null);
    }
    if method.starts_with("market.")
        || method.starts_with("watchlist.")
        || method.starts_with("alerts.")
    {
        return business::gateway(&app, window.label(), &token, &method, args).await;
    }
    let result = widgets::gateway(
        &core.store,
        &core.sessions,
        window.label(),
        &token,
        &method,
        args,
    );
    if method == "runtime.ready" && result.is_ok() {
        let id = core
            .sessions
            .0
            .lock()
            .map_err(err)?
            .get(&token)
            .map(|s| s.instance.clone());
        if let Some(id) = id {
            windows::ready(&app, window.label(), &id)?;
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
            json!({"config":core.store.preference("network").and_then(|v|serde_json::from_str::<Value>(&v).ok()).unwrap_or(json!({"mode":"system"})),"market":core.business.feed.snapshot()?}),
        ),
        "set" => {
            let config = config.ok_or("INVALID_ARGUMENT: 缺少网络配置")?;
            let network = business::resolve_network(&config)?;
            core.store.save_preference("network", &config.to_string())?;
            core.business.feed.desired.send_modify(|d| {
                d.network = network;
                d.reconnect += 1;
            });
            core.business.network_ready.store(true, Ordering::Release);
            core.business.feed.reconnect();
            Ok(Value::Null)
        }
        "reconnect" => {
            core.business.feed.reconnect();
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
) -> AppResult<()> {
    let label = window.label();
    let allowed = if label == "main" {
        true
    } else if label == "edge" {
        action == "edge-enter"
    } else if label.starts_with("detail-") {
        action == "close-detail"
    } else if label == "panel" {
        [
            "show",
            "settings",
            "collapse",
            "enter",
            "leave",
            "editing",
            "idle",
            "float",
            "hide",
            "detail",
            "interaction-start",
            "interaction-end",
            "menu-open",
            "menu-close",
        ]
        .contains(&action.as_str())
            && instance.as_ref().is_none_or(|id| {
                windows::instance(&app, id).is_ok_and(|i| i["location"] == "panel")
            })
    } else if let Some(id) = label.strip_prefix("float-") {
        ["show", "pin", "hide", "embed", "detail"].contains(&action.as_str())
            && instance.as_deref().is_none_or(|given| given == id)
    } else {
        false
    };
    if !allowed {
        return Err("CAPABILITY_DENIED: 当前窗口无此展示权限".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        windows::serialized(&app, |app| {
            let id = || instance.as_deref().ok_or("缺少实例".to_owned());
            match action.as_str() {
                "show" => windows::show(app, true),
                "settings" => windows::settings(app),
                "hide-all" => windows::hide_all(app),
                "restore-all" => windows::restore(app),
                "toggle" => windows::toggle(app),
                "collapse" => windows::collapse(app),
                "pin" => windows::pin(app, id()?),
                "hide" => windows::hide_instance(app, id()?),
                "restore" => windows::restore_instance(app, id()?),
                "float" => windows::move_to(app, id()?, "desktop"),
                "embed" => windows::move_to(app, id()?, "panel"),
                "detail" => windows::detail(app, id()?),
                "close-detail" => windows::destroy_window(app, window.label()),
                "edge-enter" => {
                    if !app.state::<Core>().visibility.lock().map_err(err)?.hidden {
                        windows::show_panel(app, false)?;
                        app.state::<Core>()
                            .edge_hits
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(())
                }
                "editing" | "idle" => {
                    let core = app.state::<Core>();
                    let mut v = core.visibility.lock().map_err(err)?;
                    v.editing = action == "editing";
                    v.epoch += 1;
                    drop(v);
                    if action == "idle" {
                        windows::auto_collapse(app);
                    }
                    Ok(())
                }
                "interaction-start" | "interaction-end" | "menu-open" | "menu-close" => {
                    let core = app.state::<Core>();
                    let mut v = core.visibility.lock().map_err(err)?;
                    let key = instance.clone().unwrap_or_else(|| "panel-settings".into());
                    match action.as_str() {
                        "interaction-start" => v.interacting = true,
                        "interaction-end" => v.interacting = false,
                        "menu-open" => {
                            v.menus.insert(key);
                        }
                        _ => {
                            v.menus.remove(&key);
                        }
                    }
                    v.epoch += 1;
                    drop(v);
                    if action == "interaction-end" || action == "menu-close" {
                        windows::auto_collapse(app);
                    }
                    Ok(())
                }
                "enter" => {
                    app.state::<Core>().visibility.lock().map_err(err)?.epoch += 1;
                    Ok(())
                }
                "leave" => {
                    windows::auto_collapse(app);
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
async fn set_panel(
    app: AppHandle,
    ratio: f64,
    tint: f64,
    slide_in_ms: Option<u32>,
    slide_out_ms: Option<u32>,
    desktop_double_click: Option<bool>,
) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        windows::serialized(&app, |app| {
            let mut settings: panel::Preferences =
                serde_json::from_value(windows::panel_preferences(app)?).map_err(err)?;
            settings.ratio = ratio;
            settings.tint = tint;
            if let Some(value) = slide_in_ms {
                settings.slide_in_ms = value;
            }
            if let Some(value) = slide_out_ms {
                settings.slide_out_ms = value;
            }
            if let Some(value) = desktop_double_click {
                settings.desktop_double_click = value;
            }
            windows::save_panel_settings(app, settings)
        })
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
    if label != "panel"
        && label != format!("float-{instance}")
        && label != format!("detail-{instance}")
    {
        return Err("CAPABILITY_DENIED: 无效展示来源".into());
    }
    windows::failed(&app, label, &instance, message.chars().take(1000).collect())
}

#[tauri::command]
fn set_shortcut(app: AppHandle, value: String) -> AppResult<()> {
    let core = app.state::<Core>();
    let old = core
        .store
        .preference("shortcut")
        .unwrap_or("Ctrl+Alt+Space".into());
    if old == value {
        return Ok(());
    }
    app.global_shortcut()
        .register(value.as_str())
        .map_err(|e| format!("快捷键注册失败，保留原键：{e}"))?;
    if let Err(e) = core.store.save_preference("shortcut", &value) {
        let _ = app.global_shortcut().unregister(value.as_str());
        return Err(e);
    }
    if let Err(e) = app.global_shortcut().unregister(old.as_str()) {
        core.report(format!("旧快捷键释放失败：{e}"));
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
    // 复用 Tauri 维护的同一 WinRT 实现，等待系统调用返回，避免插件吞掉发送错误。
    tauri_winrt_notification::Toast::new(&app.config().identifier)
        .title("SoulDesk · 后台测试")
        .text1("核心后台仍在执行；此事件与组件页面无关。")
        .sound(None)
        .show()
        .map_err(err)?;
    Ok("sent")
}

#[cfg(windows)]
fn alert_notification(app: &AppHandle, event: &alerts::AlertEvent) -> AppResult<()> {
    let kind = if event.key.price_kind == alerts::PriceKind::Last {
        "最新成交价 Last"
    } else {
        "标记价格 Mark"
    };
    let direction = if event.direction == alerts::Direction::Up {
        "达到或高于"
    } else {
        "达到或低于"
    };
    tauri_winrt_notification::Toast::new(&app.config().identifier)
        .title(&format!("SoulDesk · {}", event.rule_name))
        .text1(&format!("币安 USD-M {} · {kind}", event.key.symbol))
        .text2(&format!(
            "{} {direction} {}",
            event.observed_price, event.threshold
        ))
        .sound(None)
        .show()
        .map_err(err)
}
#[cfg(not(windows))]
fn alert_notification(_app: &AppHandle, _event: &alerts::AlertEvent) -> AppResult<()> {
    Err("当前平台未验证实际警报通知适配器".into())
}
#[cfg(not(windows))]
fn deliver_notification(app: &AppHandle) -> AppResult<&'static str> {
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
    let store = Store::open(root)?;
    let config = store
        .preference("network")
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or(json!({"mode":"system"}));
    let network = business::resolve_network(&config);
    let initial_error = network.as_ref().err().cloned();
    let network_ready = network.is_ok();
    let (feed, market_rx, market_tx, market_inputs) =
        market::Feed::new(network.unwrap_or(network::NetworkConfig::Direct));
    let hidden = store.preference("hidden_all").as_deref() == Some("true");
    let shortcut = store
        .preference("shortcut")
        .unwrap_or("Ctrl+Alt+Space".into());
    app.manage(Core {
        business: business::Service {
            feed,
            subscriptions: Default::default(),
            network_ready: AtomicBool::new(network_ready),
        },
        store,
        sessions: Default::default(),
        visibility: Mutex::new(windows::Visibility {
            hidden,
            ..Default::default()
        }),
        window_actions: Mutex::new(()),
        quitting: AtomicBool::new(false),
        ticks: AtomicU64::new(0),
        active_tasks: AtomicU64::new(0),
        last_error: Mutex::new(initial_error),
        verification,
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
    });
    let show = MenuItem::with_id(app, "show", "管理组件", true, None::<&str>)?;
    let restore = MenuItem::with_id(app, "restore", "恢复组件", true, None::<&str>)?;
    let panel = MenuItem::with_id(app, "panel", "打开组件面板", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "隐藏全部", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置与诊断", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 SoulDesk", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &panel, &restore, &hide, &settings, &quit])?;
    let mut pixels = vec![0u8; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let i = (y * 32 + x) * 4;
            let on = (8..24).contains(&x) && (7..12).contains(&y)
                || (8..13).contains(&x) && (7..25).contains(&y)
                || (8..24).contains(&x) && (20..25).contains(&y);
            pixels[i..i + 4].copy_from_slice(if on {
                &[204, 241, 126, 255]
            } else {
                &[22, 29, 26, 255]
            });
        }
    }
    TrayIconBuilder::new()
        .icon(tauri::image::Image::new_owned(pixels, 32, 32))
        .tooltip("SoulDesk · Ctrl+Alt+Space")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let action = event.id.as_ref().to_owned();
            if action == "quit" {
                windows::dispatch(app, |app| {
                    windows::save_all_geometry(app)?;
                    app.state::<Core>().quitting.store(true, Ordering::Relaxed);
                    app.exit(0);
                    Ok(())
                });
            } else {
                windows::dispatch(app, move |app| match action.as_str() {
                    "show" => windows::show(app, true),
                    "settings" => windows::settings(app),
                    "hide" => windows::hide_all(app),
                    "restore" => windows::restore(app),
                    "panel" => windows::toggle(app),
                    _ => Ok(()),
                });
            }
        })
        .build(app)?;
    if app
        .state::<Core>()
        .integration
        .as_ref()
        .is_none_or(|c| c.mode != "panel")
    {
        if let Err(e) = app.global_shortcut().register(shortcut.as_str()) {
            app.state::<Core>()
                .report(format!("快捷键不可用：{e}；请从托盘更改"));
        }
    }
    if app.state::<Core>().integration.is_none() && app.state::<Core>().verification.is_none() {
        #[cfg(windows)]
        match desktop_input::start(
            app.handle(),
            windows::panel_preferences(app.handle())?["desktopDoubleClick"]
                .as_bool()
                .unwrap_or(true),
        ) {
            Ok(listener) => {
                app.manage(listener);
            }
            Err(error) => app.state::<Core>().report(error),
        }
        updates::start(app.handle());
    }
    // 正常启动恢复桌面场景；显式隔离历史验收继续由其入口拥有。
    if app.state::<Core>().verification.is_none() && !hidden {
        windows::dispatch(app.handle(), windows::restore);
    }
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(1));
        loop {
            timer.tick().await;
            if handle.state::<Core>().quitting.load(Ordering::Relaxed) {
                break;
            }
            let tick = handle.state::<Core>().ticks.fetch_add(1, Ordering::Relaxed);
            if tick % 2 == 0 {
                windows::dispatch(&handle, windows::monitor_changes);
            }
        }
    });
    if std::env::args().any(|a| a == "--show") {
        windows::show(app.handle(), true)?;
    }
    verification::start(app.handle());
    integration::start(app.handle());
    if app
        .state::<Core>()
        .integration
        .as_ref()
        .is_none_or(|c| c.mode != "panel")
        && app
            .state::<Core>()
            .verification
            .as_ref()
            .is_none_or(|c| c.g2)
    {
        business::start(app.handle(), market_rx, market_tx, market_inputs);
    }
    Ok(())
}

pub fn run() {
    verification::capture_start();
    let panel_check = integration::parse(&std::env::args().collect::<Vec<_>>())
        .is_ok_and(|c| c.is_some_and(|c| c.mode == "panel"));
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
        }));
    }
    builder
        .plugin(tauri_plugin_global_shortcut::Builder::new().with_handler(|app,_,event|{if event.state()==ShortcutState::Pressed {windows::dispatch(app,|app| {let before=app.webview_windows().iter().any(|(label,w)|label!="edge"&&w.is_visible().unwrap_or(false));windows::toggle(app)?;let after=app.webview_windows().iter().any(|(label,w)|label!="edge"&&w.is_visible().unwrap_or(false));*app.state::<Core>().shortcut_trace.lock().map_err(err)?=Some(json!({"beforeVisible":before,"afterVisible":after}));app.state::<Core>().shortcut_hits.fetch_add(1,Ordering::Relaxed);Ok(())});}}).build())
        .plugin(tauri_plugin_notification::init()).plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .register_uri_scheme_protocol("widget",|ctx,request| {
            let result=widgets::resource(&ctx.app_handle().state::<Core>().store,request.uri().path());
            let (status,data,mime)=match result {Ok((b,m))=>(200,b,m),Err(_)=>(404,b"component resource unavailable".to_vec(),"text/plain".into())};
            let core=ctx.app_handle().state::<Core>();
            if core.verification.is_some() {if let Ok(mut reports)=core.resource_reports.lock(){if reports.len()<64 {reports.push(json!({"uri":request.uri().to_string(),"status":status,"mime":mime}));}}}
            tauri::http::Response::builder().status(status).header("Content-Type",mime).header("X-Content-Type-Options","nosniff")
                .header("Content-Security-Policy","default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'")
                .header("Cache-Control","no-store").body(data).unwrap()
        })
        .invoke_handler(|invoke| {
            let allowed=invoke.message.webview_ref().try_state::<Core>().is_some_and(|core|invoke.message.headers().get("x-souldesk-host").and_then(|h|h.to_str().ok())==Some(core.host_token.as_str()));
            if !allowed {invoke.resolver.reject("CAPABILITY_DENIED: 只允许宿主顶层容器调用");return true;}
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![snapshot,import_package,update_preset,create_instance,manage_instance,reorder_instances,unregister_package,open_session,close_session,widget_request,window_action,set_shortcut,set_autostart,schedule_test,business_settings,set_panel,save_panel_layout,app_update,surface_activate,surface_pointer,surface_failed,verification::report_view,packages::trial_failed];
            handler(invoke)
        })
        .setup(setup)
        .on_window_event(|window,event|{
            let app=window.app_handle();
            match event {
                tauri::WindowEvent::Moved(_)|tauri::WindowEvent::Resized(_)=>windows::queue_geometry_save(window),
                tauri::WindowEvent::CloseRequested{api,..}=>{api.prevent_close();let label=window.label().to_owned();windows::dispatch(app,move|app|{
                    if let Some(id)=label.strip_prefix("float-"){windows::hide_instance(app,id)}
                    else if label=="panel" {windows::collapse(app)}
                    else {if let Some(w)=app.get_webview_window(&label){windows::save_geometry(&w)?;}windows::release(app,&label)?;windows::destroy_window(app,&label)}
                });},
                tauri::WindowEvent::Focused(false) if window.label()=="panel"=>windows::auto_collapse(app),
                tauri::WindowEvent::Focused(true) if window.label()=="panel"=>{let core=app.state::<Core>();if let Ok(mut v)=core.visibility.lock(){v.epoch+=1;};},
                tauri::WindowEvent::Destroyed=>{if let Some(core)=app.try_state::<Core>(){if let Ok(mut values)=core.geometry_versions.lock(){values.remove(window.label());}if let Ok(mut sessions)=core.sessions.0.lock(){sessions.retain(|_,s|s.window!=window.label());}}},
                _=>{}
            }
        })
        .build(tauri::generate_context!()).expect("SoulDesk 初始化失败；请保留原数据并查看诊断")
        .run(|app,event|{if let tauri::RunEvent::ExitRequested{api,..}=event {if !app.state::<Core>().quitting.load(Ordering::Relaxed){api.prevent_exit();}}});
}
