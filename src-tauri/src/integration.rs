//! 显式隔离验收入口：运行正式 Core、组件包与窗口协调器，不另建产品探针界面。
use crate::{
    alert_store,
    alerts::{self, MarketKey, PriceKind, Rule},
    business, presets,
    store::{err, now_ms, AppResult, Store},
    widgets, windows, Core,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
#[derive(Clone)]
pub struct Config {
    pub root: PathBuf,
    pub report: PathBuf,
    pub mode: String,
}
impl Config {
    pub fn panel_shortcut_enabled(&self) -> bool {
        // 人工验收需要真实开关面板；自动专项仍不占用正常程序的快捷键。
        !self.standalone_check() || self.mode == "widgetjs-manual"
    }

    pub fn standalone_check(&self) -> bool {
        matches!(
            self.mode.as_str(),
            "panel" | "widgetjs" | "widgetjs-resume" | "widgetjs-manual"
        )
    }
}
fn append_lifecycle(path: &std::path::Path, event: &Value) -> std::io::Result<()> {
    use std::io::Write;
    // 后台请求与主线程事件可能同时到达，一条记录必须完整写入。
    static WRITER: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = WRITER
        .lock()
        .map_err(|_| std::io::Error::other("退出记录锁不可用"))?;
    let mut line = serde_json::to_vec(event)?;
    line.push(b'\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(&line)
}

// 仅隔离检查记录退出阶段，不读取或导出正常用户库。
pub(crate) fn trace_lifecycle(app: &AppHandle, mut event: Value) {
    let core = app.state::<Core>();
    let Some(config) = core
        .integration
        .as_ref()
        .filter(|config| config.standalone_check())
    else {
        return;
    };
    event["at"] = json!(now_ms());
    event["mode"] = json!(config.mode);
    let mut path = config.report.as_os_str().to_owned();
    path.push(".lifecycle.jsonl");
    if let Err(error) = append_lifecycle(&PathBuf::from(path), &event) {
        eprintln!("隔离退出阶段记录失败：{error}");
    }
}
pub fn parse(args: &[String]) -> AppResult<Option<Config>> {
    let flags = args
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.starts_with("--desktop-integration") || s.as_str() == "--desktop-sandbox"
        })
        .collect::<Vec<_>>();
    if flags.is_empty() {
        return Ok(None);
    }
    if flags.len() != 1 || args.iter().any(|a| a.starts_with("--g0-")) {
        return Err("桌面隔离参数不能重复或混合历史验收".into());
    }
    let (at, flag) = flags[0];
    let mode = match flag.as_str() {
        "--desktop-integration" => "first",
        "--desktop-integration-resume" => "resume",
        "--desktop-integration-surface" => "surface",
        "--desktop-integration-panel" => "panel",
        "--desktop-integration-widgetjs" => "widgetjs",
        "--desktop-integration-widgetjs-resume" => "widgetjs-resume",
        "--desktop-integration-widgetjs-manual" => "widgetjs-manual",
        "--desktop-sandbox" => "manual",
        _ => return Err("未知桌面隔离参数".into()),
    };
    let path = PathBuf::from(
        args.get(at + 1)
            .filter(|p| !p.starts_with("--"))
            .ok_or("缺少绝对隔离路径")?,
    );
    if !path.is_absolute() {
        return Err("隔离路径必须为绝对路径".into());
    }
    let root = if mode == "manual" || mode == "widgetjs-manual" {
        path.clone()
    } else {
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            return Err("报告须为JSON".into());
        }
        path.with_extension("")
    };
    if !root
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.starts_with("desktop-integration-"))
    {
        return Err("只接受desktop-integration-开头的独立测试目录".into());
    }
    if mode == "first" || mode == "panel" || mode == "widgetjs" {
        if root.exists() || path.exists() {
            return Err("不覆盖已存在的报告或测试目录".into());
        }
    } else if !root.join("isolated-test-data.marker").is_file() {
        return Err("目录缺少隔离测试标识，拒绝访问".into());
    }
    Ok(Some(Config {
        root,
        report: path,
        mode: mode.into(),
    }))
}
fn rule(name: &str) -> Rule {
    Rule {
        id: uuid::Uuid::new_v4().to_string(),
        revision: 1,
        name: name.into(),
        key: MarketKey::binance("BTCUSDT", PriceKind::Mark),
        direction: alerts::Direction::Up,
        threshold: "0.00000001".into(),
        mode: alerts::Mode::Once,
        cooldown_ms: 0,
        enabled: true,
        notification: false,
        sound: false,
    }
}
pub fn prepare(c: &Config) -> AppResult<()> {
    if c.mode == "widgetjs" {
        std::fs::create_dir(&c.root).map_err(err)?;
        std::fs::write(
            c.root.join("isolated-test-data.marker"),
            b"SoulDesk isolated WidgetJS port checks",
        )
        .map_err(err)?;
        std::fs::write(c.root.join("seed.json"), b"{}").map_err(err)?;
        return Ok(());
    }
    if c.mode == "panel" {
        std::fs::create_dir(&c.root).map_err(err)?;
        std::fs::write(
            c.root.join("isolated-test-data.marker"),
            b"SoulDesk isolated panel check",
        )
        .map_err(err)?;
        let store = Store::open(c.root.clone())?;
        let staged = widgets::stage_bytes(
            &store,
            presets::find("local.example.clock")?
                .ok_or("缺少时钟")?
                .archive()?,
        )?;
        widgets::activate(&store, &staged)?;
        let id = widgets::create_instance(&store, "local.example.clock")?;
        std::fs::write(
            c.root.join("seed.json"),
            serde_json::to_vec(&json!({"clock":id})).map_err(err)?,
        )
        .map_err(err)?;
        return Ok(());
    }
    if c.mode != "first" {
        return Ok(());
    }
    std::fs::create_dir(&c.root).map_err(err)?;
    std::fs::write(
        c.root.join("isolated-test-data.marker"),
        b"SoulDesk isolated integration; never normal data",
    )
    .map_err(err)?;
    let store = Store::open(c.root.clone())?;
    let mut seed = json!({});
    for (name, key) in [
        ("local.example.clock", "clock"),
        ("local.souldesk.market", "market"),
    ] {
        let bytes = presets::find(name)?.ok_or("缺少预置包")?.archive()?;
        let staged = widgets::stage_bytes(&store, bytes)?;
        widgets::activate(&store, &staged)?;
        let id = widgets::create_instance(&store, name)?;
        seed[key] = json!(id);
    }
    {
        let db = store.db.lock().map_err(err)?;
        db.execute(
            "UPDATE widget_instances SET settings=?1,storage=?2 WHERE id=?3",
            rusqlite::params![
                r#"{"title":"迁移保留的时钟","showSeconds":true}"#,
                r#"{"note":"非空隔离配置"}"#,
                seed["clock"].as_str().unwrap()
            ],
        )
        .map_err(err)?;
    }
    business::save_watchlist(
        &store,
        seed["market"].as_str().unwrap(),
        &["BTCUSDT".into()],
    )?;
    let r = rule("迁移夹具：已处理单次规则");
    alert_store::save_rule(&store, &r, None)?;
    let t = now_ms();
    let quote = alerts::Observation {
        key: r.key.clone(),
        price: "100".into(),
        source_utc_ms: t,
        received_mono_ms: 0,
        generation: 1,
        ordinal: 1,
        recovered: false,
        valid: true,
    };
    alert_store::Engine::default().process(
        &store,
        &quote,
        alerts::Context {
            clock: alerts::Clock {
                utc_ms: t,
                mono_ms: 0,
            },
            generation: 1,
            source_now_utc_ms: t,
        },
    )?;
    seed["completedRule"] = json!(r.id);
    seed["seedEventIsSynthetic"] = json!(true);
    // 仅将刚创建的测试夹具还原成真实旧结构，用于随后正常 Store::open 的 v3→v4 迁移。
    {
        let db = store.db.lock().map_err(err)?;
        db.execute_batch("ALTER TABLE widget_instances DROP COLUMN location;ALTER TABLE widget_instances DROP COLUMN visible;ALTER TABLE widget_instances DROP COLUMN pinned;PRAGMA user_version=3;").map_err(err)?;
    }
    drop(store);
    std::fs::write(
        c.root.join("seed.json"),
        serde_json::to_vec_pretty(&seed).map_err(err)?,
    )
    .map_err(err)
}
fn save(c: &Config, value: &Value) -> AppResult<()> {
    std::fs::write(&c.report, serde_json::to_vec_pretty(value).map_err(err)?).map_err(err)
}
fn check(report: &mut Value, name: &str, passed: bool, details: Value) -> AppResult<()> {
    report["checks"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":name,"passed":passed,"details":details,"time":now_ms()}));
    if let Some(path) = report["reportPath"].as_str() {
        std::fs::write(path, serde_json::to_vec_pretty(report).map_err(err)?).map_err(err)?;
    }
    if passed {
        Ok(())
    } else {
        Err(format!("集成检查失败：{name}"))
    }
}
async fn window(
    app: &AppHandle,
    action: impl FnOnce(&AppHandle) -> AppResult<()> + Send + 'static,
) -> AppResult<()> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || windows::serialized(&handle, action))
        .await
        .map_err(err)?
}
async fn until(mut condition: impl FnMut() -> AppResult<bool>, seconds: u64) -> AppResult<()> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if condition()? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("正式集成条件等待超时".into());
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}
fn session_count(app: &AppHandle, id: &str) -> usize {
    app.state::<Core>()
        .sessions
        .0
        .lock()
        .unwrap()
        .values()
        .filter(|s| s.instance == id && s.preview.is_none())
        .count()
}
fn events(app: &AppHandle, id: &str) -> AppResult<usize> {
    Ok(
        alert_store::list_events(&app.state::<Core>().store, 0, 100)?
            .iter()
            .filter(|e| e["ruleId"] == id)
            .count(),
    )
}
async fn execute(app: &AppHandle, c: &Config, report: &mut Value) -> AppResult<()> {
    if c.mode == "widgetjs" || c.mode == "widgetjs-resume" {
        return execute_widgetjs(app, c, report).await;
    }
    let seed: Value =
        serde_json::from_slice(&std::fs::read(c.root.join("seed.json")).map_err(err)?)
            .map_err(err)?;
    let clock = seed["clock"].as_str().ok_or("缺少测试时钟")?.to_owned();
    if c.mode == "panel" {
        return execute_panel(app, c, report, clock).await;
    }
    let market = seed["market"].as_str().ok_or("缺少测试行情")?.to_owned();
    if c.mode == "surface" {
        until(|| Ok(app.get_webview_window("edge").is_some()), 15).await?;
        #[cfg(windows)]
        {
            let edge = app.get_webview_window("edge").ok_or("缺少侧边入口")?;
            let actual = crate::desktop_native::describe(&edge);
            let width = actual["rect"]["width"].as_u64().ok_or("入口尺寸无效")?;
            let height = actual["rect"]["height"].as_u64().ok_or("入口尺寸无效")?;
            let scale = edge.scale_factor().map_err(err)?;
            check(
                report,
                "edge-has-only-explicit-thin-hit-area",
                width == (12. * scale).round() as u64 && height == (100. * scale).round() as u64,
                json!({"actual":actual,"logicalWidth":12,"logicalHeight":100}),
            )?;
        }
        window(app, |app| {
            windows::show_panel(app, false)?;
            windows::show(app, false)
        })
        .await?;
        report["status"] = json!("automatic-complete-manual-observation-pending");
        save(c, report)?;
        return Ok(());
    }
    if c.mode == "first" {
        until(
            || Ok(session_count(app, &clock) == 1 && session_count(app, &market) == 1),
            20,
        )
        .await?;
        check(
            report,
            "startup-desktop-without-manager-or-panel",
            app.get_webview_window("main").is_none() && app.get_webview_window("panel").is_none(),
            json!({"clockSessions":session_count(app,&clock),"marketSessions":session_count(app,&market)}),
        )?;
        let backup = std::fs::read_dir(c.root.join("backups"))
            .map_err(err)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("before-v3-")
            })
            .ok_or("缺少v3迁移备份")?;
        let db = rusqlite::Connection::open_with_flags(
            &backup,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(err)?;
        let version: u32 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(err)?;
        let integrity: String = db
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(err)?;
        check(
            report,
            "nonempty-v3-migration-backup",
            version == 3
                && integrity == "ok"
                && windows::instance(app, &clock)?["settings"]["title"] == "迁移保留的时钟"
                && business::watchlist(&app.state::<Core>().store, &market)? == ["BTCUSDT"],
            json!({"backup":backup,"version":version,"integrity":integrity,"seedEventIsSynthetic":true}),
        )?;
        // 通过正常包试加载路径导入外部 ZIP，再用正常添加入口创建独立实例。
        let package = c.root.join("external-card.zip");
        std::fs::write(
            &package,
            presets::find("local.example.card")?.unwrap().archive()?,
        )
        .map_err(err)?;
        crate::packages::install(app, &package).await?;
        let card = crate::create_instance(app.clone(), "local.example.card".into()).await?;
        let second = crate::create_instance(app.clone(), "local.example.card".into()).await?;
        report["card"] = json!(card);
        report["secondCard"] = json!(second);
        check(
            report,
            "normal-zip-trial-and-direct-desktop-add",
            session_count(app, &card) == 1 && session_count(app, &second) == 1,
            json!({"card":card,"secondCard":second,"trials":app.state::<Core>().trials.lock().map_err(err)?.len()}),
        )?;
        let token = app
            .state::<Core>()
            .sessions
            .0
            .lock()
            .map_err(err)?
            .iter()
            .find(|(_, s)| s.instance == card)
            .map(|(t, _)| t.clone())
            .ok_or("卡片缺少会话")?;
        widgets::gateway(
            &app.state::<Core>().store,
            &app.state::<Core>().sessions,
            &format!("float-{card}"),
            &token,
            "settings.update",
            json!({"message":"正式集成保存，重启后保留"}),
        )?;
        let id = card.clone();
        window(app, move |app| windows::move_to(app, &id, "panel")).await?;
        let id = card.clone();
        window(app, move |app| windows::move_to(app, &id, "desktop")).await?;
        check(
            report,
            "desktop-panel-roundtrip-one-primary",
            session_count(app, &card) == 1
                && windows::instance(app, &card)?["settings"]["message"]
                    == "正式集成保存，重启后保留",
            Value::Null,
        )?;
        for target in ["panel", "desktop"] {
            if target == "desktop" {
                let id = card.clone();
                window(app, move |app| windows::move_to(app, &id, "panel")).await?;
            }
            let old = windows::instance(app, &card)?["location"].clone();
            *app.state::<Core>().transfer_failure.lock().map_err(err)? =
                Some(format!("{card}/{target}"));
            let id = card.clone();
            let target = target.to_owned();
            let result = window(app, move |app| windows::move_to(app, &id, &target)).await;
            check(
                report,
                "post-mounted-target-failure-rolls-back",
                result
                    .as_ref()
                    .err()
                    .is_some_and(|e| e.contains("受控失败"))
                    && windows::instance(app, &card)?["location"] == old
                    && session_count(app, &card) == 1,
                json!({"restoredLocation":old,"result":result.err()}),
            )?;
        }
        let id = card.clone();
        window(app, move |app| windows::move_to(app, &id, "desktop")).await?;
        let id = card.clone();
        window(app, move |app| windows::hide_instance(app, &id)).await?;
        check(
            report,
            "hide-releases-only-target",
            session_count(app, &card) == 0 && session_count(app, &second) == 1,
            Value::Null,
        )?;
        let id = card.clone();
        window(app, move |app| windows::restore_instance(app, &id)).await?;
        // 实际发出 WM_CLOSE，通过正式系统关闭处理器隐藏，再恢复。
        #[cfg(windows)]
        {
            let w = app
                .get_webview_window(&format!("float-{card}"))
                .ok_or("缺少组件窗口")?;
            let h = crate::desktop_native::handle(&w)?;
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    h as _,
                    windows_sys::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                    0,
                    0,
                );
            }
        }
        until(
            || {
                Ok(windows::instance(app, &card)?["visible"] == false
                    && session_count(app, &card) == 0)
            },
            8,
        )
        .await?;
        let id = card.clone();
        window(app, move |app| windows::restore_instance(app, &id)).await?;
        check(
            report,
            "system-close-hide-and-restore",
            session_count(app, &card) == 1,
            Value::Null,
        )?;
        window(app, |app| {
            windows::save_panel(app, 0.46, 0.2)?;
            windows::show(app, false)?;
            windows::collapse(app)?;
            windows::release(app, "main")?;
            windows::destroy_window(app, "main")
        })
        .await?;
        check(
            report,
            "management-close-panel-collapse-keep-desktop",
            session_count(app, &card) == 1
                && app.get_webview_window("main").is_none()
                && app.get_webview_window("panel").is_none(),
            Value::Null,
        )?;
        let id = clock.clone();
        window(app, move |app| {
            windows::pin(app, &id)?;
            windows::pin(app, &id)
        })
        .await?;
        check(
            report,
            "topmost-return-to-desktop",
            windows::instance(app, &clock)?["pinned"] == false,
            Value::Null,
        )?;
        // 真实网络验收：目录与20个自选来自当前币安公开源，绝不注入合成行情。
        until(
            || {
                Ok(!app
                    .state::<Core>()
                    .business
                    .feed
                    .state
                    .lock()
                    .map_err(err)?
                    .catalog
                    .is_empty())
            },
            70,
        )
        .await?;
        let catalog = app
            .state::<Core>()
            .business
            .feed
            .state
            .lock()
            .map_err(err)?
            .catalog
            .clone();
        let mut symbols = Vec::new();
        for s in [
            "BTCUSDT", "ETHUSDT", "SOLUSDT", "BNBUSDT", "XRPUSDT", "DOGEUSDT", "ADAUSDT",
            "LINKUSDT", "AVAXUSDT", "LTCUSDT", "BCHUSDT", "DOTUSDT", "SUIUSDT", "TRXUSDT",
            "NEARUSDT", "UNIUSDT", "AAVEUSDT", "ETCUSDT", "FILUSDT",
        ] {
            if catalog
                .iter()
                .any(|c| c.symbol == s && c.status == "TRADING")
            {
                symbols.push(s.to_owned());
            }
        }
        if let Some(c) = catalog
            .iter()
            .find(|c| c.contract_type == "TRADIFI_PERPETUAL" && c.status == "TRADING")
        {
            if !symbols.contains(&c.symbol) {
                symbols.push(c.symbol.clone());
            }
        }
        for c in catalog.iter().filter(|c| c.status == "TRADING") {
            if symbols.len() >= 20 {
                break;
            }
            if !symbols.contains(&c.symbol) {
                symbols.push(c.symbol.clone());
            }
        }
        symbols.truncate(20);
        business::save_watchlist(&app.state::<Core>().store, &market, &symbols)?;
        until(
            || {
                let snap = app.state::<Core>().business.feed.snapshot()?;
                Ok(symbols.iter().all(|s| {
                    ["last", "mark"].iter().all(|kind| {
                        snap["quotes"].as_array().unwrap().iter().any(|q| {
                            q["key"]["symbol"] == *s
                                && q["key"]["priceKind"] == *kind
                                && q["quality"] == "fresh"
                        })
                    })
                }))
            },
            70,
        )
        .await?;
        check(
            report,
            "real-20-contracts-last-and-mark",
            symbols.len() == 20,
            json!({"symbols":symbols,"feed":app.state::<Core>().business.feed.snapshot()?,"synthetic":false}),
        )?;
        let r = rule("真实行情：视图隐藏后单次到价");
        alert_store::save_rule(&app.state::<Core>().store, &r, None)?;
        report["liveRule"] = json!(r.id);
        window(app, windows::hide_all).await?;
        until(|| Ok(events(app, &r.id)? == 1), 45).await?;
        check(
            report,
            "real-alert-with-zero-views",
            app.webview_windows().is_empty()
                && app
                    .state::<Core>()
                    .sessions
                    .0
                    .lock()
                    .map_err(err)?
                    .is_empty()
                && events(app, &r.id)? == 1,
            json!({"notification":false,"sound":false,"events":alert_store::list_events(&app.state::<Core>().store,0,10)?}),
        )?;
        window(app, windows::restore).await?;
        window(app, windows::save_all_geometry).await?;
        report["status"] = json!("restart-ready");
        save(c, report)?;
        window(app, |app| {
            for label in app.webview_windows().keys() {
                windows::release(app, label)?;
                windows::destroy_window(app, label)?;
            }
            app.state::<Core>().business.feed.cancel.cancel();
            app.state::<Core>().quitting.store(true, Ordering::Relaxed);
            app.exit(0);
            Ok(())
        })
        .await?;
    } else {
        let card = report["card"].as_str().ok_or("缺少首轮卡片")?.to_owned();
        until(
            || {
                Ok(session_count(app, &card) == 1
                    && session_count(app, &clock) == 1
                    && session_count(app, &market) == 1)
            },
            20,
        )
        .await?;
        check(
            report,
            "restart-config-geometry-and-panel-preferences",
            windows::instance(app, &card)?["settings"]["message"] == "正式集成保存，重启后保留"
                && windows::panel_preferences(app)? == json!({"ratio":0.46,"tint":0.2})
                && app.get_webview_window("main").is_none()
                && app.get_webview_window("panel").is_none(),
            json!({"cardLayout":app.state::<Core>().store.window_layout(&format!("float-{card}"))?,"panel":windows::panel_preferences(app)?}),
        )?;
        let id = report["liveRule"]
            .as_str()
            .ok_or("缺少真实规则")?
            .to_owned();
        until(
            || {
                Ok(app.state::<Core>().business.feed.snapshot()?["quotes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|q| {
                        q["key"]["symbol"] == "BTCUSDT"
                            && q["key"]["priceKind"] == "mark"
                            && q["quality"] == "fresh"
                    }))
            },
            70,
        )
        .await?;
        check(
            report,
            "restart-fresh-price-without-alert-replay",
            events(app, &id)? == 1,
            json!({"eventCount":events(app,&id)?}),
        )?;
        let target = market.clone();
        window(app, move |app| windows::detail(app, &target)).await?;
        until(|| Ok(session_count(app, &market) == 2), 15).await?;
        check(
            report,
            "detail-role-keeps-primary",
            session_count(app, &market) == 2,
            Value::Null,
        )?;
        let target = market.clone();
        window(app, move |app| {
            windows::release(app, &format!("detail-{target}"))?;
            windows::destroy_window(app, &format!("detail-{target}"))?;
            windows::show_panel(app, false)?;
            windows::show(app, false)
        })
        .await?;
        report["status"] = json!("automatic-complete-manual-observation-pending");
        report["manual"] = json!({"desktopCoverageAndWinD":"未验证","naturalCrossApplicationTyping":"未验证","focusedAndUnfocusedAcrylic":"未验证","panelHeightAndTaskbar":"未验证","installedUpgrade":"未验证"});
        #[cfg(windows)]
        {
            report["windows"] = json!(app
                .webview_windows()
                .values()
                .map(crate::desktop_native::describe)
                .collect::<Vec<_>>());
        }
        report["normalDatabaseOpened"] = json!(false);
        save(c, report)?;
    }
    Ok(())
}
// 等待正式页面的指定按钮就绪，只发送一次事件，不注入系统鼠标输入。
async fn panel_ui_event(
    app: &AppHandle,
    label: &str,
    selector: &str,
    event: &str,
) -> AppResult<()> {
    let selector = serde_json::to_string(selector).map_err(err)?;
    let script = format!("(()=>{{const node=document.querySelector({selector});if(!node)return false;{event};return true;}})()");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (send, mut receive) = tokio::sync::mpsc::channel(1);
        app.get_webview_window(label)
            .ok_or("专项检查窗口未打开")?
            .eval_with_callback(&script, move |value| {
                let _ = send.try_send(value);
            })
            .map_err(err)?;
        let value = tokio::time::timeout(Duration::from_secs(3), receive.recv())
            .await
            .map_err(err)?
            .ok_or("专项检查页面没有返回结果")?;
        if value == "true" {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "专项检查按钮未就绪：{label}/{selector}，结果：{value}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
async fn execute_panel(
    app: &AppHandle,
    c: &Config,
    report: &mut Value,
    clock: String,
) -> AppResult<()> {
    until(|| Ok(session_count(app, &clock) == 1), 20).await?;
    // 页面事件专项不移动真实鼠标；用对话框保护隔离用户指针和外部焦点变化。
    app.state::<Core>().visibility.lock().map_err(err)?.dialog = true;
    let id = clock.clone();
    window(app, move |app| {
        if let Some(w) = app.get_webview_window(&format!("float-{id}")) {
            windows::save_geometry(&w)?;
        }
        let mut settings: crate::panel::Preferences =
            serde_json::from_value(windows::panel_preferences(app)?).map_err(err)?;
        settings.slide_in_ms = 0;
        settings.slide_out_ms = 0;
        windows::save_panel_settings(app, settings)
    })
    .await?;
    let original = app
        .state::<Core>()
        .store
        .window_layout(&format!("float-{clock}"))?;
    let id = clock.clone();
    window(app, move |app| windows::move_to(app, &id, "panel")).await?;
    check(
        report,
        "panel-zero-animation-loads-real-widget",
        session_count(app, &clock) == 1
            && app.get_webview_window(&format!("float-{clock}")).is_none(),
        json!({"clock":clock}),
    )?;
    let layout = crate::panel::Layout {
        x: 25.0,
        y: 45.0,
        width: 360.0,
        height: 220.0,
    };
    app.state::<Core>()
        .store
        .save_panel_layout(&clock, layout)?;
    windows::changed(app)?;
    let mut settings: crate::panel::Preferences =
        serde_json::from_value(windows::panel_preferences(app)?).map_err(err)?;
    settings.slide_in_ms = 150;
    settings.slide_out_ms = 150;
    window(app, move |app| windows::save_panel_settings(app, settings)).await?;
    let start = Instant::now();
    panel_ui_event(app, "panel", ".panel-edge-toggle", "const r=node.getBoundingClientRect();if(Math.abs(r.right-innerWidth)>1||Math.abs(r.top+r.height/2-innerHeight/2)>1)return false;node.click()").await?;
    until(
        || {
            Ok(app.get_webview_window("panel").is_none()
                && app
                    .get_webview_window("edge")
                    .is_some_and(|w| w.is_visible().unwrap_or(false)))
        },
        10,
    )
    .await?;
    let out_ms = start.elapsed().as_millis();
    check(
        report,
        "panel-slide-out-releases-its-session",
        app.get_webview_window("panel").is_none() && session_count(app, &clock) == 0,
        json!({"configuredMs":150,"operationMs":out_ms,"toggleAtRightCenter":true}),
    )?;
    let edge_hits = app.state::<Core>().edge_hits.load(Ordering::Relaxed);
    panel_ui_event(
        app,
        "edge",
        ".edge",
        "const r=node.getBoundingClientRect();if(Math.abs(r.right-innerWidth)>1||Math.abs(r.top+r.height/2-innerHeight/2)>1)return false;node.dispatchEvent(new MouseEvent('mouseover',{bubbles:true}))",
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    check(
        report,
        "edge-hover-does-not-open-panel",
        app.get_webview_window("panel").is_none()
            && app.state::<Core>().edge_hits.load(Ordering::Relaxed) == edge_hits,
        Value::Null,
    )?;
    let start = Instant::now();
    panel_ui_event(app, "edge", ".edge", "node.click()").await?;
    until(
        || {
            Ok(session_count(app, &clock) == 1
                && app.state::<Core>().edge_hits.load(Ordering::Relaxed) == edge_hits + 1)
        },
        15,
    )
    .await?;
    let in_ms = start.elapsed().as_millis();
    check(
        report,
        "panel-slide-in-restores-single-session-and-layout",
        app.state::<Core>().store.panel_layouts()?[&clock]
            == serde_json::to_value(layout).map_err(err)?
            && app.state::<Core>().edge_hits.load(Ordering::Relaxed) == edge_hits + 1
            && app.get_webview_window("panel").is_some(),
        json!({"configuredMs":150,"operationMs":in_ms,"layout":layout}),
    )?;
    windows::auto_collapse(app);
    tokio::time::sleep(Duration::from_millis(50)).await;
    check(
        report,
        "panel-dialog-protection-defers-auto-collapse",
        app.get_webview_window("panel").is_some(),
        json!({"dialogGuard":true,"realMouseOrFocusVerified":false}),
    )?;
    let settings = windows::instance(app, &clock)?["settings"].clone();
    panel_ui_event(app, "panel", ".surface-chrome", "if(node.querySelectorAll('.surface-toolbar>button').length!==4||node.querySelector('[aria-label=置顶]')||node.querySelector('[aria-label=管理组件]'))return false").await?;
    check(
        report,
        "panel-titlebar-exposes-transfer-hide-delete-and-more",
        true,
        Value::Null,
    )?;
    panel_ui_event(
        app,
        "panel",
        ".surface-chrome [aria-label='隐藏组件']",
        "node.click()",
    )
    .await?;
    until(
        || {
            Ok(session_count(app, &clock) == 0
                && windows::instance(app, &clock)?["visible"] == false
                && app.get_webview_window("panel").is_some())
        },
        10,
    )
    .await?;
    check(
        report,
        "panel-eye-hides-widget-and-keeps-config-and-panel-open",
        windows::instance(app, &clock)?["settings"] == settings
            && app.get_webview_window("panel").is_some(),
        Value::Null,
    )?;
    let id = clock.clone();
    window(app, move |app| windows::move_to(app, &id, "desktop")).await?;
    check(
        report,
        "panel-roundtrip-keeps-desktop-geometry-and-single-owner",
        app.state::<Core>()
            .store
            .window_layout(&format!("float-{clock}"))?
            == original
            && session_count(app, &clock) == 1,
        json!({"desktopGeometry":original,"panelLayout":layout}),
    )?;
    let label = format!("float-{clock}");
    panel_ui_event(app, &label, ".widget-surface", "const previous=node.style.width;node.style.width='120px';const bounds=node.getBoundingClientRect(),buttons=[...node.querySelectorAll('.surface-toolbar>button')],fits=buttons.length===5&&buttons.every(button=>{const r=button.getBoundingClientRect();return r.width>=16&&r.right<=bounds.right+1;});node.style.width=previous;if(!fits||node.querySelector('.surface-chrome [aria-label=管理组件]'))return false").await?;
    check(
        report,
        "desktop-titlebar-keeps-four-actions-and-more-at-minimum-width",
        true,
        Value::Null,
    )?;
    panel_ui_event(app, &label, "[aria-label='更多功能']", "node.click()").await?;
    panel_ui_event(app, &label, ".surface-menu [aria-label='管理组件']", "").await?;
    check(
        report,
        "more-menu-keeps-management-entry",
        true,
        Value::Null,
    )?;
    panel_ui_event(app, &label, "[aria-label='更多功能']", "node.click()").await?;
    panel_ui_event(
        app,
        &label,
        ".surface-chrome [aria-label='删除实例']",
        "node.click()",
    )
    .await?;
    panel_ui_event(
        app,
        &label,
        "[aria-label='删除实例确认']",
        "if(!node.open)return false",
    )
    .await?;
    check(
        report,
        "delete-opens-confirmation-without-changing-instance",
        session_count(app, &clock) == 1 && windows::instance(app, &clock)?["visible"] == true,
        Value::Null,
    )?;
    panel_ui_event(app, &label, "[data-action='cancel-remove']", "node.click()").await?;
    panel_ui_event(
        app,
        &label,
        ".widget-surface",
        "if(document.querySelector('[aria-label=删除实例确认]'))return false",
    )
    .await?;
    check(
        report,
        "cancel-delete-keeps-instance-and-settings",
        session_count(app, &clock) == 1 && windows::instance(app, &clock)?["settings"] == settings,
        Value::Null,
    )?;
    panel_ui_event(
        app,
        &label,
        ".surface-chrome [aria-label='删除实例']",
        "node.click()",
    )
    .await?;
    panel_ui_event(
        app,
        &label,
        "[data-action='confirm-remove']",
        "node.click()",
    )
    .await?;
    until(
        || {
            Ok(app.get_webview_window(&label).is_none()
                && session_count(app, &clock) == 0
                && windows::instance(app, &clock).is_err())
        },
        10,
    )
    .await?;
    check(
        report,
        "desktop-confirmed-delete-removes-instance-and-session",
        windows::instance(app, &clock).is_err(),
        Value::Null,
    )?;
    let panel_clock = widgets::create_instance(&app.state::<Core>().store, "local.example.clock")?;
    let id = panel_clock.clone();
    window(app, move |app| {
        windows::move_to(app, &id, "panel")?;
        windows::show_panel(app, true)
    })
    .await?;
    panel_ui_event(
        app,
        "panel",
        ".surface-chrome [aria-label='删除实例']",
        "node.click()",
    )
    .await?;
    panel_ui_event(
        app,
        "panel",
        "[data-action='confirm-remove']",
        "node.click()",
    )
    .await?;
    until(
        || {
            Ok(windows::instance(app, &panel_clock).is_err()
                && session_count(app, &panel_clock) == 0)
        },
        10,
    )
    .await?;
    check(
        report,
        "panel-confirmed-delete-removes-instance-and-keeps-panel-open",
        app.get_webview_window("panel").is_some(),
        Value::Null,
    )?;
    window(app, windows::destroy_views).await?;
    report["status"] = json!("passed");
    report["scope"] = json!("isolated-native-panel-lifecycle; no real mouse input, no installation, no updater installation");
    save(c, report)?;
    app.state::<Core>().quitting.store(true, Ordering::Relaxed);
    app.exit(0);
    Ok(())
}
fn widgetjs_storage(app: &AppHandle, id: &str) -> AppResult<Value> {
    // 管理快照有意不暴露实例存储，验收只读取本次隔离库的指定实例。
    let text: String = app
        .state::<Core>()
        .store
        .db
        .lock()
        .map_err(err)?
        .query_row(
            "SELECT storage FROM widget_instances WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .map_err(err)?;
    serde_json::from_str(&text).map_err(err)
}
fn widgetjs_saved_state(app: &AppHandle, id: &str) -> AppResult<Value> {
    let instance = windows::instance(app, id)?;
    let mut value = json!({"settings":instance["settings"]});
    if instance["packageId"] == "local.souldesk.widgetjs.note"
        || instance["packageId"] == "local.souldesk.widgetjs.wooden-fish"
        || instance["packageId"] == "local.souldesk.widgetjs.sticker"
        || instance["packageId"] == "local.souldesk.widgetjs.server"
    {
        let mut storage = widgetjs_storage(app, id)?;
        storage
            .as_object_mut()
            .ok_or("组件存储无效")?
            .remove("verification");
        value["storage"] = storage;
    }
    if instance["packageId"] == "local.souldesk.widgetjs.photo" {
        let grant: String = app
            .state::<Core>()
            .store
            .db
            .lock()
            .map_err(err)?
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                [format!("media_directory:{id}")],
                |row| row.get(0),
            )
            .map_err(err)?;
        value["mediaDirectory"] = serde_json::from_str::<Value>(&grant).map_err(err)?;
    }
    if instance["packageId"] == "local.souldesk.widgetjs.pomodoro" {
        // 番茄钟设置和场景属于核心，空通用设置不能证明实例相同或已恢复。
        let mut focus = crate::pomodoro::read(&app.state::<Core>().store, id, now_ms())?;
        focus
            .as_object_mut()
            .ok_or("番茄钟快照无效")?
            .remove("sampledAt");
        value["pomodoro"] = focus;
    }
    if instance["packageId"] == "local.souldesk.widgetjs.water-reminder" {
        value["water"] = widgetjs_water_data(&app.state::<Core>().store, id)?;
    }
    if instance["packageId"] == "local.souldesk.widgetjs.labor" {
        value["labor"] = widgetjs_labor_state(&app.state::<Core>().store, id)?;
    }
    Ok(value)
}
async fn execute_widgetjs(app: &AppHandle, c: &Config, report: &mut Value) -> AppResult<()> {
    if std::env::args().any(|arg| arg == "--widgetjs-check-package=convert-files") {
        return execute_widgetjs_convert_files(app, c, report).await;
    }
    if std::env::args().any(|arg| arg == "--widgetjs-check-package=preview-all") {
        return execute_widgetjs_previews(app, c, report).await;
    }
    let core = app.state::<Core>();
    let mut seed: Value =
        serde_json::from_slice(&std::fs::read(c.root.join("seed.json")).map_err(err)?)
            .map_err(err)?;
    let expected: Value = if c.mode == "widgetjs-resume" {
        serde_json::from_slice(&std::fs::read(c.root.join("expected.json")).map_err(err)?)
            .map_err(err)?
    } else {
        json!({})
    };
    if c.mode == "widgetjs" {
        let selected_package = std::env::args().find_map(|arg| {
            arg.strip_prefix("--widgetjs-check-package=")
                .map(str::to_owned)
        });
        if selected_package.as_ref().is_some_and(|name| {
            ![
                "time-progress",
                "wave-progress",
                "clock",
                "flip-clock",
                "glitch-clock",
                "mickey-clock",
                "text-clock",
                "trump-clock",
                "deadline",
                "calendar",
                "countdown",
                "countdown-list",
                "birthday-list",
                "todo",
                "calendar-large",
                "pomodoro",
                "note",
                "wooden-fish",
                "forex",
                "dashboard",
                "energy-label",
                "base-panel",
                "convert",
                "photo",
                "sticker",
                "water-reminder",
                "labor",
                "server",
                "hotspots",
                "coins",
            ]
            .contains(&name.as_str())
        }) {
            return Err("未提供该组件的受控表单检查".into());
        }
        for package in presets::catalog(&[])?
            .iter()
            .filter_map(|p| p["id"].as_str())
            .filter(|id| id.starts_with("local.souldesk.widgetjs."))
            .filter(|id| {
                selected_package.as_ref().is_none_or(|name| {
                    if name == "hotspots" {
                        return id.starts_with("local.souldesk.widgetjs.hotspot-");
                    }
                    if name == "coins" {
                        return [
                            "local.souldesk.widgetjs.coin-small",
                            "local.souldesk.widgetjs.coin-medium",
                        ]
                        .contains(id);
                    }
                    id.strip_prefix("local.souldesk.widgetjs.") == Some(name.as_str())
                })
            })
            // 默认仍为原8项；新增组件需显式选择，不扩大历史整批检查的范围。
            .filter(|id| {
                if id.starts_with("local.souldesk.widgetjs.hotspot-") {
                    return selected_package.as_deref() == Some("hotspots");
                }
                if [
                    "local.souldesk.widgetjs.coin-small",
                    "local.souldesk.widgetjs.coin-medium",
                ]
                .contains(id)
                {
                    return selected_package.as_deref() == Some("coins");
                }
                if [
                    "deadline",
                    "calendar",
                    "countdown",
                    "countdown-list",
                    "birthday-list",
                    "todo",
                    "calendar-large",
                    "pomodoro",
                    "note",
                    "wooden-fish",
                    "forex",
                    "dashboard",
                    "energy-label",
                    "base-panel",
                    "convert",
                    "photo",
                    "sticker",
                    "water-reminder",
                    "labor",
                    "server",
                ]
                .iter()
                .any(|name| id.strip_prefix("local.souldesk.widgetjs.") == Some(name))
                {
                    return id.strip_prefix("local.souldesk.widgetjs.")
                        == selected_package.as_deref();
                }
                [
                    "time-progress",
                    "wave-progress",
                    "clock",
                    "flip-clock",
                    "glitch-clock",
                    "mickey-clock",
                    "text-clock",
                    "trump-clock",
                ]
                .iter()
                .any(|name| id.strip_prefix("local.souldesk.widgetjs.") == Some(name))
            })
            .map(str::to_owned)
        {
            let mut ids = Vec::new();
            // 与管理界面的预置添加使用同一入口，包含真实隐藏WebView试加载。
            for _ in 0..2 {
                let id = crate::create_instance(app.clone(), package.clone()).await?;
                if package == "local.souldesk.widgetjs.photo" {
                    prepare_widgetjs_photo_fixture(&core.store, c, &id)?;
                }
                if package == "local.souldesk.widgetjs.server" {
                    let raw = std::env::args()
                        .find_map(|arg| {
                            arg.strip_prefix("--widgetjs-server-url=")
                                .map(str::to_owned)
                        })
                        .ok_or("缺少回环验收服务地址")?;
                    let url = url::Url::parse(&raw).map_err(err)?;
                    if url.scheme() != "http"
                        || url.host_str() != Some("127.0.0.1")
                        || url.port().is_none()
                        || url.path() != "/"
                        || !url.username().is_empty()
                        || url.password().is_some()
                        || url.query().is_some()
                        || url.fragment().is_some()
                    {
                        return Err("服务器验收只接受本次回环夹具".into());
                    }
                    let fixture = json!({"version":1,"servers":[],"master":{"url":"","token":""},"verification":{"fixtureUrl":raw}});
                    core.store
                        .db
                        .lock()
                        .map_err(err)?
                        .execute(
                            "UPDATE widget_instances SET storage=?1 WHERE id=?2",
                            rusqlite::params![fixture.to_string(), id],
                        )
                        .map_err(err)?;
                }
                ids.push(id);
            }
            seed[&package] = json!(ids);
        }
        std::fs::write(
            c.root.join("seed.json"),
            serde_json::to_vec_pretty(&seed).map_err(err)?,
        )
        .map_err(err)?;
        report["selectedPackages"] = json!(seed.as_object().unwrap().keys().collect::<Vec<_>>());
    }
    if c.mode == "widgetjs" {
        if let Some(ids) = seed.get("local.souldesk.widgetjs.coin-medium") {
            verify_widgetjs_coin_sharing(app, ids, report).await?;
        }
    }
    for (package, ids) in seed.as_object().ok_or("组件验收清单无效")? {
        let ids = ids.as_array().ok_or("组件实例清单无效")?;
        for id in ids {
            let id = id.as_str().ok_or("实例标识无效")?.to_owned();
            until(
                || Ok(widgetjs_storage(app, &id)?["verification"]["uiVerified"] == true),
                if package.ends_with(".coin-small") || package.ends_with(".coin-medium") {
                    45
                } else {
                    20
                },
            )
            .await?;
            let value = windows::instance(app, &id)?;
            let storage = widgetjs_storage(app, &id)?;
            let result = &storage["verification"];
            check(
                report,
                "ported-widget-real-ui-save-and-css",
                result["uiVerified"] == true
                    && result["stylesLoaded"] == true
                    && result["invalidSaveRejected"] == true
                    && (result["savedLabel"]
                        .as_str()
                        .is_some_and(|s| s.contains(&id[..8]))
                        || result["configurationSaved"] == true
                            && (result["imagesLoaded"] == true
                                || result["businessStateVerified"] == true)),
                json!({"package":package,"instance":id,"result":result,"settings":value["settings"]}),
            )?;
            if c.mode == "widgetjs-resume" {
                until(
                    || {
                        Ok(widgetjs_storage(app, &id)?["verification"]["runtimeLoads"]
                            .as_u64()
                            .unwrap_or(0)
                            > expected[&id]["runtimeLoads"].as_u64().unwrap_or(u64::MAX))
                    },
                    20,
                )
                .await?;
                check(
                    report,
                    "ported-widget-settings-retained-after-process-restart",
                    widgetjs_saved_state(app, &id)? == expected[&id]["savedState"],
                    json!({"package":package,"instance":id,"savedState":widgetjs_saved_state(app,&id)?,"expected":expected[&id]}),
                )?;
                continue;
            }
            let old_loads = result["runtimeLoads"].as_u64().unwrap_or(0);
            let saved_state = widgetjs_saved_state(app, &id)?;
            let moved = id.clone();
            window(app, move |app| windows::move_to(app, &moved, "panel")).await?;
            let moved = id.clone();
            window(app, move |app| windows::move_to(app, &moved, "desktop")).await?;
            until(
                || {
                    Ok(widgetjs_storage(app, &id)?["verification"]["runtimeLoads"]
                        .as_u64()
                        .unwrap_or(0)
                        > old_loads)
                },
                20,
            )
            .await?;
            check(
                report,
                "ported-widget-panel-roundtrip-retains-settings",
                session_count(app, &id) == 1 && widgetjs_saved_state(app, &id)? == saved_state,
                json!({"package":package,"instance":id}),
            )?;
        }
        let first = widgetjs_saved_state(app, ids[0].as_str().unwrap())?;
        let second = widgetjs_saved_state(app, ids[1].as_str().unwrap())?;
        check(
            report,
            "ported-widget-two-instances-save-independently",
            first != second,
            json!({"package":package,"first":first,"second":second}),
        )?;
    }
    if c.mode == "widgetjs" {
        if let Some(ids) = seed.get("local.souldesk.widgetjs.water-reminder") {
            verify_widgetjs_water_background(app, ids, report).await?;
        }
        if let Some(ids) = seed.get("local.souldesk.widgetjs.labor") {
            verify_widgetjs_labor_background(app, ids, report).await?;
        }
    }
    if c.mode == "widgetjs" {
        let mut expected = json!({});
        for ids in seed.as_object().unwrap().values() {
            for id in ids.as_array().unwrap() {
                let id = id.as_str().unwrap();
                expected[id] = json!({"savedState":widgetjs_saved_state(app,id)?,
                    "runtimeLoads":widgetjs_storage(app,id)?["verification"]["runtimeLoads"]});
            }
        }
        std::fs::write(
            c.root.join("expected.json"),
            serde_json::to_vec_pretty(&expected).map_err(err)?,
        )
        .map_err(err)?;
    }
    window(app, windows::destroy_views).await?;
    if seed.get("local.souldesk.widgetjs.coin-medium").is_some() {
        until(|| Ok(core.coin_live.diagnostics()?["ownerCount"] == 0), 5).await?;
        check(
            report,
            "coin-all-hidden-views-release-live-connections",
            core.coin_live.diagnostics()?["connections"]
                .as_array()
                .is_some_and(|v| v.is_empty()),
            core.coin_live.diagnostics()?,
        )?;
    }
    check(
        report,
        "ported-widget-hidden-views-release-all-sessions",
        core.sessions.0.lock().map_err(err)?.is_empty()
            && core.trials.lock().map_err(err)?.is_empty(),
        json!({"normalDatabaseOpened":false,"syntheticInteraction":true}),
    )?;
    report["status"] = json!(if c.mode == "widgetjs" {
        "restart-ready"
    } else {
        "passed"
    });
    report["scope"] = json!("isolated-production-host-and-WebView; programmatic form interaction; no normal database or installation; no manual visual acceptance");
    if c.mode == "widgetjs" {
        // 资源回收检查调用了隐藏全部；下一阶段要验证退出前可见场景的启动恢复，
        // 不能把测试清理产生的隐藏偏好当作用户主动隐藏。
        core.store.save_preference("hidden_all", "false")?;
    }
    save(c, report)?;
    window(app, windows::quit).await?;
    Ok(())
}

async fn verify_widgetjs_coin_sharing(
    app: &AppHandle,
    ids: &Value,
    report: &mut Value,
) -> AppResult<()> {
    let ids = ids.as_array().ok_or("网页币种实例清单无效")?;
    let first = ids[0].as_str().ok_or("币种实例无效")?.to_owned();
    let second = ids[1].as_str().ok_or("币种实例无效")?.to_owned();
    for id in [&first, &second] {
        until(
            || Ok(widgetjs_storage(app, id)?["verification"]["uiVerified"] == true),
            45,
        )
        .await?;
    }
    let core = app.state::<Core>();
    until(|| Ok(core.coin_live.diagnostics()?["ownerCount"] == 2), 5).await?;
    let before = core.coin_live.diagnostics()?;
    let connection_id = before["connections"][0]["connectionId"].clone();
    check(
        report,
        "coin-same-network-instances-share-one-live-connection",
        before["connections"]
            .as_array()
            .is_some_and(|v| v.len() == 1)
            && before["connections"][0]["owners"] == 2
            && before["connections"][0]["connection"] == "connected",
        before.clone(),
    )?;
    let hidden = first.clone();
    let hidden_at = now_ms();
    window(app, move |app| windows::hide_instance(app, &hidden)).await?;
    until(|| Ok(core.coin_live.diagnostics()?["ownerCount"] == 1), 5).await?;
    until(
        || {
            Ok(
                core.coin_live.diagnostics()?["connections"][0]["lastReceivedAt"]
                    .as_u64()
                    .unwrap_or(0)
                    > hidden_at,
            )
        },
        10,
    )
    .await?;
    let remaining = core.coin_live.diagnostics()?;
    let snapshot = widgetjs_storage(app, &second)?["verification"]["latestSnapshot"].clone();
    check(
        report,
        "coin-hiding-one-instance-keeps-other-live-owner",
        remaining["connections"]
            .as_array()
            .is_some_and(|v| v.len() == 1)
            && remaining["connections"][0]["connectionId"] == connection_id
            && remaining["connections"][0]["connection"] == "connected"
            && snapshot["displayedCurrencies"].as_u64().unwrap_or(0) > 0,
        json!({"registry":remaining,"remainingInstance":second,"lastViewSnapshot":snapshot,"hiddenAt":hidden_at,"receivedNewDataAfterHiding":true}),
    )?;
    let shown = first.clone();
    window(app, move |app| windows::restore_instance(app, &shown)).await?;
    until(|| Ok(core.coin_live.diagnostics()?["ownerCount"] == 2), 10).await?;
    check(
        report,
        "coin-restored-view-rejoins-surviving-connection",
        core.coin_live.diagnostics()?["connections"][0]["connectionId"] == connection_id,
        core.coin_live.diagnostics()?,
    )?;
    Ok(())
}

fn widgetjs_labor_state(store: &Store, id: &str) -> AppResult<Value> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let text: String = store
        .db
        .lock()
        .map_err(err)?
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [format!("labor.state:{}", URL_SAFE_NO_PAD.encode(id))],
            |row| row.get(0),
        )
        .map_err(err)?;
    serde_json::from_str(&text).map_err(err)
}

fn widgetjs_labor_tag_count(app_id: &str, tag: &str) -> AppResult<u32> {
    #[cfg(windows)]
    {
        use ::windows::{core::HSTRING, UI::Notifications::ToastNotificationManager};
        let history = ToastNotificationManager::History()
            .map_err(err)?
            .GetHistoryWithId(&HSTRING::from(app_id))
            .map_err(err)?;
        let mut count = 0;
        for index in 0..history.Size().map_err(err)? {
            let toast = history.GetAt(index).map_err(err)?;
            if toast.Tag().map_err(err)?.to_string() == tag
                && toast.Group().map_err(err)?.to_string() == crate::labor::NOTIFICATION_GROUP
            {
                count += 1;
            }
        }
        Ok(count)
    }
    #[cfg(not(windows))]
    {
        let _ = (app_id, tag);
        Err("当前平台未实现通知历史验收".into())
    }
}

// 使用正式一秒循环和真实通知接口；只调整这次隔离实例的班次，不修改系统时间。
async fn verify_widgetjs_labor_background(
    app: &AppHandle,
    ids: &Value,
    report: &mut Value,
) -> AppResult<()> {
    let core = app.state::<Core>();
    let ids = ids.as_array().ok_or("打工实例清单无效")?;
    if ids.len() != 2 {
        return Err("打工后台验收需要两个隔离实例".into());
    }
    let first = ids[0].as_str().ok_or("打工实例无效")?;
    let second = ids[1].as_str().ok_or("打工实例无效")?;
    let original = windows::instance(app, first)?["settings"].clone();
    let probe = widgetjs_storage(app, first)?["verification"]["notificationTestSettings"].clone();
    if !probe.is_object() || probe == original {
        return Err("缺少临近下班的隔离测试配置".into());
    }
    let other_before = widgetjs_labor_state(&core.store, second)?;
    window(app, windows::destroy_views).await?;
    let started = now_ms();
    let ticks = core.ticks.load(Ordering::Relaxed);
    core.store
        .db
        .lock()
        .map_err(err)?
        .execute(
            "UPDATE widget_instances SET settings=?1 WHERE id=?2",
            rusqlite::params![probe.to_string(), first],
        )
        .map_err(err)?;
    let mut wait_error = None;
    if let Err(error) = until(
        || {
            let state = widgetjs_labor_state(&core.store, first)?;
            Ok(core.labor.is_idle() && state["delivery"] != "" && state["delivery"] != "sending")
        },
        20,
    )
    .await
    {
        wait_error = Some(error);
    }
    let mut observed = vec![widgetjs_labor_state(&core.store, first)?];
    if wait_error.is_none() {
        for _ in 0..2 {
            if observed.last().unwrap()["delivery"] != "accepted" {
                break;
            }
            let sequence = observed.last().unwrap()["last_sequence"]
                .as_u64()
                .unwrap_or(0);
            if let Err(error) = until(
                || {
                    let state = widgetjs_labor_state(&core.store, first)?;
                    Ok(core.labor.is_idle()
                        && (state["delivery"] != "accepted"
                            || state["last_sequence"].as_u64().unwrap_or(0) > sequence))
                },
                5,
            )
            .await
            {
                wait_error = Some(error);
                break;
            }
            observed.push(widgetjs_labor_state(&core.store, first)?);
        }
    }
    let other_after = widgetjs_labor_state(&core.store, second)?;
    // 只统计本次专属标签及分组，不输出或清除其他实例、正常版的通知。
    let tag = observed[0]["tag"].as_str().unwrap_or("");
    let history_before = widgetjs_labor_tag_count(&app.config().identifier, tag);
    // 即使系统不接受显示或更新，也恢复原班次并清理本次专属标签。
    core.store
        .db
        .lock()
        .map_err(err)?
        .execute(
            "UPDATE widget_instances SET settings=?1 WHERE id=?2",
            rusqlite::params![original.to_string(), first],
        )
        .map_err(err)?;
    let last = observed.last().unwrap();
    let cleanup_key = last["tag"]
        .as_str()
        .filter(|tag| tag.len() == 16 && tag.bytes().all(|v| v.is_ascii_hexdigit()))
        .map(|tag| format!("labor.cleanup:{tag}"));
    let cleanup = if last["delivery"] == "accepted" {
        until(
            || {
                Ok(core.labor.is_idle()
                    && widgetjs_labor_state(&core.store, first)?["delivery"] == "stopped")
            },
            5,
        )
        .await
    } else if let Some(key) = &cleanup_key {
        core.store.save_preference(key, "pending")?;
        until(
            || {
                let count: u32 = core
                    .store
                    .db
                    .lock()
                    .map_err(err)?
                    .query_row(
                        "SELECT count(*) FROM app_settings WHERE key=?1",
                        [key],
                        |row| row.get(0),
                    )
                    .map_err(err)?;
                Ok(count == 0 && core.labor.is_idle())
            },
            5,
        )
        .await
    } else {
        Ok(())
    };
    let cleaned = widgetjs_labor_state(&core.store, first)?;
    let history_removed = until(
        || Ok(widgetjs_labor_tag_count(&app.config().identifier, tag)? == 0),
        5,
    )
    .await;
    let history_after = widgetjs_labor_tag_count(&app.config().identifier, tag);
    let after_ticks = core.ticks.load(Ordering::Relaxed);
    report["laborBackgroundObserved"] = json!({"settingsFixture":probe,"started":started,"ticksBefore":ticks,"ticksAfter":after_ticks,"observed":observed,"waitError":wait_error,"cleanupError":cleanup.as_ref().err(),"cleaned":cleaned,"historyBefore":history_before,"historyAfter":history_after,"historyRemoveError":history_removed.as_ref().err(),"notificationGroup":crate::labor::NOTIFICATION_GROUP,"actualDisplayVerified":false});
    let shown = &observed[0];
    check(
        report,
        "labor-zero-view-notification-request-accepted",
        core.sessions.0.lock().map_err(err)?.is_empty()
            && core.trials.lock().map_err(err)?.is_empty()
            && after_ticks > ticks
            && shown["delivery"] == "accepted"
            && shown["target"]
                .as_u64()
                .is_some_and(|v| v > started && v <= started + 300000),
        json!({"state":shown,"nearShiftFixture":true,"actualDisplayVerified":false}),
    )?;
    let updated = observed.len() == 3
        && observed.iter().all(|v| {
            v["delivery"] == "accepted"
                && v["tag"] == shown["tag"]
                && v["target"] == shown["target"]
                && v["sent"] == shown["sent"]
        })
        && observed
            .windows(2)
            .all(|pair| pair[1]["last_sequence"].as_u64() > pair[0]["last_sequence"].as_u64());
    check(
        report,
        "labor-live-notification-silently-updated",
        updated && wait_error.is_none(),
        json!({"states":observed,"actualDisplayVerified":false,"waitError":wait_error}),
    )?;
    check(
        report,
        "labor-second-instance-not-affected",
        other_before == other_after,
        json!({"before":other_before,"after":other_after}),
    )?;
    check(
        report,
        "labor-restored-shift-clears-old-notification",
        cleanup.is_ok()
            && history_removed.is_ok()
            && history_before == Ok(1)
            && history_after == Ok(0)
            && cleaned["delivery"] == "stopped"
            && cleaned["error"].is_null()
            && windows::instance(app, first)?["settings"] == original,
        json!({"state":cleaned,"cleanupError":cleanup.err(),"settingsRestored":true,"historyBefore":history_before,"historyAfter":history_after,"historyRemoveError":history_removed.err()}),
    )?;
    Ok(())
}

fn widgetjs_water_data(store: &Store, id: &str) -> AppResult<Value> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let encoded = URL_SAFE_NO_PAD.encode(id);
    let db = store.db.lock().map_err(err)?;
    let text: String = db
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [format!("water.state:{encoded}")],
            |row| row.get(0),
        )
        .map_err(err)?;
    let mut state: Value = serde_json::from_str(&text).map_err(err)?;
    // 服务运行标识每次启动都会变化，不能作为业务数据恢复的比较项。
    state
        .as_object_mut()
        .ok_or("喝水后台状态无效")?
        .remove("run_id");
    let prefix = format!("water.history:{encoded}:");
    let mut query = db
        .prepare(
            "SELECT key,value FROM app_settings WHERE substr(key,1,length(?1))=?1 ORDER BY key",
        )
        .map_err(err)?;
    let rows = query
        .query_map([&prefix], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    let mut history = json!({});
    for (key, value) in rows {
        history[&key[prefix.len()..]] = serde_json::from_str(&value).map_err(err)?;
    }
    Ok(json!({"state":state,"history":history}))
}

// 仅操作当前隔离实例；预置到期时间后等待正式的一秒后台循环，不直接调用调度器。
async fn verify_widgetjs_water_background(
    app: &AppHandle,
    ids: &Value,
    report: &mut Value,
) -> AppResult<()> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let core = app.state::<Core>();
    let ids = ids.as_array().ok_or("喝水实例清单无效")?;
    if ids.len() != 2 {
        return Err("喝水后台验收需要两个隔离实例".into());
    }
    let first = ids[0].as_str().ok_or("喝水实例无效")?;
    let second = ids[1].as_str().ok_or("喝水实例无效")?;
    let original = [
        windows::instance(app, first)?["settings"].clone(),
        windows::instance(app, second)?["settings"].clone(),
    ];
    if original.iter().any(|v| v["enableReminder"] != false) {
        return Err("后台验收前应先关闭两个实例的提醒".into());
    }
    window(app, windows::destroy_views).await?;
    let started = now_ms();
    let seeded_last_at = started.saturating_sub(5 * 60000);
    let ticks = core.ticks.load(Ordering::Relaxed);
    {
        let mut db = core.store.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        for (index, id) in [first, second].iter().enumerate() {
            let key = format!("water.state:{}", URL_SAFE_NO_PAD.encode(id));
            let text: String = tx
                .query_row(
                    "SELECT value FROM app_settings WHERE key=?1",
                    [&key],
                    |row| row.get(0),
                )
                .map_err(err)?;
            let mut state: Value = serde_json::from_str(&text).map_err(err)?;
            state["last_at"] = json!(seeded_last_at);
            state["pending"] = Value::Null;
            state["delivery"] = json!("");
            state["error"] = Value::Null;
            let mut settings = original[index].clone();
            settings["enableReminder"] = json!(index == 0);
            settings["interval"] = json!(5);
            settings["targetCup"] = json!(20);
            tx.execute(
                "UPDATE app_settings SET value=?1 WHERE key=?2",
                rusqlite::params![state.to_string(), key],
            )
            .map_err(err)?;
            tx.execute(
                "UPDATE widget_instances SET settings=?1 WHERE id=?2",
                rusqlite::params![settings.to_string(), id],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)?;
    }
    let restore = || -> AppResult<()> {
        let db = core.store.db.lock().map_err(err)?;
        for (index, id) in [first, second].iter().enumerate() {
            db.execute(
                "UPDATE widget_instances SET settings=?1 WHERE id=?2",
                rusqlite::params![original[index].to_string(), id],
            )
            .map_err(err)?;
        }
        Ok(())
    };
    let delivered = until(
        || {
            Ok(matches!(
                widgetjs_water_data(&core.store, first)?["state"]["delivery"].as_str(),
                Some("accepted" | "failed")
            ))
        },
        20,
    )
    .await;
    if let Err(error) = delivered {
        restore()?;
        return Err(error);
    }
    let after = widgetjs_water_data(&core.store, first)?;
    let disabled = widgetjs_water_data(&core.store, second)?;
    let after_ticks = core.ticks.load(Ordering::Relaxed);
    let waited = until(
        || Ok(core.ticks.load(Ordering::Relaxed) >= after_ticks + 2),
        5,
    )
    .await;
    if let Err(error) = waited {
        restore()?;
        return Err(error);
    }
    let repeated = widgetjs_water_data(&core.store, first)?;
    let disabled_again = widgetjs_water_data(&core.store, second)?;
    restore()?;
    until(
        || {
            Ok(
                widgetjs_water_data(&core.store, first)?["state"]["pending"].is_null()
                    && widgetjs_water_data(&core.store, second)?["state"]["pending"].is_null(),
            )
        },
        5,
    )
    .await?;
    check(
        report,
        "water-background-scheduler-with-no-views",
        core.sessions.0.lock().map_err(err)?.is_empty()
            && core.trials.lock().map_err(err)?.is_empty()
            && after_ticks > ticks
            && after["state"]["last_at"]
                .as_u64()
                .is_some_and(|v| v >= started),
        json!({"seededDueTime":true,"seededLastAt":seeded_last_at,"started":started,"ticksBefore":ticks,"ticksAfter":after_ticks,"state":after}),
    )?;
    check(
        report,
        "water-system-notification-request-accepted",
        after["state"]["delivery"] == "accepted",
        json!({"delivery":after["state"]["delivery"],"error":after["state"]["error"],"actualDisplayVerified":false,"actualButtonVerified":false}),
    )?;
    check(
        report,
        "water-disabled-instance-does-not-notify",
        disabled["state"]["last_at"] == seeded_last_at
            && disabled["state"]["delivery"] == ""
            && disabled["state"]["pending"].is_null()
            && disabled == disabled_again,
        json!({"state":disabled}),
    )?;
    check(
        report,
        "water-background-notification-not-repeated",
        after["state"]["pending"]["id"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
            && after == repeated,
        json!({"first":after,"afterTwoMoreCoreTicks":repeated,"settingsRestored":true}),
    )?;
    Ok(())
}

// 仅显式隔离验收使用项目生成图片及测试目录授权，不测试或替代文件夹选择。
fn prepare_widgetjs_photo_fixture(store: &Store, c: &Config, id: &str) -> AppResult<()> {
    use base64::Engine;
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../components/widgetjs/photo-common/fixture.json"
    ))
    .map_err(err)?;
    let root = c.root.join("photo-fixtures").join(id);
    std::fs::create_dir_all(&root).map_err(err)?;
    for image in fixtures["images"].as_array().ok_or("图片夹具无效")? {
        let name = image["name"].as_str().ok_or("夹具名称无效")?;
        if !["a.png", "b.png"].contains(&name) {
            return Err("夹具名称不在固定清单".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(image["data"].as_str().ok_or("夹具内容无效")?)
            .map_err(err)?;
        std::fs::write(root.join(name), bytes).map_err(err)?;
    }
    std::fs::write(root.join("c-broken.png"), b"invalid-image-fixture").map_err(err)?;
    store.save_preference(&format!("media_directory:{id}"), &json!({"path":root.canonicalize().map_err(err)?,"revision":uuid::Uuid::new_v4().to_string()}).to_string())
}

// 单实例文件流程必须经过原生文件对话框；不给组件增加指定任意路径的接口。
async fn execute_widgetjs_convert_files(
    app: &AppHandle,
    c: &Config,
    report: &mut Value,
) -> AppResult<()> {
    use sha2::{Digest, Sha256};
    let core = app.state::<Core>();
    let id = if c.mode == "widgetjs" {
        let id =
            crate::create_instance(app.clone(), "local.souldesk.widgetjs.convert".into()).await?;
        std::fs::write(
            c.root.join("seed.json"),
            serde_json::to_vec(&json!({"convert":id})).map_err(err)?,
        )
        .map_err(err)?;
        id
    } else {
        let seed: Value =
            serde_json::from_slice(&std::fs::read(c.root.join("seed.json")).map_err(err)?)
                .map_err(err)?;
        seed["convert"].as_str().ok_or("缺少转换实例")?.to_owned()
    };
    report["instance"] = json!(id);
    report["selectedPackages"] = json!(["local.souldesk.widgetjs.convert"]);
    save(c, report)?;
    until(
        || {
            let value = widgetjs_storage(app, &id)?;
            if let Some(error) = value["verification"]["failure"].as_str() {
                return Err(format!("转换文件验收失败：{error}"));
            }
            Ok(value["verification"]["uiVerified"] == true
                && value["verification"]["runtimeLoads"].as_u64().unwrap_or(0)
                    >= if c.mode == "widgetjs" { 1 } else { 2 })
        },
        125,
    )
    .await?;
    let data = widgetjs_storage(app, &id)?;
    let value = &data["verification"];
    check(
        report,
        "convert-real-file-open-decode-and-export",
        value["uiVerified"] == true
            && value["formats"]
                .as_array()
                .is_some_and(|items| items.len() == 5)
            && value["invalidSaveRejected"] == true
            && value["configurationRetained"] == true
            && value["temporaryFilesReset"] == true,
        json!({"instance":id,"result":value,"syntheticFormInteraction":true,"nativeDialogsRequireObservedSelection":true}),
    )?;
    let path = c.root.join("converted_images.zip");
    let metadata = std::fs::metadata(&path).map_err(err)?;
    if !metadata.is_file() || metadata.len() > 32 * 1024 * 1024 {
        return Err("输出ZIP类型或大小不符".into());
    }
    let bytes = std::fs::read(&path).map_err(err)?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    check(
        report,
        "convert-native-disk-file-matches-browser-output",
        value["outputSha256"].as_str() == Some(&digest)
            && value["outputBytes"].as_u64() == Some(bytes.len() as u64),
        json!({"file":"converted_images.zip","bytes":bytes.len(),"sha256":digest}),
    )?;
    let settings = windows::instance(app, &id)?["settings"].clone();
    if c.mode == "widgetjs" {
        std::fs::write(
            c.root.join("expected.json"),
            serde_json::to_vec(&settings).map_err(err)?,
        )
        .map_err(err)?;
    } else {
        let expected: Value =
            serde_json::from_slice(&std::fs::read(c.root.join("expected.json")).map_err(err)?)
                .map_err(err)?;
        check(
            report,
            "convert-settings-survive-process-restart",
            settings == expected,
            json!({"settings":settings,"expected":expected}),
        )?;
    }
    window(app, windows::destroy_views).await?;
    check(
        report,
        "convert-hidden-views-release-sessions",
        core.sessions.0.lock().map_err(err)?.is_empty()
            && core.trials.lock().map_err(err)?.is_empty(),
        json!({"normalDatabaseOpened":false}),
    )?;
    core.store.save_preference("hidden_all", "false")?;
    report["status"] = json!(if c.mode == "widgetjs" {
        "restart-ready"
    } else {
        "passed"
    });
    report["scope"]=json!("one-converter-real-native-file-dialogs-and-file-output; programmatic-conversion-and-forms; HEIC-unverified; no-normal-data");
    save(c, report)?;
    window(app, windows::quit).await?;
    Ok(())
}

// 只覆盖正式包管线和隐藏预览；不创建业务实例、不调用组件的交互验收脚本。
async fn execute_widgetjs_previews(
    app: &AppHandle,
    c: &Config,
    report: &mut Value,
) -> AppResult<()> {
    let core = app.state::<Core>();
    let ids = presets::catalog(&[])?
        .iter()
        .filter_map(|item| item["id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let mut failures = Vec::new();
    if c.mode == "widgetjs" {
        report["selectedPackages"] = json!(ids);
        for id in &ids {
            report["currentPackage"] = json!(id);
            save(c, report)?;
            let result = crate::packages::ensure_preset(app, id).await;
            let details =
                json!({"package":id,"error":result.as_ref().err(),"syntheticInteraction":false});
            if let Err(error) = check(
                report,
                "preset-hidden-preview-ready",
                result.is_ok(),
                details,
            ) {
                failures.push(error);
            }
        }
    }
    let snapshot = core.store.snapshot()?;
    let actual = snapshot["packages"]
        .as_array()
        .ok_or("预置登记快照无效")?
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect::<std::collections::HashSet<_>>();
    check(
        report,
        "all-presets-registered-and-retained",
        actual.len() == ids.len() && ids.iter().all(|id| actual.contains(id.as_str())),
        json!({"expected":ids.len(),"actual":actual.len(),"failures":failures}),
    )?;
    check(
        report,
        "preview-only-no-business-instances-or-sessions",
        snapshot["instances"]
            .as_array()
            .is_some_and(|items| items.is_empty())
            && core.sessions.0.lock().map_err(err)?.is_empty()
            && core.trials.lock().map_err(err)?.is_empty(),
        json!({"instances":snapshot["instances"],"normalDatabaseOpened":false}),
    )?;
    if !failures.is_empty() {
        return Err(format!("{}个预置隐藏加载失败，见逐项记录", failures.len()));
    }
    report["currentPackage"] = Value::Null;
    report["status"] = json!(if c.mode == "widgetjs" {
        "restart-ready"
    } else {
        "passed"
    });
    report["scope"] = json!("all-preset-hidden-preview-load-and-registry-restart; no-created-business-instances; no-functional-or-visual-acceptance");
    save(c, report)?;
    window(app, windows::quit).await?;
    Ok(())
}

pub fn start(app: &AppHandle) {
    let Some(c) = app.state::<Core>().integration.clone() else {
        return;
    };
    if c.mode == "manual" || c.mode == "widgetjs-manual" {
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut report = if c.mode == "resume" || c.mode == "surface" || c.mode == "widgetjs-resume"
        {
            std::fs::read(&c.report)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(json!({"checks":[]}))
        } else {
            json!({"version":env!("CARGO_PKG_VERSION"),"phase":"normal-application-isolated-integration","checks":[],"normalDatabaseOpened":false,"installationEvidence":false})
        };
        report["reportPath"] = json!(c.report);
        report["status"] = json!("running");
        let _ = save(&c, &report);
        if let Err(e) = execute(&handle, &c, &mut report).await {
            handle.state::<Core>().report(e.clone());
            report["status"] = json!("failed");
            report["error"] = json!(e);
            let _ = save(&c, &report);
            if c.standalone_check() {
                handle
                    .state::<Core>()
                    .quitting
                    .store(true, Ordering::Relaxed);
                handle.exit(1);
            }
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_panel_shortcut_is_enabled_without_enabling_automatic_checks() {
        for mode in ["manual", "widgetjs-manual", "first", "resume", "surface"] {
            let config = Config {
                root: PathBuf::new(),
                report: PathBuf::new(),
                mode: mode.into(),
            };
            assert!(config.panel_shortcut_enabled(), "{mode}");
        }
        for mode in ["panel", "widgetjs", "widgetjs-resume"] {
            let config = Config {
                root: PathBuf::new(),
                report: PathBuf::new(),
                mode: mode.into(),
            };
            assert!(!config.panel_shortcut_enabled(), "{mode}");
            assert!(config.standalone_check());
        }
    }

    #[test]
    fn concurrent_exit_events_remain_complete_json_lines() {
        let path =
            std::env::temp_dir().join(format!("souldesk-exit-{}.jsonl", uuid::Uuid::new_v4()));
        let threads = (0..8)
            .map(|worker| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for sequence in 0..40 {
                        append_lifecycle(&path, &json!({"worker":worker,"sequence":sequence}))
                            .unwrap();
                    }
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }
        let text = std::fs::read_to_string(path).unwrap();
        let mut entries = std::collections::HashSet::new();
        for line in text.lines() {
            let value: Value = serde_json::from_str(line).unwrap();
            assert!(entries.insert((
                value["worker"].as_u64().unwrap(),
                value["sequence"].as_u64().unwrap()
            )));
        }
        assert_eq!(entries.len(), 320);
    }
    #[test]
    fn invalid_isolation_never_falls_back_to_normal_data() {
        for args in [
            vec!["app", "--desktop-integration"],
            vec!["app", "--desktop-integration", "relative.json"],
            vec!["app", "--desktop-integration-unknown", "C:/x.json"],
            vec!["app", "--desktop-sandbox", "C:/Users/normal"],
        ] {
            assert!(parse(&args.into_iter().map(String::from).collect::<Vec<_>>()).is_err());
        }
    }
}
