//! AI生成工具的实例凭据与版本；明文不会进入普通组件配置。
use crate::{
    ai_protocol,
    store::{err, AppResult, Store},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Profile {
    pub base_url: String,
    pub model: String,
    pub key: String,
    pub enabled: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: String,
    fingerprint: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    revision: u64,
    profile: Option<Profile>,
    receipts: Vec<Receipt>,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            profile: None,
            receipts: Vec::new(),
        }
    }
}
fn storage_key(instance: &str) -> String {
    format!("ai.credential/{instance}")
}

#[cfg(windows)]
pub(crate) fn local_dpapi(bytes: &[u8], entropy: &[u8], decrypt: bool) -> AppResult<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    if bytes.len() > 128 * 1024 {
        return Err("CREDENTIAL_ERROR: 凭据超过容量，未覆盖".into());
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let additional = CRYPT_INTEGER_BLOB {
        cbData: entropy.len() as u32,
        pbData: entropy.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        if decrypt {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &additional,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptProtectData(
                &input,
                std::ptr::null(),
                &additional,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err("CREDENTIAL_ERROR: 凭据保护失败，可能不属于当前Windows用户或实例".into());
    }
    let value = if output.cbData == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() }
    };
    unsafe {
        LocalFree(output.pbData as _);
    }
    if value.len() > 128 * 1024 {
        return Err("CREDENTIAL_ERROR: 凭据超过容量，未覆盖".into());
    }
    Ok(value)
}
#[cfg(not(windows))]
pub(crate) fn local_dpapi(_bytes: &[u8], _entropy: &[u8], _decrypt: bool) -> AppResult<Vec<u8>> {
    Err("UNSUPPORTED: 凭据当前仅支持Windows保护".into())
}

fn crypt(bytes: &[u8], instance: &str, decrypt: bool) -> AppResult<Vec<u8>> {
    local_dpapi(
        bytes,
        format!("SoulDesk.ai.v1/{instance}").as_bytes(),
        decrypt,
    )
}
fn validate_profile(value: &Profile) -> AppResult<()> {
    ai_protocol::endpoint(&value.base_url)?;
    if value.model.trim().is_empty()
        || value.model.len() > 200
        || value.model.chars().any(char::is_control)
    {
        return Err("INVALID_ARGUMENT: 请填写自己的模型名称".into());
    }
    if value.key.len() > 4096
        || !value
            .key
            .bytes()
            .all(|byte| byte.is_ascii() && !byte.is_ascii_control())
        || value.key.contains(' ')
    {
        return Err("INVALID_ARGUMENT: API Key不能包含空白/控制字符，最长4096字节".into());
    }
    Ok(())
}
fn load(db: &Connection, instance: &str) -> AppResult<Document> {
    let raw: Option<String> = db
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [storage_key(instance)],
            |row| row.get(0),
        )
        .optional()
        .map_err(err)?;
    let Some(raw) = raw else {
        return Ok(Document::default());
    };
    if raw.len() > 192 * 1024 {
        return Err("CREDENTIAL_ERROR: AI密文超过容量，未覆盖".into());
    }
    let encrypted = STANDARD
        .decode(raw)
        .map_err(|_| "CREDENTIAL_ERROR: AI密文格式无效")?;
    let document: Document = serde_json::from_slice(&crypt(&encrypted, instance, true)?)
        .map_err(|_| "CREDENTIAL_ERROR: AI凭据文档无效")?;
    if document.version != 1
        || document.revision > 9_007_199_254_740_991
        || document.receipts.len() > 64
    {
        return Err("CREDENTIAL_ERROR: AI凭据版本/回执无效，未覆盖".into());
    }
    if document
        .profile
        .as_ref()
        .is_some_and(|profile| validate_profile(profile).is_err())
    {
        return Err("CREDENTIAL_ERROR: 已保存AI配置无效，未覆盖".into());
    }
    Ok(document)
}
fn snapshot(document: &Document) -> Value {
    let profile = document.profile.as_ref();
    json!({"revision":document.revision,"configured":profile.is_some(),"enabled":profile.is_some_and(|profile|profile.enabled),"apiBaseUrl":profile.map(|profile|profile.base_url.as_str()).unwrap_or(""),"model":profile.map(|profile|profile.model.as_str()).unwrap_or(""),"hasKey":profile.is_some_and(|profile|!profile.key.is_empty())})
}
pub(crate) fn status(store: &Store, instance: &str) -> AppResult<Value> {
    Ok(snapshot(&load(&*store.db.lock().map_err(err)?, instance)?))
}
pub(crate) fn profile(store: &Store, instance: &str) -> AppResult<(u64, Option<Profile>)> {
    let document = load(&*store.db.lock().map_err(err)?, instance)?;
    Ok((document.revision, document.profile))
}
fn mutate(
    store: &Store,
    instance: &str,
    id: String,
    expected: u64,
    fingerprint: String,
    apply: impl FnOnce(&mut Document) -> AppResult<()>,
) -> AppResult<Value> {
    if uuid::Uuid::parse_str(instance).is_err() || uuid::Uuid::parse_str(&id).is_err() {
        return Err("INVALID_ARGUMENT: AI实例与请求标识须为UUID".into());
    }
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    let mut document = load(&tx, instance)?;
    if let Some(receipt) = document.receipts.iter().find(|receipt| receipt.id == id) {
        if receipt.fingerprint != fingerprint {
            return Err("REQUEST_CONFLICT: 同一AI请求标识不能用于不同配置".into());
        }
        return Ok(snapshot(&document));
    }
    if document.revision != expected {
        return Err("REVISION_CONFLICT: AI配置已更新，请重新读取".into());
    }
    apply(&mut document)?;
    document.revision = document
        .revision
        .checked_add(1)
        .filter(|revision| *revision <= 9_007_199_254_740_991)
        .ok_or("LIMIT: AI配置版本达到上限")?;
    document.receipts.push(Receipt { id, fingerprint });
    if document.receipts.len() > 64 {
        document.receipts.remove(0);
    }
    let plain = serde_json::to_vec(&document).map_err(|_| "CREDENTIAL_ERROR: AI配置序列化失败")?;
    if plain.len() > 64 * 1024 {
        return Err("LIMIT: AI凭据文档超过容量".into());
    }
    let encrypted = STANDARD.encode(crypt(&plain, instance, false)?);
    tx.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![storage_key(instance),encrypted]).map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(snapshot(&document))
}
pub(crate) fn configure(store: &Store, instance: &str, args: Value) -> AppResult<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Configure {
        request_id: String,
        expected_revision: u64,
        api_base_url: String,
        api_key: Option<String>,
        model: String,
        enabled: bool,
    }
    let fingerprint = format!("{:x}", Sha256::digest(format!("configure:{args}")));
    let change: Configure =
        serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENT: AI配置参数无效")?;
    mutate(
        store,
        instance,
        change.request_id,
        change.expected_revision,
        fingerprint,
        |document| {
            let endpoint = ai_protocol::endpoint(&change.api_base_url)?;
            let base_url = endpoint
                .as_str()
                .strip_suffix("/chat/completions")
                .ok_or("INVALID_ARGUMENT: AI接口地址无效")?
                .to_owned();
            let key=change.api_key.or_else(||document.profile.as_ref().filter(|old|old.base_url==base_url).map(|old|old.key.clone())).ok_or("INVALID_ARGUMENT: 首次或更换服务需提供自己的API Key；无鉴权服务须显式填空字符串")?;
            let profile = Profile {
                base_url,
                model: change.model.trim().into(),
                key,
                enabled: change.enabled,
            };
            validate_profile(&profile)?;
            document.profile = Some(profile);
            Ok(())
        },
    )
}
pub(crate) fn clear(store: &Store, instance: &str, args: Value) -> AppResult<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Clear {
        request_id: String,
        expected_revision: u64,
    }
    let fingerprint = format!("{:x}", Sha256::digest(format!("clear:{args}")));
    let change: Clear =
        serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENT: 清除AI配置参数无效")?;
    // 保留版本/回执的空文档，避免迟到的旧保存重新启用已清除的密钥。
    mutate(
        store,
        instance,
        change.request_id,
        change.expected_revision,
        fingerprint,
        |document| {
            document.profile = None;
            Ok(())
        },
    )
}
pub(crate) fn remove_data(db: &Connection, instance: &str) -> AppResult<()> {
    db.execute(
        "DELETE FROM app_settings WHERE key=?1",
        [storage_key(instance)],
    )
    .map_err(err)?;
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use serde_json::json;
    fn store() -> Store {
        Store::open(
            std::env::temp_dir().join(format!("souldesk-ai-credentials-{}", uuid::Uuid::new_v4())),
        )
        .unwrap()
    }
    fn config(id: &str, revision: u64, url: &str, key: Option<&str>) -> Value {
        let mut value = json!({"requestId":id,"expectedRevision":revision,"apiBaseUrl":url,"model":"fixture-model","enabled":false});
        if let Some(key) = key {
            value["apiKey"] = json!(key);
        }
        value
    }
    #[test]
    fn ai_credentials_default_disabled_and_dpapi_never_returns_or_stores_plain_key() {
        let s = store();
        let id = uuid::Uuid::new_v4().to_string();
        assert_eq!(status(&s, &id).unwrap()["enabled"], false);
        let result = configure(
            &s,
            &id,
            config(
                &uuid::Uuid::new_v4().to_string(),
                0,
                "https://api.example.com/v1",
                Some("fixture-secret-not-real"),
            ),
        )
        .unwrap();
        assert_eq!(result["configured"], true);
        assert_eq!(result["revision"], 1);
        assert!(!result.to_string().contains("fixture-secret"));
        assert_eq!(result["hasKey"], true);
        let raw: String =
            s.db.lock()
                .unwrap()
                .query_row(
                    "SELECT value FROM app_settings WHERE key=?1",
                    [format!("ai.credential/{id}")],
                    |row| row.get(0),
                )
                .unwrap();
        assert!(!raw.contains("fixture-secret"));
        let other = uuid::Uuid::new_v4().to_string();
        s.save_preference(&format!("ai.credential/{other}"), &raw)
            .unwrap();
        assert!(status(&s, &other)
            .unwrap_err()
            .starts_with("CREDENTIAL_ERROR:"));
    }
    #[test]
    fn ai_credentials_retries_are_idempotent_conflicts_preserve_data_and_endpoint_changes_cannot_reuse_key(
    ) {
        let s = store();
        let id = uuid::Uuid::new_v4().to_string();
        let request = uuid::Uuid::new_v4().to_string();
        let first = config(
            &request,
            0,
            "https://api.example.com/v1",
            Some("fixture-secret"),
        );
        let one = configure(&s, &id, first.clone()).unwrap();
        assert_eq!(configure(&s, &id, first.clone()).unwrap(), one);
        let mut changed = first;
        changed["model"] = json!("other");
        assert!(configure(&s, &id, changed)
            .unwrap_err()
            .contains("REQUEST_CONFLICT"));
        assert!(configure(
            &s,
            &id,
            config(
                &uuid::Uuid::new_v4().to_string(),
                0,
                "https://api.example.com/v1",
                None
            )
        )
        .unwrap_err()
        .contains("REVISION_CONFLICT"));
        assert!(configure(
            &s,
            &id,
            config(
                &uuid::Uuid::new_v4().to_string(),
                1,
                "https://other.example.com/v1",
                None
            )
        )
        .is_err());
        assert_eq!(status(&s, &id).unwrap(), one);
        let same = configure(
            &s,
            &id,
            config(
                &uuid::Uuid::new_v4().to_string(),
                1,
                "https://api.example.com/v1/",
                None,
            ),
        )
        .unwrap();
        assert_eq!(same["revision"], 2);
        assert_eq!(same["hasKey"], true);
    }
    #[test]
    fn ai_credentials_failed_write_is_atomic_and_clear_tombstone_blocks_old_reactivation() {
        let s = store();
        let id = uuid::Uuid::new_v4().to_string();
        let first = config(
            &uuid::Uuid::new_v4().to_string(),
            0,
            "https://api.example.com/v1",
            Some("fixture-secret"),
        );
        let one = configure(&s, &id, first.clone()).unwrap();
        s.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_ai BEFORE UPDATE ON app_settings WHEN new.key LIKE 'ai.credential/%' BEGIN SELECT RAISE(FAIL,'save denied'); END;").unwrap();
        let mut next = config(
            &uuid::Uuid::new_v4().to_string(),
            1,
            "https://api.example.com/v1",
            None,
        );
        next["enabled"] = json!(true);
        assert!(configure(&s, &id, next).is_err());
        assert_eq!(status(&s, &id).unwrap(), one);
        s.db.lock()
            .unwrap()
            .execute_batch("DROP TRIGGER refuse_ai")
            .unwrap();
        let cleared = clear(
            &s,
            &id,
            json!({"requestId":uuid::Uuid::new_v4().to_string(),"expectedRevision":1}),
        )
        .unwrap();
        assert_eq!(cleared["configured"], false);
        assert_eq!(cleared["revision"], 2);
        assert_eq!(configure(&s, &id, first).unwrap(), cleared);
        assert_eq!(status(&s, &id).unwrap()["enabled"], false);
    }
    #[test]
    fn ai_credentials_rejects_extra_fields_bad_key_headers_and_malformed_protected_data() {
        let s = store();
        let id = uuid::Uuid::new_v4().to_string();
        let mut args = config(
            &uuid::Uuid::new_v4().to_string(),
            0,
            "https://api.example.com/v1",
            Some("bad\r\nheader"),
        );
        assert!(configure(&s, &id, args.clone()).is_err());
        args["apiKey"] = json!("fixture");
        args["headers"] = json!({"arbitrary":"value"});
        assert!(configure(&s, &id, args).is_err());
        assert_eq!(status(&s, &id).unwrap()["revision"], 0);
        s.save_preference(&format!("ai.credential/{id}"), "invalid-cipher")
            .unwrap();
        assert!(status(&s, &id)
            .unwrap_err()
            .starts_with("CREDENTIAL_ERROR:"));
    }
}
