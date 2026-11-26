mod import;
mod manage;
mod transport;

use crate::{
    store::{err, now_ms, AppResult, Store},
    widgets, Core,
};
use chrono::Utc;
use import::{ConnectionInfo, Profile, Server};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tokio::{
    sync::Notify,
    task::JoinSet,
    time::{sleep, Instant},
};
use transport::{now, stamp};

pub(crate) const COMPONENT_ID: &str = "local.souldesk.cloud-workbench";
pub(crate) const CAPABILITY: &str = "cloud-workbench.read";
pub(crate) const WAKE: &str = "cloud-workbench.wake";
pub(crate) const MANAGE: &str = "cloud-workbench.manage";
const FRESH: f64 = 180.;

#[derive(Default)]
pub(crate) struct Service {
    due: Notify,
    gate: Arc<tokio::sync::Mutex<()>>,
    collecting: AtomicBool,
    completed: AtomicU64,
    wake_message: Mutex<Option<Value>>,
}

fn enabled(store: &Store) -> bool {
    store
        .db
        .lock()
        .ok()
        .and_then(|db| {
            db.query_row(
                "SELECT EXISTS (SELECT 1 FROM widget_instances WHERE package_id=?1)",
                [COMPONENT_ID],
                |row| row.get(0),
            )
            .ok()
        })
        .unwrap_or(false)
}

fn identity(server: &Server, mut row: Value) -> Value {
    if !row.is_object() {
        row = json!({ "status":"unknown" });
    }
    row["id"] = json!(server.id);
    row["name"] = json!(server.name);
    row["role"] = json!(server.role);
    row["links"] = json!(server
        .links
        .iter()
        .map(|link| &link.name)
        .map(|name| json!({"name":name}))
        .collect::<Vec<_>>());
    let (provider, address, account) = match &server.connection {
        ConnectionInfo::Aliyun { host, .. } => ("aliyun", host, None),
        ConnectionInfo::Instabox {
            machine_id,
            account,
            ..
        } => ("instabox", machine_id, account.as_ref()),
    };
    row["provider"] = json!(provider);
    row["address"] = json!(address);
    row["account"] = json!(account);
    row["credentialReady"] = json!(server.connection.ready());
    row
}

fn load_row(store: &Store, server: &Server) -> AppResult<Value> {
    let db = store.db.lock().map_err(err)?;
    let raw: Option<String> = db
        .query_row(
            "SELECT payload FROM cloud_snapshots WHERE id=?1",
            [&server.id],
            |row| row.get(0),
        )
        .optional()
        .map_err(err)?;
    let row = match raw {
        Some(raw) => serde_json::from_str(&raw).map_err(err)?,
        None => Value::Null,
    };
    Ok(identity(server, row))
}

fn add_event(db: &rusqlite::Connection, id: &str, level: &str, message: &str) -> AppResult<()> {
    db.execute(
        "INSERT INTO cloud_events(ts,id,level,message) VALUES(?1,?2,?3,?4)",
        params![now(), id, level, message],
    )
    .map_err(err)?;
    Ok(())
}

fn save_row(store: &Store, id: &str, old: &Value, row: &Value, samples: &[Value]) -> AppResult<()> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    tx.execute(
        "INSERT OR REPLACE INTO cloud_snapshots VALUES(?1,?2)",
        params![id, row.to_string()],
    )
    .map_err(err)?;
    for sample in samples {
        if let Some(at) = stamp(&sample["measured_at"]) {
            tx.execute(
                "INSERT OR REPLACE INTO cloud_samples VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    id,
                    at,
                    sample["cpu_pct"].as_f64(),
                    sample["memory_pct"].as_f64(),
                    sample["disk_pct"].as_f64(),
                    sample["net_in"].as_f64(),
                    sample["net_out"].as_f64()
                ],
            )
            .map_err(err)?;
        }
    }
    if row["error"].is_null() && row["status"] != old["status"] {
        if let Some(state) = row["status"].as_str() {
            add_event(&tx, id, "info", &format!("状态更新：{state}"))?;
        }
    }
    if row["operation"]["state"] != old["operation"]["state"] {
        match row["operation"]["state"].as_str() {
            Some("succeeded") => add_event(&tx, id, "info", "手动唤醒已完成")?,
            Some("failed") => add_event(&tx, id, "error", "手动唤醒失败")?,
            _ => {}
        }
    }
    for alert in row["alerts"].as_array().into_iter().flatten() {
        if !old["alerts"]
            .as_array()
            .is_some_and(|previous| previous.contains(alert))
        {
            if let (Some(level), Some(text)) = (alert["level"].as_str(), alert["text"].as_str()) {
                add_event(&tx, id, level, text)?;
            }
        }
    }
    tx.commit().map_err(err)
}

fn recent(value: &Value, at: f64) -> bool {
    stamp(value).is_some_and(|time| (0. ..FRESH).contains(&(at - time)))
}

fn alerts(row: &Value, at: f64) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(error) = row["error"].as_str() {
        out.push(json!({"level":"error","text":error}));
    }
    for note in row["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        out.push(json!({"level":"warning","text":note}));
    }
    if row["status"] == "error" {
        out.push(json!({"level":"error","text":"平台报告实例异常"}));
    }
    if row["status"] != "running" || !row["error"].is_null() {
        return out;
    }
    if let Some(message) =
        (!recent(&row["metrics"]["measured_at"], at)).then_some("资源指标尚未更新")
    {
        out.push(json!({"level":"warning","text":message}));
    } else {
        for (key, label) in [("cpu_pct", "CPU"), ("memory_pct", "内存")] {
            if let Some(value) = row["metrics"][key].as_f64().filter(|value| *value >= 90.) {
                out.push(json!({"level":"warning","text":format!("{label}使用率 {value:.1}%，达到 90% 阈值")}));
            }
        }
    }
    if recent(&row["disk_measured_at"], at) {
        for item in row["disks"].as_array().into_iter().flatten() {
            if let Some(used) = item["used_pct"].as_f64().filter(|used| *used >= 90.) {
                let mount = item["mount"].as_str().unwrap_or("未知");
                out.push(
                    json!({"level":"warning","text":format!("磁盘 {mount} 使用率 {used:.1}%")}),
                );
            }
        }
    }
    for group in ["services", "health"] {
        for item in row[group].as_array().into_iter().flatten() {
            if item["ok"] == false {
                let name = item["name"].as_str().unwrap_or("服务");
                out.push(json!({"level":"error","text":format!("{name}健康检查未通过")}));
            }
        }
    }
    out
}

fn row_summary(row: &Value, at: f64) -> Value {
    let mut state = row["status"].as_str().unwrap_or("unknown");
    let (mut cpu, mut memory, mut disk) = (None, None, None);
    if row["credentialReady"] == false {
        state = "needs_credentials";
    } else if !row["error"].is_null() {
        state = "collect_failed";
    } else if row["checked_at"].is_null() {
        state = "pending";
    } else if !recent(&row["checked_at"], at) {
        state = "stale";
    } else if matches!(state, "suspended" | "stopped") {
        if stamp(&row["limits"]["cooldownUntil"]).is_some_and(|expires| expires > at) {
            state = "cooling";
        }
    } else if state == "running" {
        let metrics = &row["metrics"];
        if recent(&metrics["measured_at"], at) {
            cpu = metrics["cpu_pct"].as_f64();
            memory = metrics["memory_pct"].as_f64();
            disk = metrics["disk_pct"].as_f64();
        } else {
            state = "stale";
        }
        if recent(&row["disk_measured_at"], at) {
            disk = row["disks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|disk| disk["used_pct"].as_f64())
                .reduce(f64::max)
                .or(disk);
        }
    }
    json!({"name":row["name"],"state":state,"cpuPct":cpu,"memoryPct":memory,"diskPct":disk})
}

async fn collect_one(
    store: Arc<Store>,
    profile: Arc<Profile>,
    index: usize,
    client: reqwest::Client,
) -> AppResult<()> {
    let server = &profile.servers[index];
    let old = load_row(&store, server)?;
    let mut row = old.clone();
    let (mut row, samples) =
        match transport::collect(&client, server, &store.root, &mut row, profile.interval).await {
            Ok(samples) => {
                row["error"] = Value::Null;
                (row, samples)
            }
            Err(error) => {
                let mut preserved = old.clone();
                preserved["error"] = json!(error);
                preserved["warnings"] = json!([]);
                (preserved, Vec::new())
            }
        };
    row["checked_at"] = json!(Utc::now().to_rfc3339());
    row["alerts"] = json!(alerts(&row, now()));
    save_row(&store, &server.id, &old, &row, &samples)
}

async fn collect_cycle(store: Arc<Store>, profile: Arc<Profile>) -> AppResult<()> {
    let client = transport::client()?;
    let mut workers = JoinSet::new();
    for (index, server) in profile.servers.iter().enumerate() {
        if !server.connection.ready() {
            continue;
        }
        workers.spawn(collect_one(
            store.clone(),
            profile.clone(),
            index,
            client.clone(),
        ));
    }
    let mut failure = None;
    while let Some(result) = workers.join_next().await {
        if !matches!(result, Ok(Ok(()))) {
            failure = Some("本轮部分采集结果无法保存");
        }
    }
    let cutoff = now() - 86400. * profile.retention as f64;
    let db = store.db.lock().map_err(err)?;
    db.execute("DELETE FROM cloud_samples WHERE ts<?1", [cutoff])
        .map_err(err)?;
    db.execute("DELETE FROM cloud_events WHERE ts<?1", [cutoff])
        .map_err(err)?;
    db.execute(
        "DELETE FROM cloud_wake_requests WHERE created<?1",
        [(cutoff * 1000.) as u64],
    )
    .map_err(err)?;
    if let Some(failure) = failure {
        return Err(failure.into());
    }
    Ok(())
}

pub(crate) fn start(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let core = handle.state::<Core>();
        let service = &core.cloud_workbench;
        let mut due = Instant::now();
        while !core.quitting.load(Ordering::Acquire) {
            if Instant::now() >= due && enabled(&core.store) {
                if let Ok(_gate) = service.gate.try_lock() {
                    if let Ok(Some(profile)) = import::load(&core.store) {
                        due = Instant::now() + Duration::from_secs(profile.interval);
                        service.collecting.store(true, Ordering::Release);
                        let result = tokio::select! {
                            result = collect_cycle(core.store.clone(),Arc::new(profile)) => Some(result),
                            _ = async {
                                loop {
                                    sleep(Duration::from_secs(1)).await;
                                    if core.quitting.load(Ordering::Acquire) || !enabled(&core.store) { break; }
                                }
                            } => None,
                        };
                        service.collecting.store(false, Ordering::Release);
                        if matches!(result, Some(Ok(()))) {
                            service.completed.store(now_ms(), Ordering::Release);
                        }
                    }
                }
            }
            tokio::select! {
                _ = service.due.notified() => due = Instant::now(),
                _ = sleep(Duration::from_secs(3)) => {},
            }
        }
    });
}

pub(crate) fn import_command(args: &[String]) -> Option<AppResult<Value>> {
    let action = args.get(1).map(String::as_str).filter(|action| {
        matches!(
            *action,
            "--souldesk-cloud-import"
                | "--souldesk-cloud-read-check"
                | "--souldesk-cloud-repair-ssh"
        )
    })?;
    Some((|| {
        let target = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|value| value.is_absolute())
            .ok_or("找不到本机 SoulDesk 数据目录")?
            .join("local.souldesk.desktop");
        if !target.join("workbench.sqlite").is_file() {
            return Err("本机尚未初始化 SoulDesk 数据目录".into());
        }
        let store = Store::open(target)?;
        if action != "--souldesk-cloud-read-check" {
            let source = args
                .get(2)
                .map(PathBuf::from)
                .filter(|path| args.len() == 3 && path.is_absolute())
                .ok_or("需要原云服务器项目的绝对路径")?;
            return if action == "--souldesk-cloud-repair-ssh" {
                import::repair_ssh(&store, &source)
            } else {
                import::run(&store, &source)
            };
        }
        if args.len() != 2 {
            return Err("只读采集检查不接受参数".into());
        }
        let profile = Arc::new(import::load(&store)?.ok_or("尚未导入服务器配置")?);
        let store = Arc::new(store);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(err)?;
        runtime.block_on(collect_cycle(store.clone(), profile.clone()))?;
        let mut results = Vec::new();
        for server in &profile.servers {
            let row = load_row(&store, server)?;
            results.push(json!({"provider":row["provider"],"readOk":server.connection.ready() && row["error"].is_null() && !row["checked_at"].is_null(),"state":row["status"],
                "metricsPresent":row["metrics"].is_object(),"warnings":row["warnings"],"error":row["error"]}));
        }
        Ok(json!({"servers":results}))
    })())
}

// 仅由本机命令行发起现有实例的原位升级，继续经过宿主试加载与版本检查。
pub(crate) fn queue_widget_upgrade(app: &AppHandle, args: &[String]) {
    if args.len() != 3 || args[1] != "--souldesk-cloud-widget" {
        return;
    }
    let path = PathBuf::from(&args[2]);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        use std::io::Read;
        let core = handle.state::<Core>();
        let result: AppResult<Value> = async {
            if !path.is_absolute() || !enabled(&core.store) || import::load(&core.store)?.is_none()
            {
                return Err("需要已迁移的云服务器工作台实例和本地 ZIP".into());
            }
            let mut bytes = Vec::new();
            std::fs::File::open(&path)
                .and_then(|file| {
                    file.take(widgets::MAX_PACKAGE_BYTES as u64 + 1)
                        .read_to_end(&mut bytes)
                })
                .map_err(|_| "无法读取云服务器工作台 ZIP")?;
            let manifest = crate::packages::install_verified(&handle, bytes, None, |manifest| {
                if manifest.id != COMPONENT_ID
                    || !manifest.capabilities.iter().any(|cap| cap == WAKE)
                {
                    return Err("只能升级为支持新控制台的云服务器组件".into());
                }
                Ok(())
            })
            .await?;
            Ok(json!({"version":manifest.version,"upgraded":true}))
        }
        .await;
        let report = result.unwrap_or_else(|error| json!({"upgraded":false,"error":error}));
        let _ = std::fs::write(
            core.store.root.join("cloud-widget-upgrade.json"),
            report.to_string(),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_summary_preserves_zero_and_ignores_stale_samples() {
        let row = json!({"name":"fixture","status":"running","checked_at":"2026-01-01T12:00:00Z",
            "metrics":{"measured_at":"2026-01-01T12:00:00Z","cpu_pct":0}});
        let at = stamp(&row["checked_at"]).unwrap();
        assert_eq!(row_summary(&row, at)["cpuPct"], 0.0);
        let stale = row_summary(&row, at + 181.);
        assert_eq!(stale["state"], "stale");
        assert!(stale["cpuPct"].is_null());
    }

    #[test]
    fn wake_requires_the_matching_target_state_and_cooldown() {
        use transport::wake_action;
        assert_eq!(wake_action(&json!({"state":"running"}), 0.).unwrap(), None);
        assert_eq!(
            wake_action(&json!({"state":"suspended"}), 0.).unwrap(),
            Some("resume")
        );
        assert_eq!(
            wake_action(&json!({"state":"stopped"}), 0.).unwrap(),
            Some("start")
        );
        assert!(wake_action(&json!({"state":"suspended","operationId":"other"}), 0.).is_err());
        assert!(wake_action(
            &json!({"state":"suspended","cooldownUntil":"2026-01-01T12:00:00Z"}),
            0.
        )
        .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn legacy_import_is_encrypted_idempotent_and_preserves_samples() {
        let sandbox =
            std::env::temp_dir().join(format!("souldesk-cloud-import-{}", uuid::Uuid::new_v4()));
        let source = sandbox.join("source");
        for folder in ["workbench", "secrets", "local"] {
            std::fs::create_dir_all(source.join(folder)).unwrap();
        }
        std::fs::write(source.join("workbench/servers.json"),r#"{"interval_seconds":60,"retention_days":7,
            "servers":[{"id":"fixture","name":"测试","provider":"instabox","machine_id":"m_test","org_id":"org_test","credential":"secrets/box.json"}]}"#).unwrap();
        std::fs::write(
            source.join("secrets/box.json"),
            r#"{"key":"test-only-key"}"#,
        )
        .unwrap();
        let old = rusqlite::Connection::open(source.join("local/workbench.sqlite3")).unwrap();
        old.execute_batch("CREATE TABLE snapshots(id,payload); CREATE TABLE samples(id,ts,cpu,memory,disk,rx,tx); CREATE TABLE events(seq INTEGER PRIMARY KEY,ts,id,level,message);").unwrap();
        old.execute(
            "INSERT INTO samples VALUES('fixture',?1,0,5,NULL,NULL,NULL)",
            [now()],
        )
        .unwrap();
        let store = Store::open(sandbox.join("host")).unwrap();
        let summary = import::run(&store, &source).unwrap();
        assert_eq!(summary["history"]["samples"], 1);
        assert!(!store
            .preference(import::VAULT)
            .unwrap()
            .contains("test-only-key"));
        let profile = import::load(&store).unwrap().unwrap();
        assert_eq!(profile.servers[0].id, "fixture");
        assert_eq!(
            history(&store, &profile, &json!({"id":"fixture","hours":1})).unwrap()["samples"][0]
                ["cpu"],
            0.0
        );
        assert_eq!(
            import::run(&store, &source).unwrap()["alreadyImported"],
            true
        );
    }
}

fn reserve_wake(store: &Store, id: &str, request: &str) -> AppResult<bool> {
    let db = store.db.lock().map_err(err)?;
    let inserted = db
        .execute(
            "INSERT OR IGNORE INTO cloud_wake_requests VALUES(?1,?2,?3)",
            params![request, id, now_ms()],
        )
        .map_err(err)?;
    if inserted == 1 {
        return Ok(true);
    }
    let previous: String = db
        .query_row(
            "SELECT server FROM cloud_wake_requests WHERE request=?1",
            [request],
            |row| row.get(0),
        )
        .map_err(err)?;
    if previous != id {
        return Err("此唤醒请求编号已用于其他服务器".into());
    }
    Ok(false)
}

fn start_wake(app: &AppHandle, args: &Value) -> AppResult<Value> {
    let id = args["id"].as_str().ok_or("服务器编号无效")?;
    let request = args["requestId"]
        .as_str()
        .and_then(|raw| uuid::Uuid::parse_str(raw).ok())
        .ok_or("唤醒请求编号无效")?
        .to_string();
    let core = app.state::<Core>();
    let guard = core
        .cloud_workbench
        .gate
        .clone()
        .try_lock_owned()
        .map_err(|_| "正在采集或处理操作，请稍后再试")?;
    let profile = import::load(&core.store)?.unwrap_or_default();
    let index = profile
        .servers
        .iter()
        .position(|server| {
            server.id == id && matches!(server.connection, ConnectionInfo::Instabox { .. })
        })
        .ok_or("只允许唤醒已登记的 Instabox 实例")?;
    if !profile.servers[index].connection.ready() {
        return Err("请先在编辑界面填写 Instabox API 密钥".into());
    }
    if !reserve_wake(&core.store, id, &request)? {
        return Ok(json!({"message":"这次请求已经登记，请查看操作结果"}));
    }
    let id = id.to_owned();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let core = handle.state::<Core>();
        let result: AppResult<String> = async {
            let server = &profile.servers[index];
            let client = transport::client()?;
            let Some((state, operation)) = transport::wake(&client, server, &request).await? else {
                return Ok("实例已在运行".into());
            };
            let old = load_row(&core.store, server)?;
            let mut row = old.clone();
            row["status"] = json!(state);
            row["operation"] = operation;
            row["error"] = Value::Null;
            save_row(&core.store, &id, &old, &row, &[])
                .map_err(|_| "唤醒已提交，保存结果失败，请刷新平台状态")?;
            Ok("唤醒已提交，等待平台返回运行状态".into())
        }
        .await;
        let success = result.is_ok();
        let message = result.unwrap_or_else(|error| error);
        if let Ok(db) = core.store.db.lock() {
            let _ = add_event(&db, &id, if success { "info" } else { "warning" }, &message);
        }
        if let Ok(mut slot) = core.cloud_workbench.wake_message.lock() {
            *slot = Some(json!({"requestId":request,"id":id,"message":message,"ok":success}));
        }
        drop(guard);
        core.cloud_workbench.due.notify_one();
    });
    Ok(json!({"message":"宿主已登记请求，正在核对平台状态"}))
}

fn history(store: &Store, profile: &Profile, args: &Value) -> AppResult<Value> {
    let id = args["id"]
        .as_str()
        .filter(|id| profile.servers.iter().any(|server| server.id == *id))
        .ok_or("服务器不存在")?;
    let hours = args["hours"]
        .as_u64()
        .filter(|hours| matches!(hours, 1 | 24 | 168))
        .ok_or("历史时间范围无效")?;
    let db = store.db.lock().map_err(err)?;
    let mut stmt = db.prepare("SELECT ts,cpu,memory,disk,rx,tx FROM cloud_samples WHERE id=?1 AND ts>=?2 ORDER BY ts DESC LIMIT 24000").map_err(err)?;
    let mut rows = stmt.query_map(params![id,now() - hours as f64 * 3600.],|row| Ok(json!({"ts":row.get::<_,f64>(0)?,
        "cpu":row.get::<_,Option<f64>>(1)?,"memory":row.get::<_,Option<f64>>(2)?,"disk":row.get::<_,Option<f64>>(3)?,
        "rx":row.get::<_,Option<f64>>(4)?,"tx":row.get::<_,Option<f64>>(5)?}))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
    rows.reverse();
    Ok(json!({"samples":rows}))
}

const VIEW_FIELDS: &[&str] = &[
    "id",
    "name",
    "role",
    "links",
    "provider",
    "address",
    "account",
    "credentialReady",
    "status",
    "checked_at",
    "error",
    "warnings",
    "alerts",
    "metrics",
    "disks",
    "disk_measured_at",
    "spec",
    "template",
    "runtime_ready",
    "limits",
    "usage",
    "operation",
    "services",
    "health",
    "containers",
    "tasks",
    "containers_available",
];

fn details(core: &Core, profile: &Profile) -> AppResult<Value> {
    let mut rows = Vec::new();
    for server in &profile.servers {
        rows.push(transport::public_fields(
            &load_row(&core.store, server)?,
            VIEW_FIELDS,
        ));
    }
    let events = if profile.servers.is_empty() {
        Vec::new()
    } else {
        let db = core.store.db.lock().map_err(err)?;
        let ids: Vec<_> = profile.servers.iter().map(|server| &server.id).collect();
        let mut stmt = db.prepare(
            "SELECT ts,id,level,message FROM cloud_events WHERE id IN (SELECT value FROM json_each(?1)) ORDER BY seq DESC LIMIT 60",
        ).map_err(err)?;
        let events = stmt
            .query_map([json!(ids).to_string()], |row| {
                Ok(
                    json!({"ts":row.get::<_,f64>(0)?,"id":row.get::<_,String>(1)?,
                "level":row.get::<_,Option<String>>(2)?,"message":row.get::<_,Option<String>>(3)?}),
                )
            })
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        events
    };
    let completed = core.cloud_workbench.completed.load(Ordering::Acquire);
    let wake = core.cloud_workbench.wake_message.lock().map_err(err)?;
    let wake = wake.as_ref().filter(|event| {
        profile
            .servers
            .iter()
            .any(|server| event["id"] == server.id)
    });
    Ok(
        json!({"servers":rows,"events":events,"last_cycle":chrono::DateTime::from_timestamp_millis(completed as i64).filter(|_|completed>0).map(|value|value.to_rfc3339()),
        "collecting":core.cloud_workbench.collecting.load(Ordering::Acquire)||core.cloud_workbench.gate.try_lock().is_err(),
        "interval_seconds":profile.interval,"retention_days":profile.retention,
        "wakeResult":wake}),
    )
}

fn empty_args(args: &Value) -> bool {
    args.is_null() || args.as_object().is_some_and(|object| object.is_empty())
}

fn require_management(core: &Core, window: &str, token: &str) -> AppResult<()> {
    let instance = widgets::require_capability(&core.store, &core.sessions, window, token, MANAGE)?;
    if window != format!("detail-{instance}") {
        return Err("请在云服务器控制台中操作".into());
    }
    Ok(())
}

pub(crate) async fn backup_dialog(
    app: &AppHandle,
    window: &str,
    token: &str,
    save: bool,
) -> AppResult<Value> {
    let core = app.state::<Core>();
    require_management(&core, window, token)?;
    let _dialog = crate::documents::start_dialog(&core)?;
    let handle = app.clone();
    let label = window.to_owned();
    let path = tauri::async_runtime::spawn_blocking(move || {
        let owner = handle.get_webview_window(&label).ok_or("控制台已关闭")?;
        let picker = handle
            .dialog()
            .file()
            .set_parent(&owner)
            .set_title(if save {
                "导出服务器完整备份"
            } else {
                "导入服务器完整备份"
            })
            .add_filter("服务器备份", &["json"]);
        let choice = if save {
            picker
                .set_file_name("SoulDesk-服务器完整备份.json")
                .blocking_save_file()
        } else {
            picker.blocking_pick_file()
        };
        choice.map(|file| file.into_path().map_err(err)).transpose()
    })
    .await
    .map_err(err)??;
    let Some(mut path) = path else {
        return Ok(Value::Null);
    };
    if save && path.extension().is_none() {
        path.set_extension("json");
    }
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        return Err("请选择 JSON 格式的服务器备份".into());
    }
    require_management(&core, window, token)?;
    if save {
        let mut bytes = manage::export_backup(&core.store)?;
        let written = crate::documents::atomic_write(&path, &bytes);
        bytes.fill(0);
        written?;
        return Ok(json!({"name":path.file_name().map(|name| name.to_string_lossy())}));
    }
    let _gate = tokio::time::timeout(Duration::from_secs(5), core.cloud_workbench.gate.lock())
        .await
        .map_err(|_| "正在采集或处理服务器操作，请稍后重试")?;
    require_management(&core, window, token)?;
    let mut bytes = crate::documents::read_selected(&path, manage::MAX_IMPORT_BYTES)?;
    let result = manage::import_backup(&core.store, &bytes);
    bytes.fill(0);
    let result = result?;
    if result["ready"].as_u64().is_some_and(|ready| ready > 0) {
        core.cloud_workbench.due.notify_one();
    }
    Ok(result)
}

pub(crate) async fn gateway(
    app: &AppHandle,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    let core = app.state::<Core>();
    let instance =
        widgets::require_capability(&core.store, &core.sessions, window, token, CAPABILITY)?;
    if method == "cloudWorkbench.summary" && empty_args(&args) {
        let profile = import::load(&core.store)?;
        let mut rows = Vec::new();
        let at = now();
        if let Some(profile) = &profile {
            for server in &profile.servers {
                rows.push(row_summary(&load_row(&core.store, server)?, at));
            }
        }
        let last = core.cloud_workbench.completed.load(Ordering::Acquire);
        return Ok(
            json!({"schemaVersion":1,"native":true,"configured":profile.is_some(),"choosing":false,
            "lastCycleAt":(last>0).then_some(last),"servers":rows}),
        );
    }
    if method == "cloudWorkbench.openConsole" && empty_args(&args) {
        let handle = app.clone();
        return tauri::async_runtime::spawn_blocking(move || {
            crate::windows::serialized(&handle, |app| {
                crate::windows::detail_at(app, &instance, "cloud-workbench")
            })?;
            Ok(Value::Null)
        })
        .await
        .map_err(err)?;
    }
    if window != format!("detail-{instance}") {
        return Err("请在云服务器控制台中操作".into());
    }
    if method.starts_with("cloudWorkbench.manage.") {
        require_management(&core, window, token)?;
        if method == "cloudWorkbench.manage.list" {
            if !empty_args(&args) {
                return Err("读取服务器清单不接受参数".into());
            }
            return manage::list(&core.store);
        }
        let _guard = tokio::time::timeout(Duration::from_secs(5), core.cloud_workbench.gate.lock())
            .await
            .map_err(|_| "正在采集或处理服务器操作，请稍后重试")?;
        require_management(&core, window, token)?;
        let result = manage::write(&core.store, method, args)?;
        if method == "cloudWorkbench.manage.save" && result["result"]["ready"] == true {
            core.cloud_workbench.due.notify_one();
        }
        return Ok(result);
    }
    let profile = import::load(&core.store)?.unwrap_or_default();
    match method {
        "cloudWorkbench.details" if empty_args(&args) => details(&core, &profile),
        "cloudWorkbench.history" if args.as_object().is_some_and(|object| object.len() == 2) => {
            history(&core.store, &profile, &args)
        }
        "cloudWorkbench.refresh" if empty_args(&args) => {
            core.cloud_workbench.due.notify_one();
            Ok(json!({"message":"已请求刷新"}))
        }
        "cloudWorkbench.wake" if args.as_object().is_some_and(|object| object.len() == 2) => {
            widgets::require_capability(&core.store, &core.sessions, window, token, WAKE)?;
            start_wake(app, &args)
        }
        "cloudWorkbench.openService"
            if args.as_object().is_some_and(|object| object.len() == 2) =>
        {
            let index = args["index"]
                .as_u64()
                .and_then(|index| usize::try_from(index).ok())
                .ok_or("服务入口编号无效")?;
            let server = profile
                .servers
                .iter()
                .find(|server| args["id"].as_str() == Some(&server.id))
                .ok_or("服务器不存在")?;
            let link = server.links.get(index).ok_or("服务入口不存在")?;
            crate::content::open_external(&crate::content::web_url(&link.url)?)
        }
        _ => Err("云服务器工作台请求参数无效".into()),
    }
}
