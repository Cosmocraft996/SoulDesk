//! 网页AI助手的有限配置；没有网页、登录、快捷键或系统窗口副作用。
use crate::store::{err, AppResult, Store};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Platform {
    Chatgpt,
    Grok,
    Gemini,
    Deepseek,
    Kimi,
    Qwen,
    DouBao,
    YuanBao,
}
impl Platform {
    pub(crate) fn url(self) -> &'static str {
        match self {
            Self::Chatgpt => "https://chatgpt.com/",
            Self::Grok => "https://grok.com/",
            Self::Gemini => "https://gemini.google.com/",
            Self::Deepseek => "https://chat.deepseek.com",
            Self::Kimi => "https://kimi.moonshot.cn/",
            Self::Qwen => "https://www.qianwen.com/",
            Self::DouBao => "https://www.doubao.com/chat/",
            Self::YuanBao => "https://yuanbao.tencent.com/chat/",
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Position {
    Left,
    Right,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Protocol {
    Http,
    Https,
    #[serde(alias = "sock4")]
    Socks4,
    #[serde(alias = "sock5")]
    Socks5,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "lowercase", deny_unknown_fields)]
pub(crate) enum Proxy {
    System,
    Manual {
        protocol: Protocol,
        host: String,
        port: u16,
    },
}
impl<'de> Deserialize<'de> for Proxy {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let Some(object) = value.as_object() else {
            return Err(serde::de::Error::custom("代理须为对象"));
        };
        match object.get("mode").and_then(Value::as_str) {
            Some("system") if object.len() == 1 => Ok(Self::System),
            Some("manual")
                if object.len() == 4
                    && ["protocol", "host", "port"]
                        .iter()
                        .all(|k| object.contains_key(*k)) =>
            {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Manual {
                    protocol: Protocol,
                    host: String,
                    port: u16,
                }
                let mut fields = object.clone();
                fields.remove("mode");
                let parsed: Manual = serde_json::from_value(Value::Object(fields))
                    .map_err(serde::de::Error::custom)?;
                Ok(Self::Manual {
                    protocol: parsed.protocol,
                    host: parsed.host,
                    port: parsed.port,
                })
            }
            _ => Err(serde::de::Error::custom(
                "代理模式或字段无效，不接收凭据或附加字段",
            )),
        }
    }
}
impl Proxy {
    pub(crate) fn browser_arguments(&self) -> AppResult<Option<String>> {
        match self {
            Self::System => Ok(None),
            Self::Manual {
                protocol,
                host,
                port,
            } => {
                if host.is_empty()
                    || host.len() > 253
                    || host.chars().any(|c| c.is_whitespace() || c.is_control())
                    || host.contains(['@', '/', '\\', '?', '#', '"', '\'', ';'])
                    || *port == 0
                {
                    return Err("INVALID_ARGUMENT: 代理主机或端口无效，不接收凭据或附加参数".into());
                }
                let scheme = match protocol {
                    Protocol::Http => "http",
                    Protocol::Https => "https",
                    Protocol::Socks4 => "socks4",
                    Protocol::Socks5 => "socks5",
                };
                let url = url::Url::parse(&format!("{scheme}://{host}:{port}"))
                    .map_err(|_| "INVALID_ARGUMENT: 代理主机无效")?;
                if url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.path() != "" && url.path() != "/"
                    || url.query().is_some()
                    || url.fragment().is_some()
                {
                    return Err("INVALID_ARGUMENT: 代理只接受主机和端口".into());
                }
                Ok(Some(format!("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --proxy-server={}",url.as_str().trim_end_matches('/'))))
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Config {
    pub background_enabled: bool,
    pub shortcut: String,
    pub always_top: bool,
    pub page_count: u8,
    pub platforms: [Platform; 4],
    pub position: Position,
    pub width_ratio: f64,
    pub proxy: Proxy,
    pub tip_progress: u8,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            background_enabled: false,
            shortcut: "Super+Alt+C".into(),
            always_top: false,
            page_count: 1,
            platforms: [
                Platform::Chatgpt,
                Platform::Grok,
                Platform::Gemini,
                Platform::Deepseek,
            ],
            position: Position::Right,
            width_ratio: 0.3,
            proxy: Proxy::System,
            tip_progress: 0,
        }
    }
}
impl Config {
    pub(crate) fn validate(&self) -> AppResult<()> {
        if ![1, 2, 4].contains(&self.page_count)
            || ![0.2, 0.3, 0.4, 0.5].contains(&self.width_ratio)
            || self.tip_progress > 3
            || self.shortcut.len() > 80
            || self.shortcut.chars().any(char::is_control)
            || !self.shortcut.is_empty()
                && self
                    .shortcut
                    .parse::<tauri_plugin_global_shortcut::Shortcut>()
                    .is_err()
            || self.background_enabled && self.shortcut.is_empty()
        {
            return Err("INVALID_ARGUMENT: 助手布局、宽度、说明或快捷键无效".into());
        }
        self.proxy.browser_arguments()?;
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: String,
    fingerprint: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Document {
    pub version: u8,
    pub revision: u64,
    pub config: Config,
    receipts: Vec<Receipt>,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            config: Config::default(),
            receipts: Vec::new(),
        }
    }
}
pub(crate) fn key(instance: &str) -> AppResult<String> {
    if uuid::Uuid::parse_str(instance).is_ok_and(|id| id.to_string() == instance) {
        Ok(format!("assistant.config/{instance}"))
    } else {
        Err("INVALID_ARGUMENT: 助手实例标识无效".into())
    }
}
pub(crate) fn profile_directory(root: &Path, instance: &str) -> AppResult<PathBuf> {
    key(instance)?;
    Ok(root.join("assistant-webviews").join(instance))
}
const DATA_ERROR: &str = "DATA_UNAVAILABLE: 助手配置损坏或版本无效，原数据未覆盖";
const MAX_SAFE: u64 = 9_007_199_254_740_991;
pub(crate) fn load(db: &Connection, instance: &str) -> AppResult<Document> {
    let raw: Option<String> = db
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [key(instance)?],
            |row| row.get(0),
        )
        .optional()
        .map_err(err)?;
    let Some(raw) = raw else {
        return Ok(Document::default());
    };
    if raw.len() > 32768 {
        return Err(DATA_ERROR.into());
    }
    let document: Document = serde_json::from_str(&raw).map_err(|_| DATA_ERROR)?;
    let mut ids = HashSet::new();
    if document.version != 1
        || document.revision > MAX_SAFE
        || document.revision == 0 && document.config != Config::default()
        || document.receipts.len() > 64
        || document.receipts.len() as u64 > document.revision
        || document.config.validate().is_err()
        || document.receipts.iter().any(|r| {
            !uuid::Uuid::parse_str(&r.id).is_ok_and(|id| id.to_string() == r.id)
                || !ids.insert(&r.id)
                || r.fingerprint.len() != 64
                || !r.fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
        })
    {
        return Err(DATA_ERROR.into());
    }
    Ok(document)
}
fn snapshot(document: &Document) -> Value {
    json!({"revision":document.revision,"config":document.config})
}
pub(crate) fn status(store: &Store, instance: &str) -> AppResult<Value> {
    let db = store.db.lock().map_err(err)?;
    Ok(snapshot(&load(&db, instance)?))
}
pub(crate) fn mutate(store: &Store, instance: &str, args: Value) -> AppResult<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Request {
        request_id: String,
        expected_revision: u64,
        config: Config,
    }
    if serde_json::to_vec(&args).map_err(err)?.len() > 8192 {
        return Err("CAPACITY_EXCEEDED: 助手配置请求超过容量".into());
    }
    let request: Request = serde_json::from_value(args)
        .map_err(|_| "INVALID_ARGUMENT: 助手设置只接受requestId/expectedRevision/config")?;
    if !uuid::Uuid::parse_str(&request.request_id)
        .is_ok_and(|id| id.to_string() == request.request_id)
        || request.expected_revision > MAX_SAFE
    {
        return Err("INVALID_ARGUMENT: 请求标识或版本无效".into());
    }
    request.config.validate()?;
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(format!(
            "{}:{}",
            request.expected_revision,
            serde_json::to_string(&request.config).map_err(err)?
        ))
    );
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    let mut document = load(&tx, instance)?;
    if let Some(receipt) = document
        .receipts
        .iter()
        .find(|r| r.id == request.request_id)
    {
        if receipt.fingerprint != fingerprint {
            return Err("REQUEST_CONFLICT: 同一请求标识不能保存不同助手设置".into());
        }
        let mut value = snapshot(&document);
        value["replayed"] = json!(true);
        return Ok(value);
    }
    if document.revision != request.expected_revision {
        return Err("REVISION_CONFLICT: 助手设置已有新版本，请保留草稿后重新读取".into());
    }
    if document.revision >= MAX_SAFE {
        return Err("LIMIT: 助手配置版本已达上限".into());
    }
    document.revision += 1;
    document.config = request.config;
    document.receipts.push(Receipt {
        id: request.request_id,
        fingerprint,
    });
    if document.receipts.len() > 64 {
        document.receipts.remove(0);
    }
    let raw = serde_json::to_string(&document).map_err(err)?;
    if raw.len() > 32768 {
        return Err("CAPACITY_EXCEEDED: 助手配置回执超过容量".into());
    }
    tx.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key(instance)?,raw]).map_err(|_|"STORAGE_ERROR: 助手设置保存失败")?;
    tx.commit().map_err(|_| "STORAGE_ERROR: 助手设置提交失败")?;
    let mut value = snapshot(&document);
    value["replayed"] = json!(false);
    Ok(value)
}
pub(crate) fn remove_data(db: &Connection, instance: &str) -> AppResult<()> {
    db.execute("DELETE FROM app_settings WHERE key=?1", [key(instance)?])
        .map_err(|_| "STORAGE_ERROR: 助手配置清理失败")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn uid() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn store() -> Store {
        Store::open(std::env::temp_dir().join(format!("souldesk-assistant-state-{}", uid())))
            .unwrap()
    }
    fn request(id: &str, revision: u64, config: &Config) -> Value {
        json!({"requestId":id,"expectedRevision":revision,"config":config})
    }
    #[test]
    fn assistant_state_defaults_preserve_original_services_and_keep_background_and_top_off() {
        let c = Config::default();
        c.validate().unwrap();
        assert!(!c.background_enabled && !c.always_top);
        assert_eq!(c.page_count, 1);
        assert_eq!(c.platforms[0].url(), "https://chatgpt.com/");
        assert_eq!(c.platforms[1].url(), "https://grok.com/");
        assert_eq!(c.platforms[2].url(), "https://gemini.google.com/");
        assert_eq!(c.platforms[3].url(), "https://chat.deepseek.com");
        for p in [
            Platform::Kimi,
            Platform::Qwen,
            Platform::DouBao,
            Platform::YuanBao,
        ] {
            assert!(p.url().starts_with("https://"));
        }
    }
    #[test]
    fn assistant_state_only_original_layout_width_and_platforms_are_accepted() {
        let mut c = Config::default();
        for n in [1, 2, 4] {
            c.page_count = n;
            c.validate().unwrap();
        }
        for n in [0, 3, 5] {
            c.page_count = n;
            assert!(c.validate().is_err());
        }
        c.page_count = 1;
        c.width_ratio = 0.7;
        assert!(c.validate().is_err());
        let mut v = serde_json::to_value(Config::default()).unwrap();
        v["platforms"][0] = json!("https://other.test");
        assert!(serde_json::from_value::<Config>(v).is_err());
    }
    #[test]
    fn assistant_state_background_requires_valid_shortcut_but_off_allows_empty() {
        let mut c = Config::default();
        c.shortcut.clear();
        c.validate().unwrap();
        c.background_enabled = true;
        assert!(c.validate().is_err());
        c.shortcut = "Super+Alt+C".into();
        c.validate().unwrap();
        c.shortcut = "not a hotkey".into();
        assert!(c.validate().is_err());
    }
    #[test]
    fn assistant_state_proxy_accepts_original_four_protocols_and_ignores_no_security_checks() {
        for protocol in [
            Protocol::Http,
            Protocol::Https,
            Protocol::Socks4,
            Protocol::Socks5,
        ] {
            let p = Proxy::Manual {
                protocol,
                host: "127.0.0.1".into(),
                port: 7890,
            };
            let args = p.browser_arguments().unwrap().unwrap();
            assert!(args.contains("--proxy-server="));
            assert!(!args.contains("ignore-certificate"));
            assert!(!args.contains("disable-web-security"));
        }
        assert!(Proxy::System.browser_arguments().unwrap().is_none());
        assert!(Proxy::Manual {
            protocol: Protocol::Socks5,
            host: "[::1]".into(),
            port: 1080
        }
        .browser_arguments()
        .is_ok());
    }
    #[test]
    fn assistant_state_proxy_rejects_credentials_flag_injection_and_invalid_ports() {
        for host in [
            "",
            "user:pass@host",
            "host --remote-debugging-port=9",
            "host;other",
            "host/extra",
            "host?x=y",
            "host#frag",
            "host\n",
            "\"host\"",
        ] {
            assert!(
                Proxy::Manual {
                    protocol: Protocol::Http,
                    host: host.into(),
                    port: 7890
                }
                .browser_arguments()
                .is_err(),
                "{host}"
            );
        }
        assert!(Proxy::Manual {
            protocol: Protocol::Http,
            host: "localhost".into(),
            port: 0
        }
        .browser_arguments()
        .is_err());
    }
    #[test]
    fn assistant_state_profile_is_stable_per_instance_and_never_uses_page_or_proxy_text() {
        let a = uid();
        let b = uid();
        let root = std::env::temp_dir().join("assistant-profile-path-only");
        let path = profile_directory(&root, &a).unwrap();
        assert!(path.starts_with(&root));
        assert_eq!(path, profile_directory(&root, &a).unwrap());
        assert_ne!(path, profile_directory(&root, &b).unwrap());
        assert!(profile_directory(&root, "../other").is_err());
        assert!(!path.exists());
    }
    #[test]
    fn assistant_state_new_store_has_defaults_and_no_write_on_read() {
        let s = store();
        let a = uid();
        let v = status(&s, &a).unwrap();
        assert_eq!(v["revision"], 0);
        assert_eq!(v["config"]["backgroundEnabled"], false);
        assert!(s.preference(&key(&a).unwrap()).is_none());
    }
    #[test]
    fn assistant_state_uuid_retry_replays_but_conflicting_payload_and_stale_save_are_rejected() {
        let s = store();
        let a = uid();
        let rid = uid();
        let mut c = Config::default();
        c.page_count = 4;
        let args = request(&rid, 0, &c);
        let v = mutate(&s, &a, args.clone()).unwrap();
        assert_eq!(v["revision"], 1);
        assert_eq!(mutate(&s, &a, args).unwrap()["replayed"], true);
        c.page_count = 2;
        assert!(mutate(&s, &a, request(&rid, 0, &c))
            .unwrap_err()
            .contains("REQUEST_CONFLICT"));
        assert!(mutate(&s, &a, request(&uid(), 0, &c))
            .unwrap_err()
            .contains("REVISION_CONFLICT"));
        assert_eq!(status(&s, &a).unwrap()["config"]["pageCount"], 4);
    }
    #[test]
    fn assistant_state_roundtrip_and_deletion_keep_other_instance_and_profile_files_untouched() {
        let s = store();
        let a = uid();
        let b = uid();
        let mut c = Config::default();
        c.position = Position::Left;
        c.background_enabled = true;
        mutate(&s, &a, request(&uid(), 0, &c)).unwrap();
        mutate(&s, &b, request(&uid(), 0, &Config::default())).unwrap();
        let path = s.root.clone();
        drop(s);
        let s = Store::open(path).unwrap();
        assert_eq!(status(&s, &a).unwrap()["config"]["position"], "left");
        remove_data(&s.db.lock().unwrap(), &a).unwrap();
        assert_eq!(status(&s, &a).unwrap()["revision"], 0);
        assert_eq!(status(&s, &b).unwrap()["revision"], 1);
    }
    #[test]
    fn assistant_state_bad_storage_cannot_be_overwritten_and_delete_does_not_decode_it() {
        for raw in ["broken", "{\"version\":2}"] {
            let s = store();
            let a = uid();
            s.save_preference(&key(&a).unwrap(), raw).unwrap();
            assert!(status(&s, &a).is_err());
            assert!(mutate(&s, &a, request(&uid(), 0, &Config::default())).is_err());
            assert_eq!(s.preference(&key(&a).unwrap()).as_deref(), Some(raw));
            remove_data(&s.db.lock().unwrap(), &a).unwrap();
        }
    }
    #[test]
    fn assistant_state_unknown_fields_url_key_and_bad_request_ids_do_not_write() {
        let s = store();
        let a = uid();
        for field in ["apiKey", "url", "cookie", "script"] {
            let mut v = request(&uid(), 0, &Config::default());
            v["config"][field] = json!("not-accepted");
            assert!(mutate(&s, &a, v).is_err());
        }
        let mut v = request("not-uuid", 0, &Config::default());
        assert!(mutate(&s, &a, v.clone()).is_err());
        v = request(&uid(), 0, &Config::default());
        v["instanceId"] = json!(uid());
        assert!(mutate(&s, &a, v).is_err());
        assert!(s.preference(&key(&a).unwrap()).is_none());
    }
    #[test]
    fn assistant_state_failed_sql_transaction_preserves_document_and_retry_uses_same_uuid() {
        let s = store();
        let a = uid();
        let rid = uid();
        let args = request(&rid, 0, &Config::default());
        s.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_assistant BEFORE INSERT ON app_settings WHEN new.key LIKE 'assistant.config/%' BEGIN SELECT RAISE(FAIL,'refuse'); END;").unwrap();
        assert!(mutate(&s, &a, args.clone()).is_err());
        assert_eq!(status(&s, &a).unwrap()["revision"], 0);
        s.db.lock()
            .unwrap()
            .execute_batch("DROP TRIGGER refuse_assistant;")
            .unwrap();
        assert_eq!(mutate(&s, &a, args).unwrap()["revision"], 1);
    }
    #[test]
    fn assistant_state_corrupt_zero_revision_and_hidden_proxy_credentials_are_rejected() {
        let s = store();
        let a = uid();
        let mut value = request(&uid(), 0, &Config::default());
        value["config"]["proxy"] = json!({"mode":"system","apiKey":"must-not-be-accepted"});
        assert!(mutate(&s, &a, value).is_err());
        let mut raw = serde_json::to_value(Document::default()).unwrap();
        raw["config"]["backgroundEnabled"] = json!(true);
        s.save_preference(&key(&a).unwrap(), &raw.to_string())
            .unwrap();
        assert!(status(&s, &a).is_err());
    }
    #[test]
    fn assistant_state_receipt_rotation_cannot_allow_old_configuration_to_replace_new() {
        let s = store();
        let a = uid();
        let old = request(&uid(), 0, &Config::default());
        mutate(&s, &a, old.clone()).unwrap();
        for revision in 1..66 {
            let mut c = Config::default();
            c.tip_progress = (revision % 4) as u8;
            mutate(&s, &a, request(&uid(), revision, &c)).unwrap();
        }
        assert!(mutate(&s, &a, old)
            .unwrap_err()
            .contains("REVISION_CONFLICT"));
        assert_eq!(status(&s, &a).unwrap()["revision"], 66);
    }
}
