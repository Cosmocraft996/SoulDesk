//! 常驻行情和灵动通知各有独立进程，通过已有事件表交接。
use crate::{
    alert_notice, alert_store, business, integration, market,
    network::NetworkConfig,
    reminder_store,
    store::{err, now_ms, AppResult, Store},
    windows, Core,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

const WORKER_FLAG: &str = "--souldesk-market-worker";
const RUN_STATE: &str = "market.runtime.state";
const RECONNECT: &str = "market.runtime.reconnect";
const OPEN_CHART: &str = "market.runtime.open_chart";
const OPEN_HISTORY: &str = "market.runtime.open_history";
const PENDING: &str = "market.runtime.pending";
const CHECK: &str = "market-isolation";

#[derive(Clone, Copy)]
pub(crate) enum Role {
    InProcess,
    Desktop,
    Notifier,
}
impl Role {
    pub(crate) fn is_desktop(self) -> bool {
        matches!(self, Self::Desktop)
    }
    pub(crate) fn is_notifier(self) -> bool {
        matches!(self, Self::Notifier)
    }
}

fn runtime_dir(root: &Path) -> PathBuf {
    root.join("market-runtime")
}

fn owner(root: &Path, role: &str) -> AppResult<Option<File>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(runtime_dir(root).join(format!("{role}.lock")))
        .map_err(err)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(error) => Err(err(error)),
    }
}

fn worker(root: &Path, role: &str, check: bool) -> AppResult<Child> {
    let executable = std::env::current_exe().map_err(err)?;
    let mut command = Command::new(executable);
    command
        .args([WORKER_FLAG, role])
        .arg(root)
        .stdin(Stdio::null());
    if check {
        command.arg(CHECK);
        let output = OpenOptions::new()
            .append(true)
            .create(true)
            .open(runtime_dir(root).join(format!("{role}.log")))
            .map_err(err)?;
        command
            .stdout(output.try_clone().map_err(err)?)
            .stderr(output);
    }
    command.spawn().map_err(err)
}

fn maintain_child(
    store: &Store,
    child: &mut Option<Child>,
    role: &str,
    check: bool,
) -> AppResult<()> {
    if let Some(current) = child {
        if current.try_wait().map_err(err)?.is_none() {
            return Ok(());
        }
        *child = None;
    }
    if owner(&store.root, role)?.is_some() {
        *child = Some(worker(&store.root, role, check)?);
    }
    Ok(())
}

fn preference(store: &Store, key: &str) -> AppResult<Option<String>> {
    let db = store.db.lock().map_err(err)?;
    reminder_store::preference(&db, key)
}

fn stopped(store: &Store) -> AppResult<bool> {
    Ok(preference(store, RUN_STATE)?.as_deref() == Some("stopped"))
}

fn check_stopped(store: &Store, check: bool) -> AppResult<bool> {
    if check && store.root.join("stop-isolation.marker").is_file() {
        return Ok(true);
    }
    stopped(store)
}

pub(crate) fn stop(store: &Store) -> AppResult<()> {
    store.save_preference(RUN_STATE, "stopped")
}

pub(crate) fn resume(store: &Store) -> AppResult<()> {
    store.save_preference(RUN_STATE, "running")
}

fn workers_stopped(root: &Path) -> AppResult<bool> {
    Ok(owner(root, "core")?.is_some() && owner(root, "notice")?.is_some())
}

pub(crate) fn pause_for_update(store: &Store) -> AppResult<()> {
    stop(store)?;
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if workers_stopped(&store.root)? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("行情或灵动通知尚未退出，暂停本次更新".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub(crate) fn request_reconnect(store: &Store) -> AppResult<()> {
    store.save_preference(RECONNECT, &uuid::Uuid::new_v4().to_string())
}

fn activation(app: &AppHandle, key: &str, event: &str) -> AppResult<()> {
    let core = app.state::<Core>();
    core.store.save_preference(key, event)?;
    if !is_check(app) && owner(&core.store.root, "desktop")?.is_some() {
        // 崩溃后由用户点击唤起界面。仍在运行的界面自行读取请求。
        Command::new(std::env::current_exe().map_err(err)?)
            .stdin(Stdio::null())
            .spawn()
            .map_err(err)?;
    }
    Ok(())
}

pub(crate) fn open_chart(app: &AppHandle, id: &str) -> AppResult<()> {
    activation(app, OPEN_CHART, id)
}

pub(crate) fn open_history(app: &AppHandle, id: &str) -> AppResult<()> {
    activation(app, OPEN_HISTORY, id)
}

pub(crate) fn pending_notices(store: &Store) -> AppResult<Vec<Value>> {
    if owner(&store.root, "notice")?.is_some() {
        return Ok(Vec::new());
    }
    let pending = preference(store, PENDING)?;
    pending
        .map(|raw| serde_json::from_str(&raw).map_err(err))
        .transpose()
        .map(Option::unwrap_or_default)
}

fn apply_activation(app: &AppHandle) -> AppResult<()> {
    let core = app.state::<Core>();
    for (key, open) in [
        (
            OPEN_CHART,
            (|app, id| alert_notice::open(app, id, crate::alerts::NotificationChannel::Island))
                as fn(&AppHandle, &str) -> AppResult<()>,
        ),
        (OPEN_HISTORY, windows::notification_history),
    ] {
        if let Some(event) = preference(&core.store, key)? {
            let result = windows::serialized(app, |app| open(app, &event));
            // 正在开窗时写入的新请求留给下一轮。
            core.store
                .db
                .lock()
                .map_err(err)?
                .execute(
                    "DELETE FROM app_settings WHERE key=?1 AND value=?2",
                    rusqlite::params![key, event],
                )
                .map_err(err)?;
            result?;
        }
    }
    Ok(())
}

pub(crate) fn is_check(app: &AppHandle) -> bool {
    app.state::<Core>()
        .integration
        .as_ref()
        .is_some_and(|config| config.mode == CHECK)
}

struct DesktopOwner(#[allow(dead_code)] File);

pub(crate) fn start_desktop(app: &AppHandle) -> AppResult<()> {
    let core = app.state::<Core>();
    fs::create_dir_all(runtime_dir(&core.store.root)).map_err(err)?;
    app.manage(DesktopOwner(
        owner(&core.store.root, "desktop")?.ok_or("同一数据目录的桌面进程已启动")?,
    ));
    resume(&core.store)?;
    let check = is_check(app);
    let store = core.store.clone();
    // 监测后台进程不借用窗口线程或窗口任务池。
    std::thread::Builder::new()
        .name("souldesk-market-supervisor".into())
        .spawn(move || {
            let mut child = None;
            loop {
                let result = check_stopped(&store, check).and_then(|stop| {
                    if !stop {
                        maintain_child(&store, &mut child, "core", check)?;
                    }
                    Ok(())
                });
                if let Err(error) = result {
                    let _ = store.log("error", &format!("行情核心启动失败：{error}"));
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        })
        .map_err(err)?;
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(300)).await;
            if handle.state::<Core>().quitting.load(Ordering::Acquire) {
                break;
            }
            let target = handle.clone();
            let result =
                tauri::async_runtime::spawn_blocking(move || apply_activation(&target)).await;
            if let Ok(Err(error)) = result {
                handle.state::<Core>().report(error);
            }
        }
    });
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct SourceClock {
    sampled_at: u64,
    source_now: u64,
}

#[cfg(windows)]
fn session_uptime_ms() -> u64 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }
}

#[cfg(not(windows))]
fn session_uptime_ms() -> u64 {
    now_ms()
}

fn publish_clock(root: &Path, clock: SourceClock) -> AppResult<()> {
    let directory = runtime_dir(root);
    let staged = directory.join("source-clock.tmp");
    fs::write(&staged, serde_json::to_vec(&clock).map_err(err)?).map_err(err)?;
    fs::rename(staged, directory.join("source-clock.json")).map_err(err)
}

fn source_now(root: &Path) -> Option<u64> {
    let bytes = fs::read(runtime_dir(root).join("source-clock.json")).ok()?;
    let clock: SourceClock = serde_json::from_slice(&bytes).ok()?;
    let age = session_uptime_ms().checked_sub(clock.sampled_at)?;
    (age < 3000).then(|| clock.source_now.saturating_add(age))
}

pub(crate) fn run_worker(args: &[String]) -> Option<AppResult<()>> {
    (args.get(1).is_some_and(|arg| arg == WORKER_FLAG)).then(|| {
        if !(4..=5).contains(&args.len()) {
            return Err("行情工作进程参数无效".into());
        }
        let root = PathBuf::from(&args[3]);
        let check = args.len() == 5;
        if !root.is_absolute()
            || (check
                && (args[4] != CHECK
                    || !root
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("desktop-integration-"))
                    || !root.join("isolated-test-data.marker").is_file()))
        {
            return Err("行情工作进程数据目录无效".into());
        }
        let role = args[2].as_str();
        let log = match role {
            "core" => "market-core",
            "notice" => "market-notice",
            _ => return Err("未知行情工作进程".into()),
        };
        let Some(_ownership) = owner(&root, role)? else {
            return Ok(());
        };
        let store = Store::open_market_worker(root, log)?;
        if check_stopped(&store, check)? {
            return Ok(());
        }
        match role {
            "core" => tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(err)?
                .block_on(run_core(Arc::new(store), check)),
            _ => run_notice(store, check),
        }
    })
}

async fn run_core(store: Arc<Store>, check: bool) -> AppResult<()> {
    let recovery = Arc::new(crate::market_replay::Session::begin(&store, check)?);
    let mut feeds = Vec::new();
    let mut fixture = None;
    let mut recovery_fixture_sent = false;
    for name in ["usd_m", "spot"] {
        let (feed, desired, tx, input) = market::Feed::for_market(NetworkConfig::Direct, name);
        tokio::spawn(business::consume_feed(
            store.clone(),
            feed.clone(),
            input,
            Some(recovery.clone()),
        ));
        if check && name == "usd_m" && !runtime_dir(&store.root).join("quote.json").is_file() {
            fixture = Some(tx.clone());
        }
        feeds.push((feed, Some((desired, tx))));
    }
    let mut settings: Option<(Option<String>, AppResult<NetworkConfig>)> = None;
    let mut reconnect = None;
    let mut child = None;
    let mut child_check = None;
    let mut previous_status = [String::new(), String::new()];
    let mut timer = tokio::time::interval(Duration::from_millis(300));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    if check {
        evidence(&store.root, "core", json!({"pid":std::process::id()}))?;
    }
    loop {
        timer.tick().await;
        if check_stopped(&store, check)? {
            break;
        }
        if child_check.is_none_or(|at: Instant| at.elapsed() >= Duration::from_secs(2)) {
            child_check = Some(Instant::now());
            if let Err(error) = maintain_child(&store, &mut child, "notice", check) {
                let _ = store.log("error", &format!("灵动通知进程启动失败：{error}"));
            }
        }
        if check {
            if runtime_dir(&store.root).join("ui-frozen.json").is_file() {
                if let Some(tx) = fixture.take() {
                    let beginning = now_ms();
                    market::isolation_fixture(&feeds[0].0, tx, beginning).await?;
                    evidence(
                        &store.root,
                        "quote",
                        json!({"base":beginning,"spike":beginning + 80,"returned":beginning + 160}),
                    )?;
                }
            }
            if !recovery_fixture_sent
                && runtime_dir(&store.root)
                    .join("restart-request.json")
                    .is_file()
            {
                if let Some((_, tx)) = &feeds[0].1 {
                    inject_restart_fixture(&store, &feeds[0].0, tx).await?;
                    recovery_fixture_sent = true;
                }
            }
        } else {
            let network = preference(&store, "network")?;
            let next_reconnect = preference(&store, RECONNECT)?;
            let keys = business::rule_keys(&store)?;
            let config = match &settings {
                Some((saved, config)) if saved == &network => config.clone(),
                _ => {
                    let result = network
                        .as_ref()
                        .and_then(|value| serde_json::from_str::<Value>(value).ok())
                        .map_or_else(
                            || business::resolve_network(&json!({"mode":"system"})),
                            |value| business::resolve_network(&value),
                        );
                    if let Err(error) = &result {
                        let _ = store.log("error", error);
                    }
                    settings = Some((network, result.clone()));
                    result
                }
            };
            for (feed, channels) in &mut feeds {
                let market_keys = keys
                    .iter()
                    .filter(|key| key.market == feed.market)
                    .cloned()
                    .collect();
                feed.desired.send_if_modified(|desired| {
                    let wanted = if config.is_ok() {
                        market_keys
                    } else {
                        Default::default()
                    };
                    let changed = desired.keys != wanted
                        || config
                            .as_ref()
                            .is_ok_and(|network| network != &desired.network);
                    if reconnect != next_reconnect {
                        desired.reconnect += 1;
                    }
                    desired.keys = wanted;
                    if let Ok(network) = &config {
                        desired.network = network.clone();
                    }
                    changed || reconnect != next_reconnect
                });
                if config.is_ok() {
                    if let Some((desired, tx)) = channels.take() {
                        tokio::spawn(market::run(feed.clone(), desired, tx));
                    }
                }
            }
            reconnect = next_reconnect;
        }
        for ((feed, _), previous) in feeds.iter().zip(&mut previous_status) {
            let status = business::market_log_status(feed)?;
            if status != *previous {
                store.log("market", &status)?;
                *previous = status;
            }
        }
        if let Some(now) = feeds.iter().find_map(|(feed, _)| feed.source_now()) {
            if let Err(error) = publish_clock(
                &store.root,
                SourceClock {
                    sampled_at: session_uptime_ms(),
                    source_now: now,
                },
            ) {
                let _ = store.log("error", &format!("行情源时钟同步失败：{error}"));
            }
        }
    }
    for (feed, _) in &feeds {
        feed.cancel.cancel();
        drop(
            feed.processing
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
    }
    recovery.finish(&store)
}

fn recovery_keys() -> [crate::alerts::MarketKey; 2] {
    use crate::alerts::{MarketKey, PriceKind};
    [
        MarketKey::binance("BTCUSDT", PriceKind::Last),
        MarketKey::binance("ETHUSDT", PriceKind::Mark),
    ]
}

async fn inject_restart_fixture(
    store: &Store,
    feed: &market::Feed,
    tx: &tokio::sync::mpsc::Sender<market::Input>,
) -> AppResult<()> {
    let [trade, mark] = recovery_keys();
    let path = runtime_dir(&store.root).join("restart-baseline.json");
    if !path.is_file() {
        let time = now_ms();
        market::isolation_quote(feed, tx, &trade, "99", time, 4, Some(1010)).await?;
        market::isolation_quote(feed, tx, &mark, "99", time, 5, None).await?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if [&trade, &mark].into_iter().all(|key| {
                crate::market_replay::read_checkpoint(store, key).is_ok_and(|checkpoint| {
                    checkpoint.is_some_and(|checkpoint| checkpoint.time == time)
                })
            }) {
                break;
            }
            if Instant::now() >= deadline {
                return Err("模拟停机前行情断点未提交".into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        evidence(
            &store.root,
            "restart-baseline",
            json!({"pid":std::process::id(),"time":time}),
        )?;
        // 仅验收进程在持久化断点后模拟意外中断，跳过正常退出记录。
        std::process::exit(91);
    }
    let baseline: Value = serde_json::from_slice(&fs::read(path).map_err(err)?).map_err(err)?;
    let time = baseline["time"].as_u64().ok_or("模拟停机时间无效")?;
    evidence(
        &store.root,
        "recovery-trades",
        json!([
            {"a":1010,"p":"99","T":time},
            {"a":1011,"p":"101","T":time+80},
            {"a":1012,"p":"99","T":time+160},
        ]),
    )?;
    let resumed = now_ms().max(time + 240);
    market::isolation_quote(feed, tx, &trade, "99", resumed, 1, Some(1013)).await?;
    market::isolation_quote(feed, tx, &mark, "99", resumed, 2, None).await?;
    evidence(
        &store.root,
        "recovery-source",
        json!({
            "corePid":std::process::id(),
            "missingPriceBefore":time,
            "triggeredAt":time+80,
            "liveResumedAt":resumed,
        }),
    )
}

fn run_notice(store: Store, check: bool) -> AppResult<()> {
    Store::recover_market_deliveries(&*store.db.lock().map_err(err)?)?;
    store.save_preference(PENDING, "[]")?;
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            crate::reminder_host::island_surface
        ])
        .setup(move |app| {
            let config = check.then(|| integration::Config {
                report: store.root.with_extension("json"),
                root: store.root.clone(),
                mode: CHECK.into(),
            });
            crate::manage_core(app, store, config, None, Role::Notifier);
            let handle = app.handle().clone();
            if check {
                evidence(
                    &handle.state::<Core>().store.root,
                    "notifier",
                    json!({"pid":std::process::id()}),
                )?;
            }
            tauri::async_runtime::spawn(async move {
                run_notice_delivery(handle).await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                window
                    .app_handle()
                    .state::<Core>()
                    .reminders
                    .closed(window.label());
            }
        })
        .build(tauri::generate_context!())
        .map_err(err)?
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !app.state::<Core>().quitting.load(Ordering::Acquire) {
                    api.prevent_exit();
                }
            }
        });
    Ok(())
}

async fn run_notice_delivery(handle: AppHandle) {
    let mut last_prune = None;
    let mut prune_again = false;
    let mut timer = tokio::time::interval(Duration::from_millis(150));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        timer.tick().await;
        let target = handle.clone();
        let prune =
            prune_again || last_prune.is_none_or(|at: Instant| at.elapsed().as_secs() >= 60);
        if prune {
            last_prune = Some(Instant::now());
        }
        let result = tauri::async_runtime::spawn_blocking(move || -> AppResult<Option<bool>> {
            let core = target.state::<Core>();
            if check_stopped(&core.store, is_check(&target))? {
                return Ok(None);
            }
            core.reminders.sync_market(&target)?;
            let source_time = source_now(&core.store.root);
            let more = prune
                && core
                    .reminders
                    .prune_alert_history(&core.store, now_ms(), source_time)?;
            if let Some(now) = source_time {
                for event in alert_store::pending_deliveries(&core.store, now)? {
                    use alert_store::Channel;
                    let channel = Channel::Notification;
                    if alert_store::claim_delivery(&core.store, &event.id, channel)? {
                        let result = alert_notice::deliver(&target, &event);
                        alert_store::finish_delivery(&core.store, &event.id, channel, result)?;
                    }
                }
            }
            let pending = serde_json::to_string(&core.reminders.pending_notices()?).map_err(err)?;
            if preference(&core.store, PENDING)?.as_deref() != Some(pending.as_str()) {
                core.store.save_preference(PENDING, &pending)?;
            }
            Ok(Some(more))
        })
        .await;
        match result {
            Ok(Ok(None)) => {
                handle
                    .state::<Core>()
                    .quitting
                    .store(true, Ordering::Release);
                handle.exit(0);
                return;
            }
            Ok(Ok(Some(more))) => prune_again = more,
            Ok(Err(error)) => {
                prune_again = false;
                handle.state::<Core>().report(error);
            }
            Err(error) => {
                prune_again = false;
                handle.state::<Core>().report(error.to_string());
            }
        }
    }
}

fn evidence(root: &Path, name: &str, value: Value) -> AppResult<()> {
    let directory = runtime_dir(root);
    let temporary = directory.join(format!("{name}.tmp"));
    fs::write(&temporary, serde_json::to_vec(&value).map_err(err)?).map_err(err)?;
    fs::rename(temporary, directory.join(format!("{name}.json"))).map_err(err)
}

pub(crate) fn visible_evidence(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
    event: &str,
    snapshot: &Value,
) -> AppResult<()> {
    if !is_check(app) || !app.state::<Core>().market_role.is_notifier() {
        return Ok(());
    }
    #[cfg(windows)]
    let native_visible = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible(
            window.hwnd().map_err(err)?.0 as _,
        ) != 0
    };
    #[cfg(not(windows))]
    let native_visible = window.is_visible().map_err(err)?;
    evidence(
        &app.state::<Core>().store.root,
        "notice-visible",
        json!({
            "pid":std::process::id(),
            "at":now_ms(),
            "event":event,
            "nativeVisible":native_visible,
            "items":snapshot["items"],
        }),
    )
}

async fn wait_evidence(root: &Path, name: &str) -> AppResult<Value> {
    let path = runtime_dir(root).join(format!("{name}.json"));
    let timeout = Instant::now() + Duration::from_secs(15);
    loop {
        match fs::read(&path) {
            Ok(value) => return serde_json::from_slice(&value).map_err(err),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(err(error)),
        }
        if Instant::now() >= timeout {
            return Err(format!("未收到隔离验收记录 {name}"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub(crate) fn start_isolation_check(app: &AppHandle, config: integration::Config) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = check_isolation(&handle, &config.root).await;
        let mut report = json!({
            "status": if result.is_ok() { "passed" } else { "failed" },
            "scope":"三个实际 Windows 进程及 WebView2 通知窗口，模拟行情，独立数据库",
            "normalDatabaseOpened":false,
        });
        match result {
            Ok(isolation) => {
                match check_restart(&handle, &config.root, &isolation).await {
                    Ok(recovery) => report["recovery"] = recovery,
                    Err(error) => {
                        report["status"] = json!("failed");
                        report["error"] = json!(error);
                    }
                }
                report["evidence"] = isolation;
            }
            Err(error) => report["error"] = json!(error),
        }
        let core = handle.state::<Core>();
        if let Err(error) = stop(&core.store) {
            core.report(error);
        }
        // 确认工作进程释放本次目录后才退出测试主进程。
        let mut cleaned_up = false;
        for _ in 0..50 {
            if workers_stopped(&config.root).is_ok_and(|stopped| stopped) {
                cleaned_up = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        report["workerCleanup"] = json!(cleaned_up);
        if !cleaned_up && report["status"] == "passed" {
            report["status"] = json!("failed");
            report["error"] = json!("验收结束后工作进程未退出");
        }
        let ok = report["status"] == "passed";
        let _ = fs::write(
            config.report,
            serde_json::to_vec_pretty(&report).expect("报告可序列化"),
        );
        core.quitting.store(true, Ordering::Release);
        handle.exit(i32::from(!ok));
    });
}

async fn check_restart(app: &AppHandle, root: &Path, isolation: &Value) -> AppResult<Value> {
    use crate::alerts::{Condition, Direction, Mode, NotificationChannel, Rule};
    let core = app.state::<Core>();
    for key in recovery_keys() {
        let rule = Rule {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            name: format!("隔离重启 {}", key.symbol),
            key,
            direction: Direction::Up,
            condition: Condition::Cross,
            threshold: "100".into(),
            mode: Mode::Once,
            cooldown_ms: 0,
            enabled: true,
            notification: true,
            notification_channel: NotificationChannel::Island,
            sound: false,
        };
        alert_store::save_rule(&core.store, &rule, None)?;
    }
    evidence(root, "restart-request", json!({"at":now_ms()}))?;
    let baseline = wait_evidence(root, "restart-baseline").await?;
    let source = wait_evidence(root, "recovery-source").await?;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let events = alert_store::list_events(&core.store, 0, 10)?;
        let gaps = crate::market_replay::history(&core.store)?;
        let visible = wait_evidence(root, "notice-visible").await?;
        let replay = events.iter().find(|event| event["replayed"] == true);
        if let Some(replay) = replay {
            let recognized = visible["items"].as_array().is_some_and(|items| {
                items.iter().any(|item| {
                    item["notification"]["id"] == replay["id"]
                        && item["notification"]["marketAlert"]["replayed"] == true
                        && item["notification"]["marketAlert"]["triggeredAt"]
                            == source["triggeredAt"]
                })
            });
            if !gaps.is_empty() && recognized && replay["notification"] == "sent" {
                if events.len() != 2
                    || gaps.len() != 1
                    || baseline["pid"] != isolation["core"]["pid"]
                    || source["corePid"] == baseline["pid"]
                    || visible["pid"] != isolation["notice"]["pid"]
                    || visible["nativeVisible"] != true
                    || replay["sourceUtcMs"] != source["triggeredAt"]
                    || replay["observedPrice"] != "101"
                    || gaps[0]["key"]["priceKind"] != "mark"
                    || gaps[0]["fromUtcMs"] != source["missingPriceBefore"]
                    || gaps[0]["toUtcMs"] != source["liveResumedAt"]
                {
                    return Err(format!(
                        "核心重启后验收内容不符：{source}/{replay}/{gaps:?}/{visible}"
                    ));
                }
                return Ok(
                    json!({"baseline":baseline,"source":source,"shown":visible,"replay":replay,"gaps":gaps}),
                );
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "核心重启后未生成预期补报与缺口：{events:?}/{gaps:?}/{visible}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn check_isolation(app: &AppHandle, root: &Path) -> AppResult<Value> {
    let core = wait_evidence(root, "core").await?;
    let notifier = wait_evidence(root, "notifier").await?;
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        windows::serialized(&handle, |app| windows::show(app, false))
    })
    .await
    .map_err(err)??;
    let window = app.get_webview_window("main").ok_or("测试主界面未创建")?;
    if !window.is_visible().map_err(err)? {
        return Err("测试主界面未显示".into());
    }

    let directory = root.to_owned();
    app.run_on_main_thread(move || {
        let _ = evidence(
            &directory,
            "ui-frozen",
            json!({"pid":std::process::id(),"at":now_ms()}),
        );
        // 确实停住主线程的 Windows 事件循环，并保证能自行解冻。
        std::thread::sleep(Duration::from_secs(8));
        let _ = evidence(&directory, "ui-resumed", json!({"at":now_ms()}));
    })
    .map_err(err)?;

    let frozen = wait_evidence(root, "ui-frozen").await?;
    let quote = wait_evidence(root, "quote").await?;
    let visible = wait_evidence(root, "notice-visible").await?;
    let resumed = wait_evidence(root, "ui-resumed").await?;
    let number = |value: &Value, field: &str| {
        value[field]
            .as_u64()
            .ok_or_else(|| format!("缺少验收字段 {field}"))
    };
    let spike = number(&quote, "spike")?;
    let delay = number(&visible, "at")?
        .checked_sub(spike)
        .ok_or("通知先于测试触发")?;
    let ids = [
        number(&frozen, "pid")?,
        number(&core, "pid")?,
        number(&notifier, "pid")?,
    ];
    let history = alert_store::list_events(&app.state::<Core>().store, 0, 10)?;
    let event = history.first().ok_or("未保存触发事件")?;
    let event_id = event["id"].as_str().ok_or("触发事件缺少标识")?;
    if ids[0] == ids[1]
        || ids[0] == ids[2]
        || ids[1] == ids[2]
        || number(&visible, "pid")? != ids[2]
        || number(&frozen, "at")? > number(&quote, "base")?
        || number(&visible, "at")? >= number(&resumed, "at")?
        || visible["nativeVisible"] != true
        || delay > 5000
        || history.len() != 1
        || event["notification"] != "sent"
        || event["observedPrice"] != "101"
        || event["sourceUtcMs"] != spike
        || !visible["items"].as_array().is_some_and(|items| {
            items.iter().any(|item| {
                item["notification"]["id"] == event_id
                    && item["notification"]["marketAlert"]["price"] == "101"
            })
        })
    {
        return Err(format!("进程隔离结果不符：进程={ids:?}，延迟={delay}ms，界面={frozen}/{resumed}，行情={quote}，通知={visible}，历史={history:?}"));
    }
    Ok(
        json!({"ui":frozen,"uiResumed":resumed,"core":core,"notice":visible,"quote":quote,
        "showLatencyMs":delay,"maxAllowedLatencyMs":5000,"historyCount":history.len(),"trigger":event}),
    )
}
