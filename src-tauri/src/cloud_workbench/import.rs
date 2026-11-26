use crate::{
    ai_credentials::local_dpapi,
    store::{err, AppResult, Store},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub(super) const VAULT: &str = "cloud-workbench.vault/v1";
const ENTROPY: &[u8] = b"SoulDesk.cloud-workbench.v1";

#[derive(Serialize, Deserialize)]
pub(super) struct Profile {
    pub interval: u64,
    pub retention: u64,
    pub servers: Vec<Server>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            interval: 60,
            retention: 7,
            servers: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct Server {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    #[serde(default)]
    pub links: Vec<Health>,
    #[serde(flatten)]
    pub connection: ConnectionInfo,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub(super) enum ConnectionInfo {
    Aliyun {
        host: String,
        user: String,
        port: u16,
        key: String,
        pinned: String,
        services: Vec<String>,
        health_urls: Vec<Health>,
    },
    Instabox {
        machine_id: String,
        org_id: String,
        account: Option<String>,
        key: String,
    },
}

impl ConnectionInfo {
    pub(super) fn ready(&self) -> bool {
        match self {
            Self::Aliyun { key, pinned, .. } => !key.is_empty() && !pinned.is_empty(),
            Self::Instabox { key, .. } => !key.is_empty(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Health {
    pub name: String,
    pub url: String,
}

#[derive(Deserialize)]
struct Legacy {
    interval_seconds: u64,
    retention_days: u64,
    servers: Vec<LegacyServer>,
}

#[derive(Deserialize)]
struct LegacyServer {
    id: String,
    name: String,
    role: Option<String>,
    #[serde(default)]
    links: Vec<Health>,
    #[serde(flatten)]
    connection: LegacyConnection,
}

#[derive(Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
enum LegacyConnection {
    Aliyun {
        address: String,
        ssh_alias: String,
        services: Vec<String>,
        health_urls: Vec<Health>,
    },
    Instabox {
        machine_id: String,
        org_id: String,
        credential: PathBuf,
        account: Option<String>,
    },
}

fn read_file(path: &Path, max: u64) -> AppResult<Vec<u8>> {
    let mut data = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(max + 1).read_to_end(&mut data))
        .map_err(|_| "无法读取原工作台配置或凭据")?;
    if data.len() as u64 > max {
        return Err("原工作台配置或凭据超过大小限制".into());
    }
    Ok(data)
}

fn json_file<T: serde::de::DeserializeOwned>(path: &Path, max: u64) -> AppResult<T> {
    let raw = read_file(path, max)?;
    serde_json::from_slice(raw.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&raw))
        .map_err(|_| "原工作台配置或凭据格式无效".into())
}

pub(super) fn segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
}

pub(super) fn ssh_tool(name: &str) -> AppResult<PathBuf> {
    #[cfg(windows)]
    let executable = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or("无法定位 Windows 系统目录")?
        .join("System32/OpenSSH")
        .join(format!("{name}.exe"));
    #[cfg(not(windows))]
    let executable = Path::new("/usr/bin").join(name);
    if !executable.is_file() {
        return Err("本机缺少系统 OpenSSH 客户端".into());
    }
    Ok(executable)
}

fn quiet(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.stdin(Stdio::null()).stderr(Stdio::null())
}

fn ssh_credentials(
    address: &str,
    alias: &str,
    legacy_root: &Path,
) -> AppResult<(String, String, u16, String, String)> {
    if !segment(alias) || !segment(address) {
        return Err("阿里云连接配置无效".into());
    }
    let output = quiet(Command::new(ssh_tool("ssh")?).args(["-G", "--", alias]))
        .output()
        .map_err(|_| "无法读取原 SSH 连接配置")?;
    if !output.status.success() || output.stdout.len() > 64 * 1024 {
        return Err("无法解析原 SSH 连接配置".into());
    }
    let config = std::str::from_utf8(&output.stdout).map_err(|_| "原 SSH 配置输出编码无效")?;
    let values = |option: &'static str| {
        config.lines().filter_map(move |line| {
            let (name, value) = line.split_once(' ')?;
            (name == option).then_some(value.trim())
        })
    };
    let host = values("hostname")
        .next()
        .filter(|value| *value == address)
        .ok_or("SSH 目标与原服务器配置不一致")?;
    let user = values("user")
        .next()
        .filter(|name| segment(name))
        .ok_or("SSH 用户配置无效")?;
    let port = values("port")
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port > 0)
        .ok_or("SSH 端口无效")?;
    if ["proxyjump", "proxycommand"]
        .into_iter()
        .any(|option| values(option).any(|value| value != "none"))
    {
        return Err("现有 SSH 跳板配置需单独迁移".into());
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .ok_or("无法定位本机 SSH 凭据目录")?;
    let expand = |value: &str| {
        let value = value.trim_matches('"');
        if let Some(relative) = value
            .strip_prefix("~/")
            .or_else(|| value.strip_prefix("~\\"))
        {
            home.join(relative)
        } else {
            PathBuf::from(value)
        }
    };
    let keys: Vec<_> = values("identityfile")
        .map(expand)
        .filter(|path| path.is_absolute() && path.is_file())
        .collect();
    let [key_path] = keys.as_slice() else {
        return Err("SSH 配置应有一份可明确识别的私钥文件".into());
    };
    let key_path = key_path.canonicalize().map_err(|_| "找不到原 SSH 私钥")?;
    if key_path.starts_with(legacy_root) {
        return Err("SSH 私钥应位于原项目外部的凭据目录".into());
    }
    let key =
        String::from_utf8(read_file(&key_path, 24 * 1024)?).map_err(|_| "SSH 私钥格式无效")?;
    if !key.starts_with("-----BEGIN ") || !key.contains("PRIVATE KEY-----") {
        return Err("SSH 私钥格式无效".into());
    }

    let pattern = if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    };
    let lookup = values("hostkeyalias")
        .find(|alias| *alias != "none")
        .unwrap_or(&pattern);
    let configured_files = values("userknownhostsfile").chain(values("globalknownhostsfile"));
    let mut known_hosts = HashSet::new();
    for entry in configured_files {
        let path = expand(entry);
        if path.is_absolute() && path.is_file() {
            known_hosts.insert(path);
        } else {
            known_hosts.extend(
                entry
                    .split_whitespace()
                    .map(expand)
                    .filter(|path| path.is_absolute() && path.is_file()),
            );
        }
    }
    let mut pins = HashSet::new();
    for known_hosts in known_hosts {
        let output = quiet(
            Command::new(ssh_tool("ssh-keygen")?)
                .args(["-F", lookup, "-f"])
                .arg(&known_hosts),
        )
        .output()
        .map_err(|_| "无法读取已信任的 SSH 主机密钥")?;
        if output.stdout.len() > 64 * 1024 {
            return Err("SSH 主机密钥输出超过限制".into());
        }
        let entries = std::str::from_utf8(&output.stdout).map_err(|_| "SSH 主机密钥格式无效")?;
        if entries.lines().any(|line| line.starts_with('@')) {
            return Err("检测到 SSH 主机证书或撤销记录，需单独核对".into());
        }
        for line in entries.lines().filter(|line| !line.starts_with('#')) {
            let mut fields = line.split_whitespace();
            let (Some(_), Some(kind), Some(data)) = (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            if segment(kind) && data.len() <= 8192 && STANDARD.decode(data).is_ok() {
                pins.insert(format!("{pattern} {kind} {data}"));
            }
        }
    }
    if pins.is_empty() {
        return Err("缺少既有可信主机密钥，未进行首次信任".into());
    }
    let mut pins: Vec<_> = pins.into_iter().collect();
    pins.sort();
    Ok((host.into(), user.into(), port, key, pins.join("\n") + "\n"))
}

fn profile(root: &Path) -> AppResult<Profile> {
    let legacy: Legacy = json_file(&root.join("workbench/servers.json"), 64 * 1024)?;
    if legacy.servers.is_empty() || legacy.servers.len() > 32 {
        return Err("原服务器数量超出可导入范围".into());
    }
    let mut ids = HashSet::new();
    let mut machines = HashSet::new();
    let mut servers = Vec::new();
    for item in legacy.servers {
        if !segment(&item.id)
            || !ids.insert(item.id.clone())
            || item.name.trim().is_empty()
            || item.name.chars().count() > 100
        {
            return Err("原服务器名称或编号无效".into());
        }
        if item.links.len() > 16
            || item
                .links
                .iter()
                .any(|link| crate::content::web_url(&link.url).is_err())
        {
            return Err("原服务入口配置无效".into());
        }
        let connection = match item.connection {
            LegacyConnection::Aliyun {
                address,
                ssh_alias,
                services,
                health_urls,
            } => {
                if services.len() > 32
                    || services.iter().any(|name| !segment(name))
                    || health_urls.len() > 24
                    || health_urls.iter().any(|entry| {
                        url::Url::parse(&entry.url).ok().is_none_or(|url| {
                            !matches!(url.scheme(), "http" | "https")
                                || !matches!(
                                    url.host_str(),
                                    Some("127.0.0.1" | "localhost" | "[::1]")
                                )
                                || !url.username().is_empty()
                                || url.password().is_some()
                        })
                    })
                {
                    return Err("原阿里云服务检查配置无效".into());
                }
                let (host, user, port, key, pinned) = ssh_credentials(&address, &ssh_alias, root)?;
                ConnectionInfo::Aliyun {
                    host,
                    user,
                    port,
                    key,
                    pinned,
                    services,
                    health_urls,
                }
            }
            LegacyConnection::Instabox {
                machine_id,
                org_id,
                credential,
                account,
            } => {
                if !segment(&machine_id)
                    || !segment(&org_id)
                    || !machines.insert(machine_id.clone())
                {
                    return Err("Instabox 服务器或工作区编号无效".into());
                }
                let path = root
                    .join(credential)
                    .canonicalize()
                    .map_err(|_| "找不到原 Instabox 凭据文件")?;
                if !path.starts_with(
                    root.join("secrets")
                        .canonicalize()
                        .map_err(|_| "原凭据目录无效")?,
                ) {
                    return Err("Instabox 凭据应处于原项目的本地凭据目录".into());
                }
                let credential: Value = json_file(&path, 16 * 1024)?;
                let key = credential["key"]
                    .as_str()
                    .filter(|value| {
                        !value.is_empty()
                            && value.len() <= 4096
                            && value.bytes().all(|b| b.is_ascii_graphic())
                    })
                    .ok_or("原 Instabox 凭据格式无效")?
                    .to_owned();
                ConnectionInfo::Instabox {
                    machine_id,
                    org_id,
                    account,
                    key,
                }
            }
        };
        servers.push(Server {
            id: item.id,
            name: item.name,
            role: item.role,
            links: item.links,
            connection,
        });
    }
    Ok(Profile {
        interval: legacy.interval_seconds.clamp(30, 3600),
        retention: legacy.retention_days.clamp(1, 30),
        servers,
    })
}

pub(super) fn schema(db: &Connection) -> AppResult<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS cloud_snapshots(id TEXT PRIMARY KEY, payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS cloud_samples(id TEXT NOT NULL, ts REAL NOT NULL, cpu REAL, memory REAL, disk REAL, rx REAL, tx REAL, PRIMARY KEY(id, ts));
        CREATE TABLE IF NOT EXISTS cloud_events(seq INTEGER PRIMARY KEY, ts REAL NOT NULL, id TEXT NOT NULL, level TEXT, message TEXT);
        CREATE TABLE IF NOT EXISTS cloud_wake_requests(request TEXT PRIMARY KEY, server TEXT NOT NULL, created INTEGER NOT NULL);")
        .map_err(err)
}

pub(super) fn load(store: &Store) -> AppResult<Option<Profile>> {
    let raw = {
        let db = store.db.lock().map_err(err)?;
        saved(&db)?
    };
    raw.as_deref().map(unseal).transpose()
}

pub(super) fn saved(db: &Connection) -> AppResult<Option<String>> {
    db.query_row(
        "SELECT value FROM app_settings WHERE key=?1",
        [VAULT],
        |row| row.get(0),
    )
    .optional()
    .map_err(err)
}

pub(super) fn unseal(raw: &str) -> AppResult<Profile> {
    let encrypted = STANDARD.decode(raw).map_err(|_| "服务器凭据密文无效")?;
    let mut plain = local_dpapi(&encrypted, ENTROPY, true)?;
    let result = serde_json::from_slice(&plain).map_err(|_| "服务器配置密文无效".into());
    plain.fill(0);
    result
}

pub(super) fn seal(profile: &Profile) -> AppResult<String> {
    let mut plain = serde_json::to_vec(profile).map_err(|_| "无法编码服务器配置")?;
    let encrypted = local_dpapi(&plain, ENTROPY, false);
    plain.fill(0);
    Ok(STANDARD.encode(encrypted?))
}

pub(super) fn repair_ssh(store: &Store, source: &Path) -> AppResult<Value> {
    let old = store.preference(VAULT).ok_or("未导入服务器配置")?;
    let mut existing = load(store)?.ok_or("未导入服务器配置")?;
    let source = source.canonicalize().map_err(|_| "找不到原工作台目录")?;
    let legacy: Legacy = json_file(&source.join("workbench/servers.json"), 64 * 1024)?;
    let mut repaired = 0;
    for server in &mut existing.servers {
        let ConnectionInfo::Aliyun {
            host,
            user,
            port,
            key,
            pinned,
            ..
        } = &mut server.connection
        else {
            continue;
        };
        let prior = legacy
            .servers
            .iter()
            .find(|prior| prior.id == server.id)
            .ok_or("原阿里云服务器标识不符")?;
        let LegacyConnection::Aliyun {
            address, ssh_alias, ..
        } = &prior.connection
        else {
            return Err("原连接类型不符".into());
        };
        let (target, account, endpoint, selected, trusted) =
            ssh_credentials(address, ssh_alias, &source)?;
        if *host != target || *user != account || *port != endpoint || *key != selected {
            return Err("SSH 服务器或身份凭据已变化，未覆盖".into());
        }
        *pinned = trusted;
        repaired += 1;
    }
    let ciphertext = seal(&existing)?;
    let db = store.db.lock().map_err(err)?;
    if db
        .execute(
            "UPDATE app_settings SET value=?1 WHERE key=?2 AND value=?3",
            params![ciphertext, VAULT, old],
        )
        .map_err(err)?
        != 1
    {
        return Err("服务器配置已改变，未覆盖".into());
    }
    Ok(json!({"sshTrustUpdated":repaired}))
}

fn copy_history(
    target: &rusqlite::Transaction<'_>,
    source: &Path,
    ids: &HashSet<&str>,
    retention: u64,
) -> AppResult<Value> {
    if !source.is_file() {
        return Ok(json!({"snapshots":0,"samples":0,"events":0}));
    }
    let old = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| "无法只读打开原工作台历史")?;
    let cutoff = (crate::store::now_ms() as f64 / 1000.) - retention as f64 * 86400.;
    let mut snapshots = 0;
    let mut query = old
        .prepare("SELECT id,payload FROM snapshots")
        .map_err(err)?;
    let mut rows = query.query([]).map_err(err)?;
    while let Some(row) = rows.next().map_err(err)? {
        let id: String = row.get(0).map_err(err)?;
        let payload: String = row.get(1).map_err(err)?;
        if ids.contains(id.as_str()) && payload.len() <= 256 * 1024 {
            let mut value: Value =
                serde_json::from_str(&payload).map_err(|_| "原工作台快照格式无效")?;
            if !value.is_object() {
                return Err("原工作台快照格式无效".into());
            }
            value["checked_at"] = Value::Null;
            value["error"] = Value::Null;
            target
                .execute(
                    "INSERT INTO cloud_snapshots VALUES(?1,?2)",
                    params![id, value.to_string()],
                )
                .map_err(err)?;
            snapshots += 1;
        }
    }
    let mut samples = 0;
    let mut query = old
        .prepare("SELECT id,ts,cpu,memory,disk,rx,tx FROM samples WHERE ts>=?1")
        .map_err(err)?;
    let mut rows = query.query([cutoff]).map_err(err)?;
    while let Some(row) = rows.next().map_err(err)? {
        let id: String = row.get(0).map_err(err)?;
        if ids.contains(id.as_str()) {
            target
                .execute(
                    "INSERT INTO cloud_samples VALUES(?1,?2,?3,?4,?5,?6,?7)",
                    params![
                        id,
                        row.get::<_, f64>(1).map_err(err)?,
                        row.get::<_, Option<f64>>(2).map_err(err)?,
                        row.get::<_, Option<f64>>(3).map_err(err)?,
                        row.get::<_, Option<f64>>(4).map_err(err)?,
                        row.get::<_, Option<f64>>(5).map_err(err)?,
                        row.get::<_, Option<f64>>(6).map_err(err)?
                    ],
                )
                .map_err(err)?;
            samples += 1;
        }
    }
    let mut events = 0;
    let mut query = old
        .prepare("SELECT ts,id,level,message FROM events WHERE ts>=?1 ORDER BY seq")
        .map_err(err)?;
    let mut rows = query.query([cutoff]).map_err(err)?;
    while let Some(row) = rows.next().map_err(err)? {
        let id: String = row.get(1).map_err(err)?;
        if ids.contains(id.as_str()) {
            target
                .execute(
                    "INSERT INTO cloud_events(ts,id,level,message) VALUES(?1,?2,?3,?4)",
                    params![
                        row.get::<_, f64>(0).map_err(err)?,
                        id,
                        row.get::<_, Option<String>>(2).map_err(err)?,
                        row.get::<_, Option<String>>(3).map_err(err)?
                    ],
                )
                .map_err(err)?;
            events += 1;
        }
    }
    Ok(json!({"snapshots":snapshots,"samples":samples,"events":events}))
}

pub(crate) fn run(store: &Store, source: &Path) -> AppResult<Value> {
    if store.preference(VAULT).is_some() {
        return Ok(json!({"alreadyImported":true}));
    }
    let source = source.canonicalize().map_err(|_| "找不到原工作台目录")?;
    let profile = profile(&source)?;
    let ciphertext = seal(&profile)?;
    let ids: HashSet<_> = profile
        .servers
        .iter()
        .map(|server| server.id.as_str())
        .collect();
    let mut db = store.db.lock().map_err(err)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    if tx
        .query_row("SELECT 1 FROM app_settings WHERE key=?1", [VAULT], |_| {
            Ok(())
        })
        .optional()
        .map_err(err)?
        .is_some()
    {
        return Ok(json!({"alreadyImported":true}));
    }
    schema(&tx)?;
    let history = copy_history(
        &tx,
        &source.join("local/workbench.sqlite3"),
        &ids,
        profile.retention,
    )?;
    tx.execute(
        "INSERT INTO app_settings VALUES(?1,?2)",
        params![VAULT, ciphertext],
    )
    .map_err(err)?;
    tx.execute(
        "DELETE FROM app_settings WHERE key='cloud_workbench_binding'",
        [],
    )
    .map_err(err)?;
    tx.commit().map_err(err)?;
    let instabox = profile
        .servers
        .iter()
        .filter(|server| matches!(server.connection, ConnectionInfo::Instabox { .. }))
        .count();
    Ok(
        json!({"servers":profile.servers.len(),"instabox":instabox,"aliyun":profile.servers.len()-instabox,"history":history}),
    )
}
