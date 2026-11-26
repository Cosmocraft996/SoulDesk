use super::import::{
    saved, schema, seal, segment, unseal, ConnectionInfo, Health, Profile, Server, VAULT,
};
use crate::store::{err, AppResult, Store};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, TransactionBehavior};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

const MAX_SERVERS: usize = 32;
pub(super) const MAX_IMPORT_BYTES: usize = 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    name: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    links: Vec<Health>,
    connection: Target,
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "provider",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Target {
    Aliyun {
        host: String,
        user: String,
        port: u16,
        #[serde(default)]
        services: Vec<String>,
        #[serde(default)]
        health_urls: Vec<Health>,
    },
    Instabox {
        machine_id: String,
        org_id: String,
        #[serde(default)]
        account: Option<String>,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Portable {
    schema_version: u32,
    interval_seconds: u64,
    retention_days: u64,
    servers: Vec<BackupServer>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupServer {
    server: Config,
    credentials: Credentials,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Credentials {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    api_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    private_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    host_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    revision: String,
    #[serde(default)]
    id: Option<String>,
    server: Config,
    #[serde(default)]
    credentials: Credentials,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Remove {
    revision: String,
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reorder {
    revision: String,
    ids: Vec<String>,
}

fn decode<T: DeserializeOwned>(value: Value) -> AppResult<T> {
    // 无效字段的原值可能带凭据，不通过错误消息回传。
    serde_json::from_value(value).map_err(|_| "服务器操作参数无效".into())
}

fn revision(raw: Option<&str>) -> String {
    raw.map_or_else(
        || "empty".into(),
        |value| format!("{:x}", Sha256::digest(value)),
    )
}

fn clean(value: &mut String, max: usize, message: &'static str) -> AppResult<()> {
    *value = value.trim().to_owned();
    if value.is_empty() || value.chars().count() > max || value.chars().any(char::is_control) {
        return Err(message.into());
    }
    Ok(())
}

fn clean_optional(value: &mut Option<String>, message: &'static str) -> AppResult<()> {
    if value.as_ref().is_some_and(|text| text.trim().is_empty()) {
        *value = None;
    }
    if let Some(value) = value {
        clean(value, 100, message)?;
    }
    Ok(())
}

fn check_links(links: &mut [Health], loopback: bool) -> AppResult<()> {
    for link in links {
        clean(&mut link.name, 100, "服务入口或健康检查名称无效")?;
        let url = crate::content::web_url(link.url.trim())?;
        if loopback && !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")) {
            return Err("健康检查只接受被监控服务器上的本机回环地址".into());
        }
        link.url = url.into();
    }
    Ok(())
}

fn ssh_host(value: &str) -> bool {
    value.parse::<std::net::IpAddr>().is_ok()
        || (value.len() <= 253
            && value.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label.as_bytes()[0].is_ascii_alphanumeric()
                    && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            }))
}

impl Config {
    fn validate(mut self) -> AppResult<Self> {
        clean(&mut self.name, 100, "服务器名称不能为空或超过 100 个字符")?;
        clean_optional(&mut self.role, "服务器用途无效")?;
        if self.links.len() > 16 {
            return Err("一台服务器最多可保存 16 个服务入口".into());
        }
        check_links(&mut self.links, false)?;
        match &mut self.connection {
            Target::Aliyun {
                host,
                user,
                port,
                services,
                health_urls,
            } => {
                *host = host.trim().to_ascii_lowercase();
                *user = user.trim().to_owned();
                if !ssh_host(host) || !segment(user) || *port == 0 {
                    return Err("请填写有效的 SSH 主机地址、用户名和端口".into());
                }
                if services.len() > 32
                    || services.iter().any(|name| !segment(name))
                    || health_urls.len() > 24
                {
                    return Err("服务名称、服务数量或健康检查数量无效".into());
                }
                check_links(health_urls, true)?;
            }
            Target::Instabox {
                machine_id,
                org_id,
                account,
            } => {
                *machine_id = machine_id.trim().to_owned();
                *org_id = org_id.trim().to_owned();
                if !segment(machine_id) || !segment(org_id) {
                    return Err("请填写有效的 Instabox 实例编号和工作区编号".into());
                }
                clean_optional(account, "账号备注无效")?;
            }
        }
        Ok(self)
    }
}

impl From<&Server> for Config {
    fn from(server: &Server) -> Self {
        let connection = match &server.connection {
            ConnectionInfo::Aliyun {
                host,
                user,
                port,
                services,
                health_urls,
                ..
            } => Target::Aliyun {
                host: host.clone(),
                user: user.clone(),
                port: *port,
                services: services.clone(),
                health_urls: health_urls.clone(),
            },
            ConnectionInfo::Instabox {
                machine_id,
                org_id,
                account,
                ..
            } => Target::Instabox {
                machine_id: machine_id.clone(),
                org_id: org_id.clone(),
                account: account.clone(),
            },
        };
        Self {
            name: server.name.clone(),
            role: server.role.clone(),
            links: server.links.clone(),
            connection,
        }
    }
}

impl Target {
    fn matches(&self, connection: &ConnectionInfo) -> bool {
        match (self, connection) {
            (
                Self::Aliyun { host, port, .. },
                ConnectionInfo::Aliyun {
                    host: current,
                    port: current_port,
                    ..
                },
            ) => host.eq_ignore_ascii_case(current) && port == current_port,
            (
                Self::Instabox {
                    machine_id, org_id, ..
                },
                ConnectionInfo::Instabox {
                    machine_id: current,
                    org_id: current_org,
                    ..
                },
            ) => machine_id == current && org_id == current_org,
            _ => false,
        }
    }
}

fn host_pins(host: &str, port: u16, public_keys: &str) -> AppResult<String> {
    let invalid = "请填写可信的 SSH 主机公钥，每行以公钥类型开头";
    if public_keys.len() > 32 * 1024 || public_keys.lines().count() > 8 {
        return Err(invalid.into());
    }
    let host = if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    };
    let mut pins = String::new();
    for line in public_keys.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let kind = fields
            .next()
            .filter(|kind| {
                matches!(
                    *kind,
                    "ssh-ed25519"
                        | "ssh-rsa"
                        | "ecdsa-sha2-nistp256"
                        | "ecdsa-sha2-nistp384"
                        | "ecdsa-sha2-nistp521"
                )
            })
            .ok_or(invalid)?;
        let data = fields
            .next()
            .filter(|data| data.len() <= 8192)
            .ok_or(invalid)?;
        let decoded = STANDARD.decode(data).map_err(|_| invalid)?;
        let prefix = (kind.len() as u32).to_be_bytes();
        if !decoded.starts_with(&prefix)
            || decoded.get(4..4 + kind.len()) != Some(kind.as_bytes())
            || decoded.len() <= 4 + kind.len()
        {
            return Err(invalid.into());
        }
        pins.push_str(&format!("{host} {kind} {data}\n"));
    }
    Ok(pins)
}

fn build(config: Config, previous: Option<&Server>, credentials: Credentials) -> AppResult<Server> {
    let Credentials {
        api_key,
        private_key,
        host_key,
    } = credentials;
    let previous = previous.filter(|server| config.connection.matches(&server.connection));
    let connection = match config.connection {
        Target::Aliyun {
            host,
            user,
            port,
            services,
            health_urls,
        } => {
            if !api_key.is_empty() {
                return Err("SSH 服务器不使用 API 密钥".into());
            }
            let (old_key, old_pin) = match previous.map(|server| &server.connection) {
                Some(ConnectionInfo::Aliyun {
                    user: old_user,
                    key,
                    pinned,
                    ..
                }) => (
                    if *old_user == user { key.as_str() } else { "" },
                    pinned.as_str(),
                ),
                _ => ("", ""),
            };
            let private_key = private_key.trim();
            let key = if private_key.is_empty() {
                old_key.to_owned()
            } else {
                let kind = [
                    "OPENSSH PRIVATE KEY",
                    "RSA PRIVATE KEY",
                    "EC PRIVATE KEY",
                    "PRIVATE KEY",
                ]
                .into_iter()
                .any(|kind| {
                    private_key.lines().next() == Some(&format!("-----BEGIN {kind}-----"))
                        && private_key.lines().last() == Some(&format!("-----END {kind}-----"))
                });
                if private_key.len() > 24 * 1024 || !kind || private_key.contains('\0') {
                    return Err("SSH 私钥文件格式有误或超过 24 KiB".into());
                }
                format!("{private_key}\n")
            };
            let pinned = if host_key.trim().is_empty() {
                old_pin.to_owned()
            } else {
                host_pins(&host, port, &host_key)?
            };
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
        Target::Instabox {
            machine_id,
            org_id,
            account,
        } => {
            if !private_key.is_empty() || !host_key.is_empty() {
                return Err("Instabox 不使用 SSH 凭据".into());
            }
            let api_key = api_key.trim();
            let key = if api_key.is_empty() {
                match previous.map(|server| &server.connection) {
                    Some(ConnectionInfo::Instabox { key, .. }) => key.clone(),
                    _ => String::new(),
                }
            } else {
                if api_key.len() > 4096 || !api_key.bytes().all(|b| b.is_ascii_graphic()) {
                    return Err("Instabox API 密钥格式无效".into());
                }
                api_key.into()
            };
            ConnectionInfo::Instabox {
                machine_id,
                org_id,
                account,
                key,
            }
        }
    };
    Ok(Server {
        id: previous.map_or_else(|| Uuid::new_v4().to_string(), |server| server.id.clone()),
        name: config.name,
        role: config.role,
        links: config.links,
        connection,
    })
}

fn public_list(profile: &Profile, revision: String) -> Value {
    let servers: Vec<_> = profile.servers.iter().map(|server| {
        let (has_key, has_host_key) = match &server.connection {
            ConnectionInfo::Aliyun { key, pinned, .. } => (!key.is_empty(), !pinned.is_empty()),
            ConnectionInfo::Instabox { key, .. } => (!key.is_empty(), false),
        };
        json!({"id":server.id,"server":Config::from(server),"hasKey":has_key,"hasHostKey":has_host_key})
    }).collect();
    json!({"revision":revision,"servers":servers})
}

pub(super) fn export_backup(store: &Store) -> AppResult<Vec<u8>> {
    let profile = super::import::load(store)?.unwrap_or_default();
    let servers = profile
        .servers
        .iter()
        .map(|server| {
            let credentials = match &server.connection {
                ConnectionInfo::Aliyun { key, pinned, .. } => Credentials {
                    private_key: key.clone(),
                    host_key: pinned
                        .lines()
                        .filter_map(|line| line.split_once(' ').map(|(_, key)| key))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    ..Credentials::default()
                },
                ConnectionInfo::Instabox { key, .. } => Credentials {
                    api_key: key.clone(),
                    ..Credentials::default()
                },
            };
            BackupServer {
                server: Config::from(server),
                credentials,
            }
        })
        .collect();
    serde_json::to_vec_pretty(&Portable {
        schema_version: 1,
        interval_seconds: profile.interval,
        retention_days: profile.retention,
        servers,
    })
    .map_err(|_| "无法生成服务器备份".into())
}

fn update(
    store: &Store,
    expected: Option<&str>,
    change: impl FnOnce(&mut Profile) -> AppResult<Value>,
) -> AppResult<Value> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    let raw = saved(&tx)?;
    if expected.is_some_and(|expected| revision(raw.as_deref()) != expected) {
        return Err("CONFLICT: 服务器清单已变化，请刷新后重试".into());
    }
    let mut profile = raw.as_deref().map(unseal).transpose()?.unwrap_or_default();
    let mut old_ids: Vec<_> = profile
        .servers
        .iter()
        .map(|server| server.id.clone())
        .collect();
    let result = change(&mut profile)?;
    old_ids.retain(|id| !profile.servers.iter().any(|server| &server.id == id));
    let ciphertext = seal(&profile)?;
    schema(&tx)?;
    for id in old_ids {
        tx.execute("DELETE FROM cloud_snapshots WHERE id=?1", [id])
            .map_err(err)?;
    }
    tx.execute(
        "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![VAULT, ciphertext],
    ).map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(json!({"list":public_list(&profile, revision(Some(&ciphertext))),"result":result}))
}

pub(super) fn list(store: &Store) -> AppResult<Value> {
    let raw = {
        let db = store.db.lock().map_err(err)?;
        saved(&db)?
    };
    let profile = raw.as_deref().map(unseal).transpose()?.unwrap_or_default();
    Ok(public_list(&profile, revision(raw.as_deref())))
}

pub(super) fn import_backup(store: &Store, bytes: &[u8]) -> AppResult<Value> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err("导入文件超过 1 MiB".into());
    }
    let portable: Portable =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))
            .map_err(|_| "服务器备份 JSON 格式无效".to_owned())?;
    if portable.schema_version != 1
        || portable.servers.len() > MAX_SERVERS
        || !(30..=3600).contains(&portable.interval_seconds)
        || !(1..=30).contains(&portable.retention_days)
    {
        return Err("备份版本、采集设置或服务器数量无效".into());
    }
    let incoming = portable
        .servers
        .into_iter()
        .map(|entry| {
            let config = entry.server.validate()?;
            build(config, None, entry.credentials)
        })
        .collect::<AppResult<Vec<_>>>()?;
    let result = update(store, None, |profile| {
        if profile.servers.is_empty() {
            profile.interval = portable.interval_seconds;
            profile.retention = portable.retention_days;
        }
        let mut added = 0;
        let mut skipped = 0;
        let mut ready = 0;
        for server in incoming {
            let config = Config::from(&server);
            if profile
                .servers
                .iter()
                .any(|current| config.connection.matches(&current.connection))
            {
                skipped += 1;
                continue;
            }
            if profile.servers.len() >= MAX_SERVERS {
                return Err("合并后将超过 32 台服务器，未导入文件".into());
            }
            ready += usize::from(server.connection.ready());
            profile.servers.push(server);
            added += 1;
        }
        Ok(json!({"added":added,"skipped":skipped,"ready":ready}))
    })?;
    Ok(result["result"].clone())
}

pub(super) fn write(store: &Store, method: &str, args: Value) -> AppResult<Value> {
    match method {
        "cloudWorkbench.manage.save" => {
            let Save {
                revision,
                id,
                server,
                credentials,
            } = decode(args)?;
            let server = server.validate()?;
            update(store, Some(&revision), |profile| {
                let existing = id
                    .as_ref()
                    .map(|id| {
                        profile
                            .servers
                            .iter()
                            .position(|server| &server.id == id)
                            .ok_or("服务器不存在")
                    })
                    .transpose()?;
                if profile.servers.iter().enumerate().any(|(index, current)| {
                    Some(index) != existing && server.connection.matches(&current.connection)
                }) {
                    return Err("清单里已有同一服务器".into());
                }
                let server = build(
                    server,
                    existing.map(|index| &profile.servers[index]),
                    credentials,
                )?;
                let result = json!({"id":server.id,"ready":server.connection.ready()});
                if let Some(index) = existing {
                    profile.servers[index] = server;
                } else {
                    if profile.servers.len() >= MAX_SERVERS {
                        return Err("服务器清单最多允许 32 台".into());
                    }
                    profile.servers.push(server);
                }
                Ok(result)
            })
        }
        "cloudWorkbench.manage.remove" => {
            let Remove { revision, id } = decode(args)?;
            update(store, Some(&revision), |profile| {
                let index = profile
                    .servers
                    .iter()
                    .position(|server| server.id == id)
                    .ok_or("服务器不存在")?;
                profile.servers.remove(index);
                Ok(Value::Null)
            })
        }
        "cloudWorkbench.manage.reorder" => {
            let Reorder { revision, ids } = decode(args)?;
            update(store, Some(&revision), |profile| {
                let order: HashMap<_, _> = ids.iter().enumerate().map(|(i, id)| (id, i)).collect();
                if ids.len() != profile.servers.len()
                    || order.len() != ids.len()
                    || profile
                        .servers
                        .iter()
                        .any(|server| !order.contains_key(&server.id))
                {
                    return Err("排序须包含完整且未重复的服务器清单".into());
                }
                profile.servers.sort_by_key(|server| order[&server.id]);
                Ok(Value::Null)
            })
        }
        _ => Err("未开放的服务器管理操作".into()),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::super::import;
    use super::*;

    fn store() -> Store {
        Store::open(std::env::temp_dir().join(format!("souldesk-cloud-manage-{}", Uuid::new_v4())))
            .unwrap()
    }

    fn action(store: &Store, name: &str, mut args: Value) -> AppResult<Value> {
        let current = list(store)?;
        args["revision"] = current["revision"].clone();
        write(store, &format!("cloudWorkbench.manage.{name}"), args)
    }

    #[test]
    fn cloud_full_backup_restores_ready_server_without_returning_secrets_in_list() {
        let store = store();
        assert_eq!(list(&store).unwrap()["servers"], json!([]));
        let server = json!({
            "name":"演示实例","connection":{"provider":"instabox","machineId":"demo01","orgId":"workspace01"},
            "links":[{"name":"入口","url":"https://example.invalid/panel?token=private-test-value#login"}]
        });
        let saved = action(
            &store,
            "save",
            json!({"server":server,"credentials":{"apiKey":"example-test-only-key"}}),
        )
        .unwrap();
        let id = saved["result"]["id"].as_str().unwrap();
        assert_eq!(saved["result"]["ready"], true);
        assert!(!saved.to_string().contains("example-test-only-key"));
        assert_eq!(saved["list"]["servers"][0]["hasKey"], true);
        let mut change = server.clone();
        change["name"] = json!("改名");
        let renamed = action(&store, "save", json!({"id":id,"server":change})).unwrap();
        assert_eq!(renamed["result"], json!({"id":id,"ready":true}));
        assert_eq!(
            revision(import::saved(&store.db.lock().unwrap()).unwrap().as_deref()),
            renamed["list"]["revision"]
        );

        let file = export_backup(&store).unwrap();
        let portable: Value = serde_json::from_slice(&file).unwrap();
        assert_eq!(
            portable["servers"][0]["credentials"]["apiKey"],
            "example-test-only-key"
        );
        assert_eq!(
            portable["servers"][0]["server"]["links"][0]["url"],
            "https://example.invalid/panel?token=private-test-value#login"
        );
        assert_eq!(
            import_backup(&store, &file).unwrap(),
            json!({"added":0,"skipped":1,"ready":0})
        );
        let fresh = self::store();
        assert_eq!(
            import_backup(&fresh, &file).unwrap(),
            json!({"added":1,"skipped":0,"ready":1})
        );
        let imported = list(&fresh).unwrap();
        assert!(!imported.to_string().contains("example-test-only-key"));
        let server = &imported["servers"][0];
        assert_eq!(server["hasKey"], true);
        assert_ne!(server["id"], id);
        assert!(import::load(&fresh).unwrap().unwrap().servers[0]
            .connection
            .ready());

        change["connection"]["orgId"] = json!("workspace02");
        let moved = action(&store, "save", json!({"id":id,"server":change})).unwrap();
        assert_ne!(moved["result"]["id"], id);
        assert_eq!(moved["result"]["ready"], false);
    }

    #[test]
    fn cloud_order_requires_current_full_list_and_remove_preserves_historical_samples() {
        let store = store();
        let make = |name| json!({"server":{"name":name,"connection":{"provider":"aliyun","host":format!("{name}.example.invalid"),"user":"demo","port":22}}});
        let first = action(&store, "save", make("one")).unwrap();
        let second = action(&store, "save", make("two")).unwrap();
        let a = first["result"]["id"].as_str().unwrap();
        let b = second["result"]["id"].as_str().unwrap();
        assert!(action(&store, "reorder", json!({"ids":[a,a]})).is_err());
        assert!(write(
            &store,
            "cloudWorkbench.manage.reorder",
            json!({"revision":first["list"]["revision"],"ids":[b,a]})
        )
        .unwrap_err()
        .starts_with("CONFLICT:"));
        let sorted = action(&store, "reorder", json!({"ids":[b,a]})).unwrap();
        assert_eq!(sorted["list"]["servers"][0]["id"], b);
        let root = store.root.clone();
        {
            let db = store.db.lock().unwrap();
            db.execute("INSERT INTO cloud_snapshots VALUES(?1,'{}')", [b])
                .unwrap();
            db.execute("INSERT INTO cloud_samples(id,ts,cpu) VALUES(?1,1,20)", [b])
                .unwrap();
        }
        action(&store, "remove", json!({"id":b})).unwrap();
        drop(store);
        let store = Store::open(root).unwrap();
        assert_eq!(list(&store).unwrap()["servers"][0]["id"], a);
        let db = store.db.lock().unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM cloud_snapshots", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM cloud_samples", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn cloud_import_rejects_unknown_fields_and_applies_no_partial_changes() {
        let store = store();
        let good = json!({"server":{"name":"示例","connection":{"provider":"instabox","machineId":"demo","orgId":"org"}},"credentials":{"apiKey":"test-only-key"}});
        let mut invalid = good.clone();
        invalid["credentials"]["unexpected"] = json!("test-only-key");
        for version in [
            json!({"schemaVersion":1,"intervalSeconds":60,"retentionDays":7,"servers":[good,invalid]}),
            json!({"schemaVersion":7,"intervalSeconds":60,"retentionDays":7,"servers":[good]}),
        ] {
            assert!(import_backup(&store, &serde_json::to_vec(&version).unwrap()).is_err());
        }
        assert_eq!(list(&store).unwrap()["revision"], "empty");
    }

    #[test]
    fn cloud_backup_preserves_ssh_private_key_public_pin_and_sampling_settings() {
        let store = store();
        let mut wire = (11_u32).to_be_bytes().to_vec();
        wire.extend_from_slice(b"ssh-ed25519");
        wire.extend([7_u8; 36]);
        let public_key = format!("ssh-ed25519 {}", STANDARD.encode(wire));
        let private_key =
            "-----BEGIN OPENSSH PRIVATE KEY-----\nTEST\n-----END OPENSSH PRIVATE KEY-----";
        let server = json!({"server":{"name":"演示 SSH","connection":{"provider":"aliyun","host":"sample.example.invalid","user":"demo","port":2222,"healthUrls":[{"name":"状态","url":"http://127.0.0.1:8123/health"}]}},"credentials":{"privateKey":private_key,"hostKey":public_key}});
        let backup =
            json!({"schemaVersion":1,"intervalSeconds":90,"retentionDays":10,"servers":[server]});
        assert_eq!(
            import_backup(&store, &serde_json::to_vec(&backup).unwrap()).unwrap()["ready"],
            1
        );
        let restored = import::load(&store).unwrap().unwrap();
        assert_eq!((restored.interval, restored.retention), (90, 10));
        let ConnectionInfo::Aliyun { key, pinned, .. } = &restored.servers[0].connection else {
            panic!("应恢复 SSH 配置");
        };
        assert_eq!(key.trim_end(), private_key);
        assert_eq!(
            pinned,
            &format!("[sample.example.invalid]:2222 {public_key}\n")
        );
        let reexport: Value = serde_json::from_slice(&export_backup(&store).unwrap()).unwrap();
        assert_eq!(reexport["servers"][0]["credentials"]["hostKey"], public_key);
    }
}
