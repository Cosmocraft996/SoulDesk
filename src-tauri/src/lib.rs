mod alert_store;
pub mod alerts;
mod business;
pub mod desktop_geometry;
mod desktop_probe;
mod desktop_widgets;
pub mod market;
pub mod network;
mod packages;
mod power;
mod presets;
mod recovery;
mod soak;
mod store;
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
async fn create_instance(app: AppHandle, package_id: String) -> AppResult<String> {
    packages::ensure_preset(&app, &package_id).await?;
    let id = widgets::create_instance(&app.state::<Core>().store, &package_id)?;
    app.emit("host:changed", ()).map_err(err)?;
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
    let core = app.state::<Core>();
    match action.as_str() {
        "resize" => widgets::set_instance_size(
            &core.store,
            &instance,
            width.ok_or("缺少宽度")?,
            height.ok_or("缺少高度")?,
        )?,
        "remove" => {
            widgets::remove_instance(&core.store, &core.sessions, &instance)?;
            if let Some(w) = app.get_webview_window(&format!("float-{instance}")) {
                w.destroy().map_err(err)?;
            }
        }
        _ => return Err("INVALID_ARGUMENT: 未知实例操作".into()),
    }
    app.emit("host:changed", ()).map_err(err)
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
    let mut value = widgets::open_session_expected(
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
    if method.starts_with("market.")
        || method.starts_with("watchlist.")
        || method.starts_with("alerts.")
    {
        return business::gateway(&app, window.label(), &token, &method, args).await;
    }
    widgets::gateway(
        &core.store,
        &core.sessions,
        window.label(),
        &token,
        &method,
        args,
    )
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
async fn window_action(app: AppHandle, action: String, instance: Option<String>) -> AppResult<()> {
    let core = app.state::<Core>();
    let _guard = core.window_actions.lock().map_err(err)?;
    match action.as_str() {
        "show" => windows::show(&app, true),
        "hide-all" => windows::hide_all(&app),
        "toggle" => windows::toggle(&app),
        "pin" => {
            let id = instance.ok_or("缺少浮窗实例")?;
            let window = app
                .get_webview_window(&format!("float-{id}"))
                .ok_or("浮窗不存在")?;
            window
                .set_always_on_top(!window.is_always_on_top().map_err(err)?)
                .map_err(err)?;
            windows::save_geometry(&window)
        }
        "dock" => {
            let mut v = core.visibility.lock().map_err(err)?;
            v.docked = !v.docked;
            let enabled = v.docked;
            drop(v);
            core.store
                .save_preference("docked", if enabled { "true" } else { "false" })?;
            if enabled {
                windows::collapse(&app)?;
            } else {
                windows::show(&app, true)?;
            }
            Ok(())
        }
        "edge-enter" => {
            if !core.visibility.lock().map_err(err)?.hidden {
                windows::show(&app, false)?;
                core.edge_hits.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        }
        "enter" => {
            core.visibility.lock().map_err(err)?.epoch += 1;
            Ok(())
        }
        "leave" => {
            windows::delayed_collapse(&app, false);
            Ok(())
        }
        "float" | "embed" => windows::move_instance(
            &app,
            instance.as_deref().ok_or("缺少实例")?,
            action == "float",
        ),
        "destroy" => windows::destroy_views(&app),
        "repair" => {
            for w in app.webview_windows().values() {
                windows::clamp(w)?;
            }
            Ok(())
        }
        _ => Err("未知窗口操作".into()),
    }
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
    let verification = verification::parse(&std::env::args().collect::<Vec<_>>())?;
    let root = match &verification {
        Some(config) => config.root.clone(),
        None => app.path().app_local_data_dir()?,
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
    let docked = store.preference("docked").as_deref() == Some("true");
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
            hidden: true,
            docked,
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
    });
    let show = MenuItem::with_id(app, "show", "显示工作台", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "隐藏全部", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置与诊断", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 SoulDesk", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &settings, &quit])?;
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
                    _ => Ok(()),
                });
            }
        })
        .build(app)?;
    if let Err(e) = app.global_shortcut().register(shortcut.as_str()) {
        app.state::<Core>()
            .report(format!("快捷键不可用：{e}；请从托盘更改"));
    }
    // 不在启动时创建可见窗口。此任务属于 Rust 核心进程。
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(1));
        loop {
            timer.tick().await;
            if handle.state::<Core>().quitting.load(Ordering::Relaxed) {
                break;
            }
            handle.state::<Core>().ticks.fetch_add(1, Ordering::Relaxed);
        }
    });
    if std::env::args().any(|a| a == "--show") {
        windows::show(app.handle(), true)?;
    }
    verification::start(app.handle());
    if app
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
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app,args,_| {app.state::<Core>().second_launches.fetch_add(1,Ordering::Relaxed);if args.iter().any(|a|a=="--show"){windows::dispatch(app,|app| windows::show(app,true));}}))
        .plugin(tauri_plugin_global_shortcut::Builder::new().with_handler(|app,_,event|{if event.state()==ShortcutState::Pressed {windows::dispatch(app,|app| {let before=app.webview_windows().iter().any(|(label,w)|label!="edge"&&w.is_visible().unwrap_or(false));windows::toggle(app)?;let after=app.webview_windows().iter().any(|(label,w)|label!="edge"&&w.is_visible().unwrap_or(false));*app.state::<Core>().shortcut_trace.lock().map_err(err)?=Some(json!({"beforeVisible":before,"afterVisible":after}));app.state::<Core>().shortcut_hits.fetch_add(1,Ordering::Relaxed);Ok(())});}}).build())
        .plugin(tauri_plugin_notification::init()).plugin(tauri_plugin_dialog::init())
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
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![snapshot,import_package,create_instance,manage_instance,reorder_instances,unregister_package,open_session,close_session,widget_request,window_action,set_shortcut,set_autostart,schedule_test,business_settings,verification::report_view,packages::trial_failed];
            handler(invoke)
        })
        .setup(setup)
        .on_window_event(|window,event|{
            let app=window.app_handle();
            match event {
                tauri::WindowEvent::Moved(_)|tauri::WindowEvent::Resized(_)=>windows::queue_geometry_save(window),
                tauri::WindowEvent::CloseRequested{api,..}=>{api.prevent_close();let label=window.label().to_owned();windows::dispatch(app,move|app|{if label=="main" {if let Some(window)=app.get_webview_window("main"){windows::save_geometry(&window)?;window.hide().map_err(err)?;}Ok(())}else if let Some(id)=label.strip_prefix("float-"){windows::move_instance(app,id,false)}else{Ok(())}});},
                tauri::WindowEvent::Focused(false) if window.label()=="main"=>windows::delayed_collapse(app,true),
                tauri::WindowEvent::Focused(true) if window.label()=="main"=>{let core=app.state::<Core>();let Ok(mut v)=core.visibility.lock() else{return;};v.epoch+=1;},
                tauri::WindowEvent::Destroyed=>{if let Some(core)=app.try_state::<Core>(){if let Ok(mut values)=core.geometry_versions.lock(){values.remove(window.label());}let Ok(mut sessions)=core.sessions.0.lock() else{return;};sessions.retain(|_,s|s.window!=window.label());}},
                _=>{}
            }
        })
        .build(tauri::generate_context!()).expect("SoulDesk 初始化失败；请保留原数据并查看诊断")
        .run(|app,event|{if let tauri::RunEvent::ExitRequested{api,..}=event {if !app.state::<Core>().quitting.load(Ordering::Relaxed){api.prevent_exit();}}});
}
