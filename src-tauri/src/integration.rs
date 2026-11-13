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
    let root = if mode == "manual" {
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
    if mode == "first" || mode == "panel" {
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
async fn execute_panel(
    app: &AppHandle,
    c: &Config,
    report: &mut Value,
    clock: String,
) -> AppResult<()> {
    until(|| Ok(session_count(app, &clock) == 1), 20).await?;
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
    window(app, windows::collapse).await?;
    let out_ms = start.elapsed().as_millis();
    check(
        report,
        "panel-slide-out-releases-its-session",
        app.get_webview_window("panel").is_none() && session_count(app, &clock) == 0,
        json!({"configuredMs":150,"operationMs":out_ms}),
    )?;
    let start = Instant::now();
    window(app, |app| windows::show_panel(app, true)).await?;
    let in_ms = start.elapsed().as_millis();
    until(|| Ok(session_count(app, &clock) == 1), 15).await?;
    check(
        report,
        "panel-slide-in-restores-single-session-and-layout",
        app.state::<Core>().store.panel_layouts()?[&clock]
            == serde_json::to_value(layout).map_err(err)?,
        json!({"configuredMs":150,"operationMs":in_ms,"layout":layout}),
    )?;
    windows::auto_collapse(app);
    tokio::time::sleep(Duration::from_millis(50)).await;
    check(
        report,
        "explicit-open-is-not-collapsed-by-mouse-policy",
        app.get_webview_window("panel").is_some(),
        json!({"manual":app.state::<Core>().visibility.lock().map_err(err)?.panel_manual}),
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
    window(app, windows::destroy_views).await?;
    report["status"] = json!("passed");
    report["scope"] = json!("isolated-native-panel-lifecycle; no real mouse input, no installation, no updater installation");
    save(c, report)?;
    app.state::<Core>().quitting.store(true, Ordering::Relaxed);
    app.exit(0);
    Ok(())
}
pub fn start(app: &AppHandle) {
    let Some(c) = app.state::<Core>().integration.clone() else {
        return;
    };
    if c.mode == "manual" {
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut report = if c.mode == "resume" || c.mode == "surface" {
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
            if c.mode == "panel" {
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
