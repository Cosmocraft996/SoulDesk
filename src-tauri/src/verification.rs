//! G0 的显式安装版探针。默认启动不会执行测试或修改实例。
use crate::{
    schedule_test,
    store::{err, now_ms, AppResult},
    widgets, windows, Core,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

#[tauri::command]
pub fn report_view(
    window: tauri::WebviewWindow,
    core: tauri::State<'_, Core>,
    value: Value,
) -> AppResult<()> {
    if core.verification.is_none() || value.to_string().len() > 16 * 1024 {
        return Err("验证报告不可用或超出大小".into());
    }
    core.view_reports
        .lock()
        .map_err(err)?
        .insert(window.label().into(), value);
    Ok(())
}

async fn capture_failure(app: &AppHandle, report: &mut Report) {
    let token = serde_json::to_string(&app.state::<Core>().host_token).unwrap();
    let script=format!("window.__TAURI_INTERNALS__?.invoke('report_view',{{value:{{href:location.href,title:document.title,ready:document.readyState,hostTokenPresent:typeof window.__SOULDESK_HOST_TOKEN__==='string',body:document.body?.innerText.slice(0,3000),frames:Array.from(document.querySelectorAll('iframe')).map(f=>f.src),resources:performance.getEntriesByType('resource').map(r=>r.name)}}}},{{headers:{{'x-souldesk-host':{token}}}}})");
    for window in app.webview_windows().values() {
        let _ = window.eval(&script);
    }
    tokio::time::sleep(Duration::from_millis(700)).await;
    let core = app.state::<Core>();
    report.value["diagnostics"] = json!({"views":core.view_reports.lock().ok().as_deref(),"resources":core.resource_reports.lock().ok().as_deref(),"snapshot":core.store.snapshot().ok(),"backendError":core.last_error.lock().ok().as_deref()});
}

static START_FOREGROUND: AtomicUsize = AtomicUsize::new(0);
pub fn capture_start() {
    START_FOREGROUND.store(foreground(), Ordering::Relaxed);
}

#[derive(Clone)]
pub struct Config {
    pub package: PathBuf,
    pub report: PathBuf,
    pub root: PathBuf,
    pub run_id: String,
    pub auto: bool,
    pub g1: bool,
    pub g2: bool,
    pub g3: bool,
    pub runtime: bool,
    pub soak_seconds: Option<u64>,
}
fn parse_soak(args: &[String]) -> AppResult<Option<u64>> {
    let values = args
        .iter()
        .enumerate()
        .filter(|(_, s)| s.as_str() == "--g0-soak")
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(None);
    }
    if values.len() != 1
        || !args.iter().any(|s| s == "--g0-g2")
        || !args.iter().any(|s| s == "--g0-runtime")
    {
        return Err("SOAK_ARGUMENT: 长期检查必须唯一指定并使用G2静默运行路径".into());
    }
    let seconds = args
        .get(values[0].0 + 1)
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|s| (120..=86400).contains(s))
        .ok_or("SOAK_ARGUMENT: 时长为120～86400秒；不足24小时只能作短检查")?;
    Ok(Some(seconds))
}

pub fn parse(args: &[String]) -> AppResult<Option<Config>> {
    let requested = args.iter().any(|a| a.starts_with("--g0-"));
    if !requested {
        return Ok(None);
    }
    if args.iter().any(|a| a == "--g0-g3")
        && (!args.iter().any(|a| a == "--g0-g2")
            || args
                .iter()
                .any(|a| matches!(a.as_str(), "--g0-g1" | "--g0-soak")))
    {
        return Err("G3必须使用G2组件入口，不能混合G1或长期采样".into());
    }
    let value = |flag: &str| -> AppResult<PathBuf> {
        let indices = args
            .iter()
            .enumerate()
            .filter(|(_, a)| a.as_str() == flag)
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        if indices.len() != 1 {
            return Err(format!("G0 参数 {flag} 必须且只能出现一次"));
        }
        let path = args
            .get(indices[0] + 1)
            .filter(|v| !v.starts_with("--"))
            .ok_or_else(|| format!("G0 参数 {flag} 缺少路径"))?;
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(format!("G0 参数 {flag} 必须是绝对路径"));
        }
        Ok(path)
    };
    let package = value("--g0-smoke")?;
    let report = value("--g0-report")?;
    let parent = report
        .parent()
        .filter(|p| p.is_dir())
        .ok_or("G0 报告父目录不存在")?;
    let root = parent.join(format!("g0-data-{}", uuid::Uuid::new_v4()));
    let run_id = match args.iter().position(|a| a == "--g0-run-id") {
        Some(i) => uuid::Uuid::parse_str(args.get(i + 1).ok_or("G0 运行标识缺失")?)
            .map_err(err)?
            .to_string(),
        None => uuid::Uuid::new_v4().to_string(),
    };
    Ok(Some(Config {
        soak_seconds: parse_soak(args)?,
        package,
        report,
        root,
        run_id,
        auto: args.iter().any(|a| a == "--g0-auto"),
        g1: args.iter().any(|a| a == "--g0-g1"),
        g2: args.iter().any(|a| a == "--g0-g2"),
        g3: args.iter().any(|a| a == "--g0-g3"),
        runtime: args.iter().any(|a| a == "--g0-runtime"),
    }))
}
#[cfg(windows)]
fn foreground() -> usize {
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() as usize }
}
#[cfg(not(windows))]
fn foreground() -> usize {
    0
}

struct Report {
    path: PathBuf,
    value: Value,
}
impl Report {
    fn save(&self) -> AppResult<()> {
        std::fs::write(
            &self.path,
            serde_json::to_vec_pretty(&self.value).map_err(err)?,
        )
        .map_err(err)
    }
    fn record(&mut self, name: &str, detail: Value) -> AppResult<()> {
        self.value["checks"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":name,"time":now_ms(),"detail":detail}));
        self.save()
    }
    fn stage(&mut self, name: &str) -> AppResult<()> {
        self.value["stage"] = json!(name);
        self.save()
    }
}

async fn wait_for(mut predicate: impl FnMut() -> bool, timeout: u64) -> AppResult<()> {
    let start = Instant::now();
    loop {
        if predicate() {
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(timeout) {
            return Err("探针等待超时".into());
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}
fn storage(app: &AppHandle, id: &str) -> Option<Value> {
    let core = app.state::<Core>();
    let db = core.store.db.lock().ok()?;
    let data: String = db
        .query_row(
            "SELECT storage FROM widget_instances WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .ok()?;
    serde_json::from_str(&data).ok()
}

async fn check(
    app: &AppHandle,
    package: PathBuf,
    report: &mut Report,
    start_foreground: usize,
    auto: bool,
) -> AppResult<()> {
    if app
        .state::<Core>()
        .verification
        .as_ref()
        .is_some_and(|c| c.g2)
    {
        return check_g2(app, package, report, start_foreground).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    if !app.webview_windows().is_empty() || foreground() != start_foreground {
        return Err("后台启动创建视图或改变前台".into());
    }
    report.record("后台启动不创建视图且前台保持", json!(true))?;
    let exe = std::env::current_exe().map_err(err)?;
    let second_foreground = foreground();
    let mut second = std::process::Command::new(exe)
        .arg("--g0-second")
        .spawn()
        .map_err(err)?;
    wait_for(
        || app.state::<Core>().second_launches.load(Ordering::Relaxed) > 0,
        10,
    )
    .await?;
    let mut second_status = None;
    wait_for(
        || {
            second_status = second.try_wait().ok().flatten();
            second_status.is_some()
        },
        10,
    )
    .await?;
    if !second_status.is_some_and(|s| s.success())
        || !app.webview_windows().is_empty()
        || foreground() != second_foreground
    {
        return Err("二次启动意外显示窗口".into());
    }
    report.record("单实例转发且不激活", json!(true))?;
    let manifest = crate::packages::install(app, &package).await?;
    if app
        .state::<Core>()
        .verification
        .as_ref()
        .is_some_and(|c| c.runtime && !c.g1)
    {
        report.record("开发环境隐藏试加载与真实消息桥",json!({"package":manifest.id,"resources":app.state::<Core>().resource_reports.lock().map_err(err)?.clone()}))?;
        report.value["runtimeChecksComplete"] = json!(true);
        report.stage("开发试加载完成；不代表普通安装验收")?;
        return Ok(());
    }
    let a = widgets::create_instance(&app.state::<Core>().store, &manifest.id)?;
    let b = widgets::create_instance(&app.state::<Core>().store, &manifest.id)?;
    windows::show(app, true)?;
    wait_for(
        || {
            storage(app, &a).is_some_and(|v| v["moduleExecuted"] == true)
                && storage(app, &b).is_some_and(|v| v["moduleExecuted"] == true)
        },
        15,
    )
    .await?;
    for id in [&a, &b] {
        let s = storage(app, id).unwrap();
        if s["origin"] != "http://widget.localhost"
            || s["background"] != "rgb(27, 34, 28)"
            || s["denied"] != true
            || s["buttonSave"] != true
            || s["hostTokenAbsent"] != true
            || s["directHostResult"] == "意外成功"
            || s["savedTitle"] != format!("验证-{}", &id[..8])
        {
            return Err(format!("组件证据不符：{s}"));
        }
    }
    report.record(
        "外部包资源模块CSS来源消息桥与双实例配置",
        json!({"a":storage(app,&a),"b":storage(app,&b),"package":manifest.id,"path":package}),
    )?;
    let token = serde_json::to_string(&app.state::<Core>().host_token).unwrap();
    let gate_script=format!("(async()=>{{let denied=false;try{{await window.__TAURI_INTERNALS__.invoke('snapshot',{{}},{{headers:{{'x-souldesk-host':'invalid-g0-token'}}}});}}catch(error){{denied=String(error).includes('CAPABILITY_DENIED');}}await window.__TAURI_INTERNALS__.invoke('report_view',{{value:{{hostGateDenied:denied}}}},{{headers:{{'x-souldesk-host':{token}}}}});}})()");
    windows::main_window(app)?.eval(gate_script).map_err(err)?;
    wait_for(
        || {
            app.state::<Core>()
                .view_reports
                .lock()
                .ok()
                .is_some_and(|reports| {
                    reports
                        .get("main")
                        .is_some_and(|v| v["hostGateDenied"] == true)
                })
        },
        5,
    )
    .await?;
    report.record("宿主入口拒绝伪造令牌且组件不持有令牌", json!(true))?;
    windows::move_instance(app, &a, true)?;
    wait_for(
        || storage(app, &a).is_some_and(|v| v["titleBefore"] == format!("验证-{}", &a[..8])),
        15,
    )
    .await?;
    report.record("同一实例浮窗复用及保存后配置", storage(app, &a).unwrap())?;
    let floating = app
        .get_webview_window(&format!("float-{a}"))
        .ok_or("缺少浮窗")?;
    floating.close().map_err(err)?;
    wait_for(
        || app.get_webview_window(&format!("float-{a}")).is_none(),
        5,
    )
    .await?;
    if app.state::<Core>().store.snapshot()?["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == a)
        .is_none_or(|i| i["floating"] != false)
    {
        return Err("浮窗关闭未归位".into());
    }
    report.record("浮窗原生关闭请求归位且保留实例配置", json!(true))?;
    windows::move_instance(app, &a, true)?;
    wait_for(
        || storage(app, &a).is_some_and(|v| v["moduleExecuted"] == true),
        5,
    )
    .await?;
    windows::hide_all(app)?;
    if app
        .webview_windows()
        .values()
        .any(|w| w.is_visible().unwrap_or(true))
    {
        return Err("隐藏全部仍有可见窗口".into());
    }
    let epoch = app.state::<Core>().visibility.lock().map_err(err)?.epoch;
    windows::delayed_collapse(app, false);
    tokio::time::sleep(Duration::from_secs(1)).await;
    if app
        .webview_windows()
        .values()
        .any(|w| w.is_visible().unwrap_or(true))
    {
        return Err("旧收起回调意外恢复视图".into());
    }
    report.record("隐藏全部含浮窗和入口且不意外唤出", json!({"epoch":epoch}))?;
    windows::toggle(app)?;
    if !app
        .get_webview_window(&format!("float-{a}"))
        .is_some_and(|w| w.is_visible().unwrap_or(false))
    {
        return Err("场景未恢复浮窗".into());
    }
    windows::move_instance(app, &a, false)?;
    windows::show(app, true)?;
    windows::main_window(app)?.close().map_err(err)?;
    wait_for(
        || {
            app.get_webview_window("main")
                .is_some_and(|w| !w.is_visible().unwrap_or(true))
        },
        5,
    )
    .await?;
    report.record(
        "主窗口原生关闭请求只隐藏",
        json!({"coreTicks":app.state::<Core>().ticks.load(Ordering::Relaxed)}),
    )?;
    windows::show(app, true)?;
    if !auto {
        report.stage("等待一次 Ctrl+Alt+Space 实际按键")?;
        wait_for(
            || app.state::<Core>().shortcut_hits.load(Ordering::Relaxed) > 0,
            180,
        )
        .await?;
        let trace = app
            .state::<Core>()
            .shortcut_trace
            .lock()
            .map_err(err)?
            .clone()
            .ok_or("快捷键缺少状态证据")?;
        if trace["beforeVisible"] == trace["afterVisible"] {
            return Err("快捷键没有切换场景".into());
        }
        report.record(
            "全局快捷键实际输入",
            json!({"hits":app.state::<Core>().shortcut_hits.load(Ordering::Relaxed),"trace":trace}),
        )?;
        windows::show(app, true)?;
        app.state::<Core>().visibility.lock().map_err(err)?.docked = true;
        windows::collapse(app)?;
        if !app
            .get_webview_window("edge")
            .is_some_and(|w| w.is_visible().unwrap_or(false))
        {
            return Err("侧边入口未显示".into());
        }
        report.stage("等待鼠标点击侧边入口")?;
        let edge_hits = app.state::<Core>().edge_hits.load(Ordering::Relaxed);
        wait_for(
            || {
                app.state::<Core>().edge_hits.load(Ordering::Relaxed) > edge_hits
                    && app
                        .get_webview_window("main")
                        .is_some_and(|w| w.is_visible().unwrap_or(false))
            },
            180,
        )
        .await?;
        report.record("侧边入口实际鼠标展开", json!(true))?;
        report.stage("等待鼠标移离工作台自动收起")?;
        wait_for(
            || {
                app.get_webview_window("edge")
                    .is_some_and(|w| w.is_visible().unwrap_or(false))
            },
            180,
        )
        .await?;
        report.record("鼠标离开后实际延迟收起", json!(true))?;
    }
    app.state::<Core>().visibility.lock().map_err(err)?.docked = false;
    windows::show(app, true)?;
    let main = windows::main_window(app)?;
    let monitor = main
        .current_monitor()
        .map_err(err)?
        .or(main.primary_monitor().map_err(err)?)
        .ok_or("缺少可用显示器")?;
    let area = monitor.work_area();
    main.set_size(tauri::PhysicalSize::new(
        area.size.width + 200,
        area.size.height + 100,
    ))
    .map_err(err)?;
    main.set_position(tauri::PhysicalPosition::new(
        area.position.x + area.size.width as i32 + 500,
        area.position.y + area.size.height as i32 + 500,
    ))
    .map_err(err)?;
    windows::clamp(&main)?;
    let position = main.outer_position().map_err(err)?;
    let size = main.outer_size().map_err(err)?;
    if position.x < area.position.x
        || position.y < area.position.y
        || position.x as i64 + size.width as i64 > area.position.x as i64 + area.size.width as i64
        || position.y as i64 + size.height as i64 > area.position.y as i64 + area.size.height as i64
    {
        return Err("窗口修复后仍超出工作区".into());
    }
    report.record("越界和超大窗口实际修复",json!({"position":position,"outerSize":size,"workArea":area,"scaleFactor":monitor.scale_factor()}))?;
    if app
        .state::<Core>()
        .verification
        .as_ref()
        .is_some_and(|c| c.g1)
    {
        check_g1(app, report).await?;
    }
    if app
        .state::<Core>()
        .verification
        .as_ref()
        .is_some_and(|c| c.runtime)
    {
        windows::destroy_views(app)?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        let core = app.state::<Core>();
        report.record("开发运行最终计数",json!({"sessions":core.sessions.0.lock().map_err(err)?.len(),"views":app.webview_windows().len(),"windowTasks":core.window_tasks.load(Ordering::Relaxed),"trials":core.trials.lock().map_err(err)?.len()}))?;
        report.value["runtimeChecksComplete"] = json!(true);
        report.stage("开发G1运行检查完成；未测试通知声音，不代表安装验收")?;
        return Ok(());
    }
    let silent = schedule_test(app.clone(), false, false, 2).await?;
    let notification = schedule_test(app.clone(), true, false, 3).await?;
    let sound = schedule_test(app.clone(), false, true, 4).await?;
    let tick = app.state::<Core>().ticks.load(Ordering::Relaxed);
    windows::destroy_views(app)?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let before = foreground();
    wait_for(
        || app.state::<Core>().active_tasks.load(Ordering::Relaxed) == 0,
        12,
    )
    .await?;
    let core = app.state::<Core>();
    let snapshot = core.store.snapshot()?;
    for (id, want_notification, want_sound) in [
        (&silent, false, false),
        (&notification, true, false),
        (&sound, false, true),
    ] {
        let event = snapshot["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == *id)
            .ok_or("后台测试事件缺失")?;
        let notification_ok = if want_notification && cfg!(windows) {
            event["notification"] == "sent"
        } else if want_notification {
            event["notification"]
                .as_str()
                .is_some_and(|s| s == "unknown" || s.starts_with("unknown:"))
        } else {
            event["notification"] == "suppressed"
        };
        let sound_ok = event["sound"] == if want_sound { "sent" } else { "suppressed" };
        if !notification_ok || !sound_ok {
            return Err(format!("通知或声音组合失败：{event}"));
        }
    }
    if snapshot["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["viewCount"] != 0 || e["completed"].is_null())
        || !app.webview_windows().is_empty()
        || core.ticks.load(Ordering::Relaxed) <= tick
        || foreground() != before
        || !core.sessions.0.lock().map_err(err)?.is_empty()
    {
        return Err("无视图后台、计数或前台保持检查失败".into());
    }
    report.record("零视图后台执行与前台保持", snapshot["events"].clone())?;
    report.record("最终计数",json!({"views":0,"sessions":core.sessions.0.lock().map_err(err)?.len(),"pendingTasks":core.active_tasks.load(Ordering::Relaxed),"ticks":core.ticks.load(Ordering::Relaxed)}))?;
    report.value["programChecksComplete"] = json!(true);
    report.value["g0Passed"] = json!(false);
    report.stage(if auto {
        "自动检查完成，人工交互项仍待验证"
    } else {
        "程序检查完成，通知可见性与声音听感仍需人工确认"
    })?;
    Ok(())
}

async fn check_g2(
    app: &AppHandle,
    package: PathBuf,
    report: &mut Report,
    start_foreground: usize,
) -> AppResult<()> {
    use crate::{
        alert_store,
        alerts::{Direction, MarketKey, Mode, PriceKind, Rule},
        business,
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    if !app.webview_windows().is_empty() || foreground() != start_foreground {
        return Err("G2后台启动改变视图或前台".into());
    }
    report.record("G2后台启动保持隐藏", json!(true))?;
    report.stage("等待真实目录并试加载外部行情包")?;
    wait_for(
        || {
            app.state::<Core>()
                .business
                .feed
                .state
                .lock()
                .is_ok_and(|s| !s.catalog.is_empty())
        },
        40,
    )
    .await?;
    let manifest = crate::packages::install(app, &package).await?;
    let a = widgets::create_instance(&app.state::<Core>().store, &manifest.id)?;
    let b = widgets::create_instance(&app.state::<Core>().store, &manifest.id)?;
    windows::show(app, true)?;
    report.stage("实际组件通过消息桥验证20合约、双实例、规则表单")?;
    wait_for(
        || {
            [&a, &b].iter().all(|id| {
                storage(app, id).is_some_and(|s| s["g2Ready"] == true || !s["g2Error"].is_null())
            })
        },
        125,
    )
    .await?;
    for id in [&a, &b] {
        let s = storage(app, id).ok_or("组件未保存证据")?;
        if s["g2Ready"] != true
            || s["observedKeys"] != 40
            || s["priceRows"] != 20
            || s["hostTokenAbsent"] != true
            || s["origin"] != "http://widget.localhost"
        {
            return Err(format!("G2组件证据不符：{s}"));
        }
    }
    report.record(
        "G2外部包真实行情双实例与表单保存",
        json!({"a":storage(app,&a),"b":storage(app,&b)}),
    )?;
    let expected = app.state::<Core>().business.feed.snapshot()?["subscriptions"]
        .as_u64()
        .ok_or("缺少订阅数")?;
    if expected != 62 {
        return Err(format!("20合约共享连接应为62个流，实际{expected}"));
    }
    let old_loads = storage(app, &a).unwrap()["loads"].as_u64().unwrap();
    windows::move_instance(app, &a, true)?;
    wait_for(
        || storage(app, &a).is_some_and(|s| s["loads"].as_u64().is_some_and(|n| n > old_loads)),
        110,
    )
    .await?;
    if app
        .state::<Core>()
        .verification
        .as_ref()
        .is_some_and(|c| c.g3)
    {
        return check_g3(app, report).await;
    }
    widgets::remove_instance(
        &app.state::<Core>().store,
        &app.state::<Core>().sessions,
        &b,
    )?;
    app.emit("host:changed", ()).map_err(err)?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    if business::watchlist(&app.state::<Core>().store, &a)?.len() != 20
        || app.state::<Core>().business.feed.snapshot()?["subscriptions"] != expected
    {
        return Err("移除一个实例影响其他消费者".into());
    }
    report.record(
        "G2工作台浮窗复用与移除单实例订阅隔离",
        json!({"streams":expected,"watchlist":20}),
    )?;
    windows::destroy_views(app)?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    if !app
        .state::<Core>()
        .sessions
        .0
        .lock()
        .map_err(err)?
        .is_empty()
        || !app
            .state::<Core>()
            .business
            .subscriptions
            .lock()
            .map_err(err)?
            .is_empty()
    {
        return Err("G2视图会话或临时订阅未回收".into());
    }
    let runtime = app
        .state::<Core>()
        .verification
        .as_ref()
        .is_some_and(|c| c.runtime);
    let mut ids = Vec::new();
    for (name, notification, sound) in [
        ("静默后台规则", false, false),
        ("后台仅通知", !runtime, false),
        ("后台仅声音", false, !runtime),
    ] {
        let rule = Rule {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            name: name.into(),
            key: MarketKey::binance("BTCUSDT", PriceKind::Mark),
            direction: Direction::Up,
            threshold: "0.00000001".into(),
            mode: Mode::Once,
            cooldown_ms: 1000,
            enabled: true,
            notification,
            sound,
        };
        alert_store::save_rule(&app.state::<Core>().store, &rule, None)?;
        ids.push(rule.id);
    }
    let before = foreground();
    report.stage(if runtime {
        "零视图真实行情驱动规则；开发模式不播放提醒"
    } else {
        "零视图真实行情驱动规则；请观察通知与声音"
    })?;
    wait_for(
        || {
            alert_store::list_events(&app.state::<Core>().store, 0, 100).is_ok_and(|events| {
                ids.iter().all(|id| {
                    events.iter().any(|e| {
                        e["ruleId"] == *id
                            && !matches!(e["notification"].as_str(), Some("pending" | "sending"))
                            && !matches!(e["sound"].as_str(), Some("pending" | "sending"))
                    })
                })
            })
        },
        20,
    )
    .await?;
    let events = alert_store::list_events(&app.state::<Core>().store, 0, 100)?
        .into_iter()
        .filter(|e| ids.iter().any(|id| e["ruleId"] == *id))
        .collect::<Vec<_>>();
    if events.len() != 3
        || events.iter().any(|e| {
            !matches!(e["notification"].as_str(), Some("sent" | "suppressed"))
                || !matches!(e["sound"].as_str(), Some("sent" | "suppressed"))
        })
        || !app.webview_windows().is_empty()
        || foreground() != before
    {
        return Err(format!("零视图提醒或前台检查失败：{events:?}"));
    }
    report.record(
        "G2零视图真实行情警报与独立递送",
        json!({"events":events,"views":0,"foregroundUnchanged":true,"actualDelivery":!runtime}),
    )?;
    report.stage("执行10次受控行情重连并核对单次事件不重播")?;
    let mut reconnects = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        let old = app
            .state::<Core>()
            .business
            .feed
            .generation
            .load(Ordering::SeqCst);
        app.state::<Core>()
            .business
            .feed
            .desired
            .send_modify(|d| d.reconnect += 1);
        wait_for(
            || {
                app.state::<Core>().business.feed.snapshot().is_ok_and(|s| {
                    s["status"] == "connected"
                        && s["generation"].as_u64().is_some_and(|g| g > old)
                        && s["quotes"].as_array().is_some_and(|a| {
                            a.iter()
                                .filter(|q| {
                                    q["quality"] == "fresh" && q["key"]["priceKind"] == "mark"
                                })
                                .count()
                                == 20
                        })
                })
            },
            60,
        )
        .await?;
        reconnects.push(start.elapsed().as_millis() as u64);
    }
    let count = alert_store::list_events(&app.state::<Core>().store, 0, 100)?
        .into_iter()
        .filter(|e| ids.iter().any(|id| e["ruleId"] == *id))
        .count();
    if count != 3 {
        return Err("受控重连重复了已完成的单次事件".into());
    }
    report.record("G2十次受控重连不重播单次事件",json!({"durationsMs":reconnects,"eventCount":count,"scope":"主动取消重连；不等于实际断网或休眠"}))?;
    let backup = app.state::<Core>().store.backup()?;
    report.record(
        "G2当前数据库备份完整性与版本检查",
        json!({"backup":backup,"version":3}),
    )?;
    report.value["g2ChecksComplete"] = json!(true);
    report.value["programChecksComplete"] = json!(true);
    report.value["manualPending"] = json!(if runtime {
        vec![
            "普通桌面安装与实际通知声音",
            "实际断网10次",
            "系统休眠5次",
            "本轮30分钟稳定性（独立报告）",
        ]
    } else {
        vec![
            "通知可见性及声音听感",
            "实际断网10次",
            "系统休眠5次",
            "本轮30分钟稳定性（独立报告）",
        ]
    });
    report.stage(if runtime {
        "开发G2检查完成；不代表普通安装验收"
    } else {
        "程序检查完成；G2实际提醒及恢复场景待人工确认"
    })?;
    Ok(())
}

#[cfg(windows)]
fn shortcut_conflict(app: &AppHandle) -> AppResult<Value> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_SHIFT,
    };
    let core = app.state::<Core>();
    let old = core
        .store
        .preference("shortcut")
        .unwrap_or("Ctrl+Alt+Space".into());
    // 与调用线程绑定，整个作用域无await，确保在同一线程释放系统注册。
    struct Reservation;
    impl Drop for Reservation {
        fn drop(&mut self) {
            unsafe {
                UnregisterHotKey(std::ptr::null_mut(), 0x5344);
            }
        }
    }
    let key = [0x7a, 0x79, 0x78].into_iter().find(|key| unsafe { RegisterHotKey(std::ptr::null_mut(), 0x5344, MOD_CONTROL | MOD_ALT | MOD_SHIFT, *key) } != 0).ok_or("无法建立隔离快捷键冲突条件")?;
    let _reservation = Reservation;
    let candidate = format!("Ctrl+Alt+Shift+F{}", key - 0x70 + 1);
    let failure = crate::set_shortcut(app.clone(), candidate.clone())
        .err()
        .ok_or("已占用快捷键被错误接受")?;
    if !failure.contains("保留原键")
        || !app.global_shortcut().is_registered(old.as_str())
        || core
            .store
            .preference("shortcut")
            .unwrap_or("Ctrl+Alt+Space".into())
            != old
    {
        return Err("快捷键冲突未保留旧注册及配置".into());
    }
    Ok(
        json!({"old":old,"candidate":candidate,"error":failure,"oldRegistered":true,"preferenceUnchanged":true,"method":"Windows真实注册占用；非模拟返回值"}),
    )
}
#[cfg(not(windows))]
fn shortcut_conflict(_app: &AppHandle) -> AppResult<Value> {
    Err("快捷键冲突探针仅支持Windows".into())
}

async fn view_resources(
    app: &AppHandle,
    report: &mut Report,
    phase: &str,
    seconds: u64,
    views: usize,
    visible: usize,
    sessions: usize,
    known: &mut std::collections::BTreeMap<u32, u64>,
) -> AppResult<Value> {
    use std::io::Write;
    let path = report.path.with_extension(format!("{phase}.jsonl"));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(&path)
        .map_err(err)?;
    let start = Instant::now();
    let mut previous: Option<(u64, Vec<Value>)> = None;
    let (mut private, mut cpu, mut working) = (Vec::new(), Vec::new(), Vec::new());
    let mut count = 0;
    let mut identities = None;
    let mut churn = 0;
    loop {
        let core = app.state::<Core>();
        let elapsed = start.elapsed().as_millis() as u64;
        let current = crate::view_probe::processes(known)?;
        let ids = current
            .iter()
            .map(|p| (p["pid"].clone(), p["createdTicks"].clone()))
            .collect::<Vec<_>>();
        let changed = identities.as_ref().is_some_and(|old| old != &ids);
        if changed {
            churn += 1;
        }
        identities = Some(ids);
        let visible_now = app
            .webview_windows()
            .values()
            .filter(|w| w.is_visible().unwrap_or(false))
            .count();
        let sessions_now = core.sessions.0.lock().map_err(err)?.len();
        let feed = core.business.feed.snapshot()?;
        if app.webview_windows().len() != views
            || visible_now != visible
            || sessions_now != sessions
            || feed["subscriptions"] != 62
            || feed["queueOverflows"] != 0
            || (views > 0 && current.len() < 2)
        {
            return Err(format!("G3资源阶段{phase}的视图/会话/行情边界不符：views={},visible={visible_now},sessions={sessions_now},processes={},streams={}",app.webview_windows().len(), current.len(), feed["subscriptions"]));
        }
        let percent = previous
            .as_ref()
            .map(|(at, old)| crate::view_probe::cpu_percent(old, &current, elapsed - at))
            .transpose()?;
        if previous
            .as_ref()
            .is_some_and(|(at, _)| elapsed - at > 15000)
        {
            return Err("G3资源采样间隔超过15秒，需复核".into());
        }
        let bytes: u64 = current
            .iter()
            .map(|p| p["privateBytes"].as_u64().unwrap())
            .sum();
        let ws: u64 = current
            .iter()
            .map(|p| p["workingSetBytes"].as_u64().unwrap())
            .sum();
        let sample = json!({"time":now_ms(),"elapsedMs":elapsed,"phase":phase,"processes":current,"privateBytes":bytes,"workingSetBytesSum":ws,"cpuPercentOneCore":percent,"views":views,"visible":visible,"sessions":sessions_now,"streams":feed["subscriptions"],"queueDepth":feed["queueDepth"],"status":feed["status"]});
        writeln!(file, "{}", sample).map_err(err)?;
        file.flush().map_err(err)?;
        count += 1;
        if elapsed >= 15000 {
            private.push(bytes as f64);
            working.push(ws as f64);
            if !changed {
                if let Some(p) = percent {
                    cpu.push(p);
                }
            }
        }
        previous = Some((elapsed, current));
        if elapsed >= seconds * 1000 {
            break;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    let private = crate::view_probe::summary(&private)?;
    if private["growth"].as_f64().unwrap() > 64. * 1048576.
        && private["last"].as_f64().unwrap() > private["first"].as_f64().unwrap() * 1.25
    {
        return Err("G3进程树私有内存增长需要复核".into());
    }
    Ok(
        json!({"phase":phase,"seconds":start.elapsed().as_secs(),"samples":count,"warmupSeconds":15,"privateBytes":private,"workingSetBytesSum":crate::view_probe::summary(&working)?,"cpuPercentOneCore":crate::view_probe::summary(&cpu)?,"processSetChanges":churn,"raw":path,"views":views,"visible":visible,"sessions":sessions,"scope":"本次宿主和创建时间校验后的WebView2后代；工作集求和可能重复计算共享页；CPU仅统计相邻样本同身份进程，进程创建/退出区间不推算"}),
    )
}

async fn check_g3(app: &AppHandle, report: &mut Report) -> AppResult<()> {
    // 只为显式资源探针保持双窗负载；正常失焦隐藏另有既有验收，不在此阶段触发。
    struct HoldViews(AppHandle, bool);
    impl Drop for HoldViews {
        fn drop(&mut self) {
            if let Ok(mut visibility) = self.0.state::<Core>().visibility.lock() {
                visibility.dialog = self.1;
            }
        }
    }
    let previous_dialog = {
        let core = app.state::<Core>();
        let mut visibility = core.visibility.lock().map_err(err)?;
        let old = visibility.dialog;
        visibility.dialog = true;
        old
    };
    let _hold = HoldViews(app.clone(), previous_dialog);
    windows::show(app, false)?;
    report.stage("G3：真实快捷键冲突与本机屏幕外窗口找回")?;
    report.record("G3快捷键冲突保留旧键", shortcut_conflict(app)?)?;
    let main = windows::main_window(app)?;
    main.set_position(tauri::PhysicalPosition::new(-30000, -30000))
        .map_err(err)?;
    windows::clamp(&main)?;
    let area = main
        .current_monitor()
        .map_err(err)?
        .ok_or("找回窗口后没有显示器")?
        .work_area()
        .to_owned();
    let pos = main.outer_position().map_err(err)?;
    let size = main.outer_size().map_err(err)?;
    if pos.x < area.position.x
        || pos.y < area.position.y
        || pos.x + size.width as i32 > area.position.x + area.size.width as i32
        || pos.y + size.height as i32 > area.position.y + area.size.height as i32
    {
        return Err("屏幕外窗口未完整找回到工作区".into());
    }
    report.record("G3本机屏幕外窗口找回", json!({"position":pos,"outerSize":size,"workArea":area,"scale":main.scale_factor().map_err(err)?,"scope":"实际本机工作区；未修改系统缩放，不代表其他DPI或拔除副屏"}))?;
    let mut phases = Vec::new();
    let mut known = std::collections::BTreeMap::new();
    for (name, seconds) in [
        ("visible", 60),
        ("hidden", 30),
        ("destroyed", 30),
        ("reopened", 30),
    ] {
        if name == "hidden" {
            windows::hide_all(app)?;
        }
        if name == "destroyed" {
            windows::destroy_views(app)?;
        }
        if name == "reopened" {
            windows::show(app, false)?;
        }
        let expected = if name == "destroyed" { 0 } else { 2 };
        wait_for(
            || {
                app.state::<Core>()
                    .sessions
                    .0
                    .lock()
                    .is_ok_and(|s| s.len() == expected)
            },
            15,
        )
        .await?;
        report.stage(&format!("G3：{name}视图资源采样{seconds}秒，无需操作"))?;
        phases.push(
            view_resources(
                app,
                report,
                name,
                seconds,
                expected,
                if name == "visible" || name == "reopened" {
                    2
                } else {
                    0
                },
                expected,
                &mut known,
            )
            .await?,
        );
        report.value["resourcePhases"] = json!(phases);
        report.save()?;
    }
    let growth = phases[3]["privateBytes"]["median"].as_f64().unwrap()
        - phases[0]["privateBytes"]["median"].as_f64().unwrap();
    if growth > 64. * 1048576.
        && phases[3]["privateBytes"]["median"].as_f64().unwrap()
            > phases[0]["privateBytes"]["median"].as_f64().unwrap() * 1.25
    {
        return Err("视图重建后的进程树内存增长需要复核".into());
    }
    windows::destroy_views(app)?;
    wait_for(
        || {
            app.state::<Core>()
                .sessions
                .0
                .lock()
                .is_ok_and(|s| s.is_empty())
                && app.state::<Core>().window_tasks.load(Ordering::Relaxed) == 0
                && app
                    .state::<Core>()
                    .business
                    .subscriptions
                    .lock()
                    .is_ok_and(|s| s.is_empty())
        },
        15,
    )
    .await?;
    report.record("G3视图进程树资源与回收", json!({"phases":phases,"reopenMedianGrowthBytes":growth,"finalViews":0,"finalSessions":0,"finalWindowTasks":0,"temporarySubscriptions":0}))?;
    report.value["g3ChecksComplete"] = json!(true);
    report.value["programChecksComplete"] = json!(true);
    report.value["manualPending"] = json!([
        "普通安装升级（安装入口及启动上下文）",
        "其他缩放、副屏与其他Windows设备（缺设备时单列）"
    ]);
    report.stage("程序检查完成；G3视图资源与快捷键冲突已有当次证据")?;
    Ok(())
}

async fn check_g1(app: &AppHandle, report: &mut Report) -> AppResult<()> {
    report.stage("G1：兼容升级与失败保护")?;
    let fixtures = report
        .path
        .parent()
        .ok_or("缺少验收产物目录")?
        .join("g1-fixtures");
    // 关闭验收页面，避免页面重建时的自动保存干扰失败升级前后的配置比较。
    windows::destroy_views(app)?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let front = foreground();
    let before = app.state::<Core>().store.snapshot()?;
    let old_settings = before["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["id"].clone(), i["settings"].clone()))
        .collect::<Vec<_>>();
    crate::packages::install(app, &fixtures.join("upgrade.zip")).await?;
    let upgraded = app.state::<Core>().store.snapshot()?;
    if upgraded["packages"][0]["version"] != "0.2.0" {
        return Err("升级未切换目标版本".into());
    }
    for (id, settings) in old_settings {
        let current = upgraded["instances"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == id)
            .ok_or("升级丢失实例")?;
        if current["settings"]["title"] != settings["title"]
            || current["settings"]["showSeconds"] != settings["showSeconds"]
            || current["settings"]["upgradeNote"] != "兼容升级"
        {
            return Err("升级未保留旧配置或补充兼容默认值".into());
        }
    }
    report.record(
        "G1兼容升级保留实例配置并补默认项",
        upgraded["instances"].clone(),
    )?;
    let mut failures = Vec::new();
    for file in [
        "runtime-failure.zip",
        "incompatible.zip",
        "unknown-capability.zip",
        "bad-path.zip",
    ] {
        let result = crate::packages::install(app, &fixtures.join(file)).await;
        let error = result
            .err()
            .ok_or_else(|| format!("坏包意外安装：{file}"))?;
        let expected = match file {
            "runtime-failure.zip" => "LOAD_FAILED:",
            "incompatible.zip" => "VERSION_INCOMPATIBLE:",
            "unknown-capability.zip" => "CAPABILITY_DENIED:",
            _ => "INVALID_ARGUMENT:",
        };
        if !error.starts_with(expected)
            || (file == "runtime-failure.zip" && !error.contains("G1 受控脚本失败"))
        {
            return Err(format!("故障包未命中预期边界：{file}：{error}"));
        }
        if app.state::<Core>().store.snapshot()? != upgraded {
            return Err(format!("失败升级改变了原登记或配置：{file}"));
        }
        if !app.state::<Core>().trials.lock().map_err(err)?.is_empty()
            || app
                .webview_windows()
                .keys()
                .any(|name| name.starts_with("trial-"))
        {
            return Err("失败试加载残留容器或试加载状态".into());
        }
        failures.push(json!({"file":file,"error":error}));
    }
    if foreground() != front {
        return Err("隐藏试加载激活了前台".into());
    }
    report.record(
        "G1脚本错误坏包未知能力及配置不兼容均保留旧版",
        json!(failures),
    )?;

    report.stage("G1：第二种独立组件、实例布局和窗口恢复")?;
    let card = report
        .path
        .parent()
        .unwrap()
        .join("local.example.card-0.1.0.zip");
    crate::packages::install(app, &card).await?;
    let a = widgets::create_instance(&app.state::<Core>().store, "local.example.card")?;
    let b = widgets::create_instance(&app.state::<Core>().store, "local.example.card")?;
    windows::show(app, true)?;
    app.emit("host:changed", ()).map_err(err)?;
    wait_for(
        || {
            [&a, &b].iter().all(|id| {
                storage(app, id).is_some_and(|value| {
                    value["moduleExecuted"] == true
                        && value["buttonSave"] == true
                        && value["standardsMode"] == "CSS1Compat"
                        && value["savedMessage"] == format!("实例-{}", &id[..8])
                })
            })
        },
        10,
    )
    .await?;
    widgets::set_instance_size(&app.state::<Core>().store, &a, 400, 220)?;
    let mut ids = app.state::<Core>().store.snapshot()?["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    ids.reverse();
    widgets::reorder_instances(&app.state::<Core>().store, &ids)?;
    let snapshot = app.state::<Core>().store.snapshot()?;
    if snapshot["instances"][0]["id"] != ids[0] {
        return Err("实例顺序未保存".into());
    }
    let resources = app
        .state::<Core>()
        .resource_reports
        .lock()
        .map_err(err)?
        .clone();
    if !resources.iter().any(|r| {
        r["uri"]
            .as_str()
            .is_some_and(|u| u.contains("local.example.card") && u.ends_with("card.js"))
            && r["status"] == 200
    }) {
        return Err("第二种组件未取得真实模块加载证据".into());
    }
    report.record("G1同一主程序导入第二种包双实例及布局保存",json!({"instances":snapshot["instances"],"resources":resources.iter().filter(|r|r["uri"].as_str().is_some_and(|u|u.contains("local.example.card"))).collect::<Vec<_>>()}))?;

    let main = windows::main_window(app)?;
    main.set_size(tauri::LogicalSize::new(820., 540.))
        .map_err(err)?;
    windows::save_geometry(&main)?;
    let saved = app
        .state::<Core>()
        .store
        .window_layout("main")?
        .ok_or("缺少主窗布局")?;
    windows::destroy_window(app, "main")?;
    let restored = windows::main_window(app)?;
    let scale = restored.scale_factor().map_err(err)?;
    let size = restored.inner_size().map_err(err)?;
    if (size.width as f64 / scale - saved["width"].as_f64().unwrap()).abs() > 2.0
        || (size.height as f64 / scale - saved["height"].as_f64().unwrap()).abs() > 2.0
    {
        return Err("主窗口重建未恢复保存的逻辑尺寸".into());
    }
    windows::move_instance(app, &a, true)?;
    let label = format!("float-{a}");
    let window = app.get_webview_window(&label).ok_or("缺少浮窗")?;
    window.set_always_on_top(true).map_err(err)?;
    windows::save_geometry(&window)?;
    windows::destroy_window(app, &label)?;
    windows::show(app, true)?;
    if !app
        .get_webview_window(&label)
        .is_some_and(|w| w.is_always_on_top().unwrap_or(false))
    {
        return Err("浮窗重建未恢复置顶及归属".into());
    }
    windows::move_instance(app, &a, false)?;
    report.record("G1实际窗口销毁重建恢复逻辑尺寸浮窗归属和置顶", saved)?;
    for _ in 0..12 {
        windows::move_instance(app, &a, true)?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        windows::move_instance(app, &a, false)?;
    }
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let core = app.state::<Core>();
    let sessions = core.sessions.0.lock().map_err(err)?.len();
    let instances = core.store.snapshot()?["instances"]
        .as_array()
        .unwrap()
        .len();
    if sessions > instances
        || !core.trials.lock().map_err(err)?.is_empty()
        || core.window_tasks.load(Ordering::Relaxed) != 0
    {
        return Err("反复切换后会话或试加载持续残留".into());
    }
    report.record("G1反复切换12次会话数量有界",json!({"sessions":sessions,"instances":instances,"trials":0,"windowTasks":core.window_tasks.load(Ordering::Relaxed),"views":app.webview_windows().len()}))?;
    report.value["g1ChecksComplete"] = json!(true);
    Ok(())
}

pub fn start(app: &AppHandle) {
    let Some(config) = app.state::<Core>().verification.clone() else {
        return;
    };
    let start_foreground = START_FOREGROUND.load(Ordering::Relaxed);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let executable = std::env::current_exe().ok();
        let executable_hash = executable
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
        let mut report = Report {
            path: config.report,
            value: json!({"version":handle.package_info().version.to_string(),"verificationProtocol":2,"phase":if config.runtime {"development-runtime"} else if config.g3 {"g3"} else if config.g2 {"g2"} else if config.g1 {"g1"} else {"g0"},"runId":config.run_id,"started":now_ms(),"executable":executable,"executableSha256":executable_hash,"dataRoot":handle.state::<Core>().store.root,"checks":[],"stage":"开始","g0Passed":false,"manualPending":if config.auto {vec!["全局快捷键实际输入","侧边鼠标展开与离开","托盘菜单交互","通知可见性及声音听感"]} else {vec!["托盘菜单交互","通知可见性及声音听感"]}}),
        };
        if let Err(e) = check(
            &handle,
            config.package,
            &mut report,
            start_foreground,
            config.auto,
        )
        .await
        {
            capture_failure(&handle, &mut report).await;
            report.value["failure"] = json!(e);
            let _ = report.stage("失败");
            handle.state::<Core>().report(e);
        }
        if report.value["failure"].is_null() {
            if let Some(seconds) = config.soak_seconds {
                if let Err(e) =
                    crate::soak::check(&handle, &report.path, &mut report.value, seconds).await
                {
                    report.value["failure"] = json!(e);
                    let _ = report.stage("长期检查失败；保留完整采样记录");
                }
            }
        }
        let _ = report.save();
        handle
            .state::<Core>()
            .quitting
            .store(true, Ordering::Relaxed);
        handle.exit(0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn soak_duration_is_bounded_and_requires_the_explicit_g2_runtime_route() {
        let parse_flags =
            |s: &[&str]| parse_soak(&s.iter().map(|v| v.to_string()).collect::<Vec<_>>());
        assert_eq!(parse_flags(&[]).unwrap(), None);
        for flags in [
            vec!["--g0-soak"],
            vec!["--g0-soak", "0", "--g0-g2", "--g0-runtime"],
            vec!["--g0-soak", "86401", "--g0-g2", "--g0-runtime"],
            vec!["--g0-soak", "86400"],
            vec![
                "--g0-soak",
                "120",
                "--g0-g2",
                "--g0-runtime",
                "--g0-soak",
                "120",
            ],
        ] {
            assert!(parse_flags(&flags).is_err());
        }
        assert_eq!(
            parse_flags(&["--g0-soak", "120", "--g0-g2", "--g0-runtime"]).unwrap(),
            Some(120)
        );
    }
    #[test]
    fn incomplete_probe_arguments_never_select_normal_data() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(parse(&args(&["souldesk", "--g0-smoke", "C:\\test.zip"])).is_err());
        assert!(parse(&args(&["souldesk", "--g0-report", "C:\\report.json"])).is_err());
        assert!(parse(&args(&["souldesk", "--g0-auto"])).is_err());
        assert!(parse(&args(&[
            "souldesk",
            "--g0-smoke",
            "--g0-report",
            "C:\\report.json"
        ]))
        .is_err());
        assert!(parse(&args(&["souldesk", "--show"])).unwrap().is_none());
    }
}
