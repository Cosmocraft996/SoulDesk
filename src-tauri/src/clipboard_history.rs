//! 剪切板历史的实例加密存储；不读取系统剪贴板、不注册快捷键或发送输入。
use crate::store::{now_ms, AppResult, Store};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const MAX_ITEMS: usize = 1000;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_SAFE: u64 = 9_007_199_254_740_991;
const DATA_ERROR: &str =
    "DATA_UNAVAILABLE: 剪切板历史无效或不属于当前Windows用户/实例，原数据未覆盖";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Settings {
    capture_enabled: bool,
    shortcut_enabled: bool,
    paste_enabled: bool,
    shortcut: String,
    history_expired_hours: u16,
    #[serde(
        default = "default_search_platform",
        skip_serializing_if = "search_platform_is_default"
    )]
    search_platform: String,
}
fn default_search_platform() -> String {
    "google".into()
}
fn search_platform_is_default(value: &str) -> bool {
    value == "google"
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            capture_enabled: false,
            shortcut_enabled: false,
            paste_enabled: false,
            shortcut: "Super+Alt+V".into(),
            history_expired_hours: 12,
            search_platform: default_search_platform(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    id: String,
    revision: u64,
    content: String,
    #[serde(rename = "type")]
    kind: String,
    labels: Vec<String>,
    favorite: bool,
    use_count: u64,
    create_time: u64,
    update_time: u64,
    latest_use_time: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: String,
    fingerprint: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    version: u32,
    revision: u64,
    config_revision: u64,
    settings: Settings,
    items: Vec<Record>,
    receipts: Vec<Receipt>,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            config_revision: 0,
            settings: Settings::default(),
            items: Vec::new(),
            receipts: Vec::new(),
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    request_id: String,
    command: Operation,
}
#[derive(Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Operation {
    Configure {
        expected_revision: u64,
        capture_enabled: bool,
        shortcut_enabled: bool,
        paste_enabled: bool,
        shortcut: String,
        history_expired_hours: u16,
        #[serde(
            default = "default_search_platform",
            skip_serializing_if = "search_platform_is_default"
        )]
        search_platform: String,
    },
    Edit {
        id: String,
        expected_item_revision: u64,
        content: String,
        labels: Vec<String>,
        favorite: bool,
    },
    Used {
        id: String,
        expected_item_revision: u64,
    },
    Delete {
        id: String,
        expected_item_revision: u64,
    },
    Clear {
        expected_revision: u64,
    },
}
fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id)
}
fn key(instance: &str) -> AppResult<String> {
    if !valid_id(instance) {
        return Err("INVALID_ARGUMENT: 剪切板实例标识无效".into());
    }
    Ok(format!("clipboard.history/{instance}"))
}
fn next(value: u64) -> AppResult<u64> {
    value
        .checked_add(1)
        .filter(|v| *v <= MAX_SAFE)
        .ok_or_else(|| "CAPACITY_EXCEEDED: 剪切板版本或使用计数已达上限".into())
}
fn text_valid(text: &str) -> bool {
    !text.is_empty() && !text.contains('\0') && text.chars().take(50001).count() <= 50000
}
fn labels_valid(labels: &[String]) -> bool {
    labels.len() <= 3
        && labels.iter().enumerate().all(|(i, label)| {
            let count = label.chars().count();
            (1..=10).contains(&count)
                && label.chars().all(|ch| {
                    ch.is_ascii_alphanumeric()
                        || ch == '_'
                        || ('\u{4e00}'..='\u{9fa5}').contains(&ch)
                })
                && !labels[..i].contains(label)
        })
}
fn settings_valid(value: &Settings) -> bool {
    matches!(value.history_expired_hours, 12 | 24 | 48)
        && matches!(value.search_platform.as_str(), "google" | "bing" | "baidu")
        && value.shortcut.len() <= 80
        && value
            .shortcut
            .parse::<tauri_plugin_global_shortcut::Shortcut>()
            .is_ok()
}
fn validate(doc: &Document) -> AppResult<()> {
    if doc.version != 1
        || doc.revision > MAX_SAFE
        || doc.config_revision > doc.revision
        || !settings_valid(&doc.settings)
        || doc.items.len() > MAX_ITEMS
        || doc.receipts.len() > 64
    {
        return Err(DATA_ERROR.into());
    }
    let mut ids = HashSet::new();
    for item in &doc.items {
        if !valid_id(&item.id)
            || !ids.insert(&item.id)
            || item.kind != "text"
            || item.revision == 0
            || item.revision > doc.revision
            || item.use_count > MAX_SAFE
            || !text_valid(&item.content)
            || !labels_valid(&item.labels)
            || item.create_time > item.update_time
            || item.create_time > item.latest_use_time
            || item.update_time > MAX_SAFE
            || item.latest_use_time > MAX_SAFE
        {
            return Err(DATA_ERROR.into());
        }
    }
    ids.clear();
    for receipt in &doc.receipts {
        if !valid_id(&receipt.id)
            || !ids.insert(&receipt.id)
            || receipt.fingerprint.len() != 64
            || !receipt.fingerprint.bytes().all(|ch| ch.is_ascii_hexdigit())
        {
            return Err(DATA_ERROR.into());
        }
    }
    Ok(())
}
#[cfg(windows)]
pub(crate) fn crypt(bytes: &[u8], instance: &str, decrypt: bool) -> AppResult<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    if bytes.len() > MAX_BYTES + 64 * 1024 {
        return Err(DATA_ERROR.into());
    }
    let entropy = format!("SoulDesk.clipboard-history.v1/{instance}").into_bytes();
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
        return Err(DATA_ERROR.into());
    }
    let value = if output.cbData == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() }
    };
    unsafe {
        LocalFree(output.pbData as _);
    }
    if value.len() > MAX_BYTES + 64 * 1024 {
        return Err(DATA_ERROR.into());
    }
    Ok(value)
}
#[cfg(not(windows))]
pub(crate) fn crypt(_bytes: &[u8], _instance: &str, _decrypt: bool) -> AppResult<Vec<u8>> {
    Err("UNSUPPORTED: 剪切板历史当前仅支持Windows保护".into())
}
fn load(db: &Connection, instance: &str) -> AppResult<Document> {
    let encoded: Option<String> = db
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [key(instance)?],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "STORAGE_ERROR: 剪切板历史读取失败")?;
    let Some(encoded) = encoded else {
        return Ok(Document::default());
    };
    if encoded.len() > MAX_BYTES * 2 {
        return Err(DATA_ERROR.into());
    }
    let cipher = STANDARD.decode(encoded).map_err(|_| DATA_ERROR)?;
    let bytes = crypt(&cipher, instance, true)?;
    if bytes.len() > MAX_BYTES {
        return Err(DATA_ERROR.into());
    }
    let doc: Document = serde_json::from_slice(&bytes).map_err(|_| DATA_ERROR)?;
    validate(&doc)?;
    Ok(doc)
}
fn save(db: &Connection, instance: &str, doc: &Document) -> AppResult<()> {
    validate(doc)?;
    let bytes = serde_json::to_vec(doc).map_err(|_| "STORAGE_ERROR: 剪切板历史编码失败")?;
    if bytes.len() > MAX_BYTES {
        return Err("CAPACITY_EXCEEDED: 剪切板历史已达4MiB，原数据未覆盖".into());
    }
    let cipher = STANDARD.encode(crypt(&bytes, instance, false)?);
    db.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key(instance)?, cipher])
        .map_err(|_| "STORAGE_ERROR: 剪切板历史保存失败，原数据未覆盖")?;
    Ok(())
}
fn snapshot(doc: &Document) -> Value {
    json!({"revision":doc.revision,"configRevision":doc.config_revision,
        "captureEnabled":doc.settings.capture_enabled,"shortcutEnabled":doc.settings.shortcut_enabled,
        "pasteEnabled":doc.settings.paste_enabled,"shortcut":doc.settings.shortcut,
        "historyExpiredHours":doc.settings.history_expired_hours,"searchPlatform":doc.settings.search_platform,"count":doc.items.len()})
}
pub(crate) fn status(store: &Store, instance: &str) -> AppResult<Value> {
    let db = store
        .db
        .lock()
        .map_err(|_| "STORAGE_ERROR: 剪切板历史锁不可用")?;
    Ok(snapshot(&load(&db, instance)?))
}
pub(crate) fn query(store: &Store, instance: &str, args: Value) -> AppResult<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Search {
        #[serde(default)]
        keyword: String,
        #[serde(default = "one")]
        page: usize,
        #[serde(default = "ten")]
        size: usize,
        #[serde(default)]
        favorites_only: bool,
    }
    fn one() -> usize {
        1
    }
    fn ten() -> usize {
        10
    }
    let search: Search =
        serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENT: 剪切板搜索参数无效")?;
    if !(1..=1000).contains(&search.page)
        || !(1..=100).contains(&search.size)
        || search.keyword.chars().count() > 200
        || search.keyword.contains('\0')
    {
        return Err("INVALID_ARGUMENT: 剪切板搜索超出范围".into());
    }
    let db = store
        .db
        .lock()
        .map_err(|_| "STORAGE_ERROR: 剪切板历史锁不可用")?;
    let doc = load(&db, instance)?;
    let keyword = search.keyword.to_lowercase();
    let mut records: Vec<_> = doc
        .items
        .iter()
        .filter(|item| {
            (!search.favorites_only || item.favorite)
                && (keyword.is_empty()
                    || item.content.to_lowercase().contains(&keyword)
                    || item
                        .labels
                        .iter()
                        .any(|label| label.to_lowercase().contains(&keyword)))
        })
        .collect();
    records.sort_by(|a, b| {
        b.latest_use_time
            .cmp(&a.latest_use_time)
            .then_with(|| b.create_time.cmp(&a.create_time))
            .then_with(|| a.id.cmp(&b.id))
    });
    let total = records.len();
    let start = (search.page - 1) * search.size;
    Ok(
        json!({"revision":doc.revision,"total":total,"hasMore":start+search.size<total,"items":records.into_iter().skip(start).take(search.size).collect::<Vec<_>>()}),
    )
}
pub(crate) fn get(store: &Store, instance: &str, id: &str) -> AppResult<Value> {
    let db = store
        .db
        .lock()
        .map_err(|_| "STORAGE_ERROR: 剪切板历史锁不可用")?;
    let doc = load(&db, instance)?;
    let item = doc
        .items
        .iter()
        .find(|item| item.id == id)
        .ok_or("NOT_FOUND: 剪切板记录不存在")?;
    serde_json::to_value(item).map_err(|_| "STORAGE_ERROR: 剪切板记录编码失败".into())
}
pub(crate) fn mutate(store: &Store, instance: &str, args: Value) -> AppResult<Value> {
    mutate_at(store, instance, args, now_ms())
}
fn item_index(doc: &Document, id: &str, expected: u64) -> AppResult<usize> {
    let index = doc
        .items
        .iter()
        .position(|item| item.id == id)
        .ok_or("NOT_FOUND: 剪切板记录不存在")?;
    if doc.items[index].revision != expected {
        return Err("REVISION_CONFLICT: 剪切板记录已变化，请重新读取".into());
    }
    Ok(index)
}
fn mutate_at(store: &Store, instance: &str, args: Value, now: u64) -> AppResult<Value> {
    let request: Request =
        serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENT: 剪切板操作参数无效")?;
    if !valid_id(&request.request_id) || now > MAX_SAFE {
        return Err("INVALID_ARGUMENT: 剪切板请求标识或时间无效".into());
    }
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&request.command)
                .map_err(|_| "INVALID_ARGUMENT: 剪切板操作编码失败")?
        )
    );
    let mut db = store
        .db
        .lock()
        .map_err(|_| "STORAGE_ERROR: 剪切板历史锁不可用")?;
    let tx = db
        .transaction()
        .map_err(|_| "STORAGE_ERROR: 剪切板事务不可用")?;
    let mut doc = load(&tx, instance)?;
    if let Some(receipt) = doc.receipts.iter().find(|r| r.id == request.request_id) {
        if receipt.fingerprint != fingerprint {
            return Err("REQUEST_CONFLICT: 同一剪切板请求标识不能更换内容".into());
        }
        let mut result = snapshot(&doc);
        result["replayed"] = json!(true);
        return Ok(result);
    }
    match request.command {
        Operation::Configure {
            expected_revision,
            capture_enabled,
            shortcut_enabled,
            paste_enabled,
            shortcut,
            history_expired_hours,
            search_platform,
        } => {
            if doc.config_revision != expected_revision {
                return Err("REVISION_CONFLICT: 剪切板配置已变化，请重新读取".into());
            }
            let settings = Settings {
                capture_enabled,
                shortcut_enabled,
                paste_enabled,
                shortcut,
                history_expired_hours,
                search_platform,
            };
            if !settings_valid(&settings) {
                return Err("INVALID_ARGUMENT: 剪切板快捷键或保留时长无效".into());
            }
            doc.settings = settings;
            doc.config_revision = next(doc.config_revision)?;
        }
        Operation::Edit {
            id,
            expected_item_revision,
            content,
            labels,
            favorite,
        } => {
            if !text_valid(&content) || !labels_valid(&labels) {
                return Err("INVALID_ARGUMENT: 内容须为1～50000字符且不含零字符，标签最多3个且各1～10个中英文数字或下划线".into());
            }
            let index = item_index(&doc, &id, expected_item_revision)?;
            let item = &mut doc.items[index];
            item.content = content;
            item.labels = labels;
            item.favorite = favorite;
            item.revision = next(item.revision)?;
            item.update_time = now.max(item.update_time);
        }
        Operation::Used {
            id,
            expected_item_revision,
        } => {
            let index = item_index(&doc, &id, expected_item_revision)?;
            let item = &mut doc.items[index];
            item.use_count = next(item.use_count)?;
            item.revision = next(item.revision)?;
            item.latest_use_time = now.max(item.latest_use_time);
            item.update_time = now.max(item.update_time);
        }
        Operation::Delete {
            id,
            expected_item_revision,
        } => {
            let index = item_index(&doc, &id, expected_item_revision)?;
            doc.items.remove(index);
        }
        Operation::Clear { expected_revision } => {
            if doc.revision != expected_revision {
                return Err("REVISION_CONFLICT: 清理前历史已变化，请重新读取".into());
            }
            doc.items.retain(|item| item.favorite);
        }
    }
    doc.revision = next(doc.revision)?;
    doc.receipts.push(Receipt {
        id: request.request_id,
        fingerprint,
    });
    if doc.receipts.len() > 64 {
        doc.receipts.remove(0);
    }
    save(&tx, instance, &doc)?;
    tx.commit()
        .map_err(|_| "STORAGE_ERROR: 剪切板事务提交失败，请重读确认")?;
    let mut result = snapshot(&doc);
    result["replayed"] = json!(false);
    Ok(result)
}
pub(crate) fn capture(
    store: &Store,
    instance: &str,
    expected_config_revision: u64,
    content: &str,
    now: u64,
) -> AppResult<Value> {
    let mut db = store
        .db
        .lock()
        .map_err(|_| "STORAGE_ERROR: 剪切板历史锁不可用")?;
    let tx = db
        .transaction()
        .map_err(|_| "STORAGE_ERROR: 剪切板事务不可用")?;
    let mut doc = load(&tx, instance)?;
    if doc.config_revision != expected_config_revision {
        return Err("STALE_CAPTURE: 采样所属剪切板配置已失效".into());
    }
    if !doc.settings.capture_enabled || content.is_empty() {
        return Ok(json!({"captured":false,"revision":doc.revision}));
    }
    if !text_valid(content) || now > MAX_SAFE {
        return Err("INVALID_ARGUMENT: 剪切板文本超过50000字符或包含零字符，未保存".into());
    }
    let before = doc.items.len();
    expire(&mut doc, now);
    let duplicate = doc.items.iter().any(|item| item.content == content);
    if duplicate && doc.items.len() == before {
        return Ok(json!({"captured":false,"revision":doc.revision}));
    }
    if !duplicate {
        if doc.items.len() >= MAX_ITEMS {
            return Err("CAPACITY_EXCEEDED: 剪切板历史已达1000条，未删除已有记录".into());
        }
        doc.items.push(Record {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            content: content.into(),
            kind: "text".into(),
            labels: Vec::new(),
            favorite: false,
            use_count: 0,
            create_time: now,
            update_time: now,
            latest_use_time: now,
        });
    }
    doc.revision = next(doc.revision)?;
    save(&tx, instance, &doc)?;
    tx.commit()
        .map_err(|_| "STORAGE_ERROR: 剪切板采样提交失败，请重读确认")?;
    Ok(json!({"captured":!duplicate,"revision":doc.revision}))
}
pub(crate) fn remove_data(db: &Connection, instance: &str) -> AppResult<()> {
    db.execute("DELETE FROM app_settings WHERE key=?1", [key(instance)?])
        .map_err(|_| "STORAGE_ERROR: 剪切板历史清理失败")?;
    Ok(())
}
fn expire(doc: &mut Document, now: u64) -> usize {
    let cutoff = now.saturating_sub(u64::from(doc.settings.history_expired_hours) * 60 * 60 * 1000);
    let before = doc.items.len();
    doc.items
        .retain(|item| item.favorite || item.latest_use_time >= cutoff);
    before - doc.items.len()
}
pub(crate) fn prune(store: &Store, instance: &str, now: u64) -> AppResult<Value> {
    if now > MAX_SAFE {
        return Err("INVALID_ARGUMENT: 剪切板清理时间无效".into());
    }
    let mut db = store
        .db
        .lock()
        .map_err(|_| "STORAGE_ERROR: 剪切板历史锁不可用")?;
    let tx = db
        .transaction()
        .map_err(|_| "STORAGE_ERROR: 剪切板事务不可用")?;
    let mut doc = load(&tx, instance)?;
    let removed = expire(&mut doc, now);
    if removed > 0 {
        doc.revision = next(doc.revision)?;
        save(&tx, instance, &doc)?;
        tx.commit()
            .map_err(|_| "STORAGE_ERROR: 剪切板清理提交失败，请重读确认")?;
    }
    let mut result = snapshot(&doc);
    result["removed"] = json!(removed);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn clipboard_history_legacy_config_receipt_replays_after_optional_search_platform_added() {
        let s = store();
        let owner = id();
        let request_id = id();
        let old = r#"{"type":"configure","expectedRevision":0,"captureEnabled":false,"shortcutEnabled":false,"pasteEnabled":false,"shortcut":"Super+Alt+V","historyExpiredHours":12}"#;
        let mut doc = Document::default();
        doc.revision = 1;
        doc.config_revision = 1;
        doc.receipts.push(Receipt {
            id: request_id.clone(),
            fingerprint: format!("{:x}", Sha256::digest(old.as_bytes())),
        });
        save(&s.db.lock().unwrap(), &owner, &doc).unwrap();
        let result = mutate_at(
            &s,
            &owner,
            json!({"requestId":request_id,"command":config(0,false)}),
            1000,
        )
        .unwrap();
        assert_eq!(result["replayed"], true);
        assert_eq!(result["configRevision"], 1);
        assert_eq!(result["searchPlatform"], "google");
    }

    fn store() -> Store {
        Store::open(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("artifacts")
                .join(format!(
                    "widgetjs-clipboard-history-test-{}",
                    uuid::Uuid::new_v4()
                )),
        )
        .unwrap()
    }
    fn id() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn request(command: Value) -> Value {
        json!({"requestId":id(),"command":command})
    }
    fn config(revision: u64, enabled: bool) -> Value {
        json!({"type":"configure","expectedRevision":revision,"captureEnabled":enabled,"shortcutEnabled":false,"pasteEnabled":false,"shortcut":"Super+Alt+V","historyExpiredHours":12})
    }
    fn change(s: &Store, owner: &str, command: Value, now: u64) -> Value {
        mutate_at(s, owner, request(command), now).unwrap()
    }
    fn list(s: &Store, owner: &str) -> Value {
        query(s, owner, json!({"page":1,"size":100})).unwrap()
    }
    fn add(s: &Store, owner: &str, content: &str, now: u64) -> Value {
        capture(s, owner, 1, content, now).unwrap();
        let rows = list(s, owner);
        rows["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["content"] == content)
            .unwrap()
            .clone()
    }
    fn edit(item: &Value, content: &str, favorite: bool, labels: Value) -> Value {
        json!({"type":"edit","id":item["id"],"expectedItemRevision":item["revision"],"content":content,"favorite":favorite,"labels":labels})
    }

    #[test]
    fn clipboard_history_default_is_disabled_and_reading_does_not_create_private_data() {
        let s = store();
        let owner = id();
        let state = status(&s, &owner).unwrap();
        assert_eq!(state["captureEnabled"], false);
        assert_eq!(state["shortcutEnabled"], false);
        assert_eq!(state["pasteEnabled"], false);
        assert_eq!(state["revision"], 0);
        assert_eq!(state["configRevision"], 0);
        assert_eq!(state["count"], 0);
        assert_eq!(list(&s, &owner)["items"], json!([]));
        assert_eq!(
            capture(&s, &owner, 0, "fixture-not-collected", 1000).unwrap()["captured"],
            false
        );
        let count: i64 =
            s.db.lock()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM app_settings WHERE key LIKE 'clipboard.history/%'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(count, 0);
    }
    #[test]
    fn clipboard_history_search_platform_defaults_to_google_and_only_accepts_three_original_engines(
    ) {
        let s = store();
        let owner = id();
        assert_eq!(status(&s, &owner).unwrap()["searchPlatform"], "google");
        let mut c = config(0, true);
        c["searchPlatform"] = json!("bing");
        change(&s, &owner, c, 1000);
        assert_eq!(status(&s, &owner).unwrap()["searchPlatform"], "bing");
        let mut c = config(1, true);
        c["searchPlatform"] = json!("javascript:alert(1)");
        assert!(mutate_at(&s, &owner, request(c), 1001).is_err());
        assert_eq!(status(&s, &owner).unwrap()["searchPlatform"], "bing");
    }

    #[test]
    fn clipboard_history_capture_deduplicates_exact_text_and_searches_content_and_labels_with_paging(
    ) {
        let s = store();
        let owner = id();
        change(&s, &owner, config(0, true), 1000);
        let first = add(&s, &owner, "First 中文\n原文", 1001);
        let before = status(&s, &owner).unwrap()["revision"].clone();
        assert_eq!(
            capture(&s, &owner, 1, "First 中文\n原文", 1002).unwrap()["captured"],
            false
        );
        assert_eq!(status(&s, &owner).unwrap()["revision"], before);
        assert_eq!(first["type"], "text");
        assert_eq!(first["useCount"], 0);
        change(
            &s,
            &owner,
            edit(&first, "First 中文\n原文", false, json!(["工作_A"])),
            1003,
        );
        add(&s, &owner, "second", 1004);
        add(&s, &owner, "third", 1005);
        let found = query(&s, &owner, json!({"keyword":"fIRSt","page":1,"size":1})).unwrap();
        assert_eq!(found["items"][0]["id"], first["id"]);
        assert_eq!(found["total"], 1);
        assert_eq!(
            query(&s, &owner, json!({"keyword":"工作_a"})).unwrap()["total"],
            1
        );
        let page = query(&s, &owner, json!({"page":2,"size":1})).unwrap();
        assert_eq!(page["items"][0]["content"], "second");
        assert_eq!(page["hasMore"], true);
        assert_eq!(
            query(&s, &owner, json!({"page":4,"size":1})).unwrap()["items"],
            json!([])
        );
    }

    #[test]
    fn clipboard_history_edit_and_delete_use_item_versions_without_conflicting_with_new_capture() {
        let s = store();
        let owner = id();
        change(&s, &owner, config(0, true), 1000);
        let item = add(&s, &owner, "草稿", 1001);
        add(&s, &owner, "后台新记录", 1002);
        change(
            &s,
            &owner,
            edit(&item, "手工内容", true, json!(["标签"])),
            1003,
        );
        assert_eq!(
            get(&s, &owner, item["id"].as_str().unwrap()).unwrap()["content"],
            "手工内容"
        );
        assert!(mutate_at(
            &s,
            &owner,
            request(edit(&item, "迟到草稿", false, json!([]))),
            1004
        )
        .unwrap_err()
        .contains("REVISION_CONFLICT"));
        assert!(mutate_at(
            &s,
            &owner,
            request(json!({"type":"delete","id":item["id"],"expectedItemRevision":1})),
            1005
        )
        .unwrap_err()
        .contains("REVISION_CONFLICT"));
        assert_eq!(list(&s, &owner)["total"], 2);
        let latest = get(&s, &owner, item["id"].as_str().unwrap()).unwrap();
        change(
            &s,
            &owner,
            json!({"type":"delete","id":latest["id"],"expectedItemRevision":latest["revision"]}),
            1006,
        );
        assert_eq!(list(&s, &owner)["total"], 1);
    }

    #[test]
    fn clipboard_history_usage_is_counted_once_and_changes_recent_order_only_after_confirmed_action(
    ) {
        let s = store();
        let owner = id();
        change(&s, &owner, config(0, true), 1000);
        let a = add(&s, &owner, "A", 1001);
        add(&s, &owner, "B", 1002);
        let used =
            request(json!({"type":"used","id":a["id"],"expectedItemRevision":a["revision"]}));
        mutate_at(&s, &owner, used.clone(), 1003).unwrap();
        assert_eq!(mutate_at(&s, &owner, used, 2000).unwrap()["replayed"], true);
        let rows = list(&s, &owner);
        assert_eq!(rows["items"][0]["content"], "A");
        assert_eq!(rows["items"][0]["useCount"], 1);
        assert_eq!(rows["items"][0]["latestUseTime"], 1003);
    }

    #[test]
    fn clipboard_history_expiration_is_strict_and_clear_preserves_favorites() {
        let s = store();
        let owner = id();
        change(&s, &owner, config(0, true), 1000);
        let ordinary = add(&s, &owner, "普通", 1001);
        let favorite = add(&s, &owner, "收藏", 1001);
        change(&s, &owner, edit(&favorite, "收藏", true, json!([])), 1002);
        capture(&s, &owner, 1, "边界", 1001 + 12 * 60 * 60 * 1000).unwrap();
        assert!(get(&s, &owner, ordinary["id"].as_str().unwrap()).is_ok());
        capture(&s, &owner, 1, "边界之后", 1002 + 12 * 60 * 60 * 1000).unwrap();
        assert!(get(&s, &owner, ordinary["id"].as_str().unwrap())
            .unwrap_err()
            .contains("NOT_FOUND"));
        let before = status(&s, &owner).unwrap()["revision"].clone();
        change(
            &s,
            &owner,
            json!({"type":"clear","expectedRevision":before}),
            1003 + 12 * 60 * 60 * 1000,
        );
        let remaining = list(&s, &owner);
        assert_eq!(remaining["total"], 1);
        assert_eq!(remaining["items"][0]["content"], "收藏");
        assert_eq!(
            query(&s, &owner, json!({"favoritesOnly":true})).unwrap()["total"],
            1
        );
    }

    #[test]
    fn clipboard_history_uuid_replays_cannot_clear_newer_data_or_reenable_old_configuration() {
        let s = store();
        let owner = id();
        let enable = request(config(0, true));
        mutate_at(&s, &owner, enable.clone(), 1000).unwrap();
        add(&s, &owner, "旧记录", 1001);
        let before = status(&s, &owner).unwrap()["revision"].clone();
        let clear = request(json!({"type":"clear","expectedRevision":before}));
        mutate_at(&s, &owner, clear.clone(), 1002).unwrap();
        add(&s, &owner, "新记录", 1003);
        assert_eq!(
            mutate_at(&s, &owner, clear.clone(), 1004).unwrap()["replayed"],
            true
        );
        assert_eq!(list(&s, &owner)["total"], 1);
        let mut conflict = clear;
        conflict["command"]["expectedRevision"] = json!(999);
        assert!(mutate_at(&s, &owner, conflict, 1005)
            .unwrap_err()
            .contains("REQUEST_CONFLICT"));
        change(&s, &owner, config(1, false), 1006);
        assert_eq!(
            mutate_at(&s, &owner, enable, 1007).unwrap()["captureEnabled"],
            false
        );
        assert!(mutate_at(&s, &owner, request(config(0, true)), 1008)
            .unwrap_err()
            .contains("REVISION_CONFLICT"));
    }

    #[test]
    fn clipboard_history_config_epochs_reject_late_capture_and_instances_are_independent() {
        let s = store();
        let a = id();
        let b = id();
        change(&s, &a, config(0, true), 1000);
        change(&s, &b, config(0, true), 1000);
        change(&s, &a, config(1, false), 1001);
        change(&s, &a, config(2, true), 1002);
        assert!(capture(&s, &a, 1, "旧采样", 1003)
            .unwrap_err()
            .contains("STALE_CAPTURE"));
        assert_eq!(
            capture(&s, &a, 3, "新采样", 1004).unwrap()["captured"],
            true
        );
        add(&s, &b, "另一个实例", 1005);
        assert_eq!(list(&s, &a)["total"], 1);
        assert_eq!(list(&s, &b)["total"], 1);
        let db = s.db.lock().unwrap();
        remove_data(&db, &a).unwrap();
        drop(db);
        assert_eq!(status(&s, &a).unwrap()["captureEnabled"], false);
        assert_eq!(list(&s, &b)["total"], 1);
    }

    #[test]
    fn clipboard_history_content_labels_settings_and_query_are_strictly_bounded() {
        let s = store();
        let owner = id();
        change(&s, &owner, config(0, true), 1000);
        let item = add(&s, &owner, "内容", 1001);
        assert!(capture(&s, &owner, 1, &"界".repeat(50001), 1002).is_err());
        assert!(capture(&s, &owner, 1, "含\0零", 1002).is_err());
        for labels in [
            json!(["a", "b", "c", "d"]),
            json!(["重复", "重复"]),
            json!(["非法-标签"]),
            json!(["超过十个字符的中文标签了"]),
        ] {
            assert!(mutate_at(
                &s,
                &owner,
                request(edit(&item, "内容", false, labels)),
                1003
            )
            .is_err());
        }
        for args in [
            json!({"page":0}),
            json!({"size":101}),
            json!({"url":"https://example.com"}),
        ] {
            assert!(query(&s, &owner, args).is_err());
        }
        let mut bad = config(1, true);
        bad["historyExpiredHours"] = json!(1);
        assert!(mutate_at(&s, &owner, request(bad), 1004).is_err());
        let mut bad = request(config(1, true));
        bad["requestId"] = json!("not-a-uuid");
        assert!(mutate_at(&s, &owner, bad, 1004).is_err());
        let mut bad = request(config(1, true));
        bad["extra"] = json!("ignored?");
        assert!(mutate_at(&s, &owner, bad, 1004).is_err());
        assert_eq!(
            get(&s, &owner, item["id"].as_str().unwrap()).unwrap()["content"],
            "内容"
        );
    }

    #[test]
    fn clipboard_history_total_capacity_rejects_new_text_without_evicting_favorites_or_prior_history(
    ) {
        let s = store();
        let owner = id();
        change(&s, &owner, config(0, true), 1000);
        let favorite = add(&s, &owner, "保留收藏", 1001);
        change(
            &s,
            &owner,
            edit(&favorite, "保留收藏", true, json!([])),
            1002,
        );
        let text = "界".repeat(49990);
        let mut rejected = false;
        for n in 0..40 {
            let content = format!("{n}:{text}");
            let before = status(&s, &owner).unwrap()["count"].clone();
            match capture(&s, &owner, 1, &content, 1003 + n) {
                Ok(_) => {}
                Err(e) => {
                    assert!(e.contains("CAPACITY_EXCEEDED"));
                    assert_eq!(status(&s, &owner).unwrap()["count"], before);
                    rejected = true;
                    break;
                }
            }
        }
        assert!(rejected);
        assert_eq!(
            get(&s, &owner, favorite["id"].as_str().unwrap()).unwrap()["favorite"],
            true
        );
    }

    #[test]
    fn clipboard_history_dpapi_hides_text_and_binds_ciphertext_to_instance() {
        let s = store();
        let owner = id();
        change(&s, &owner, config(0, true), 1000);
        add(&s, &owner, "fixture-剪贴板-secret", 1001);
        let raw: String =
            s.db.lock()
                .unwrap()
                .query_row(
                    "SELECT value FROM app_settings WHERE key=?1",
                    [format!("clipboard.history/{owner}")],
                    |r| r.get(0),
                )
                .unwrap();
        assert!(!raw.contains("fixture"));
        assert!(!raw.contains("剪贴板"));
        assert!(!status(&s, &owner).unwrap().to_string().contains("secret"));
        let other = id();
        s.save_preference(&format!("clipboard.history/{other}"), &raw)
            .unwrap();
        assert!(status(&s, &other)
            .unwrap_err()
            .starts_with("DATA_UNAVAILABLE:"));
        assert_eq!(
            list(&s, &owner)["items"][0]["content"],
            "fixture-剪贴板-secret"
        );
    }

    #[test]
    fn clipboard_history_failed_sqlite_write_is_atomic_and_damaged_data_is_not_overwritten() {
        let s = store();
        let owner = id();
        change(&s, &owner, config(0, true), 1000);
        let item = add(&s, &owner, "保存原值", 1001);
        let before = status(&s, &owner).unwrap();
        s.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_clipboard BEFORE UPDATE ON app_settings WHEN new.key LIKE 'clipboard.history/%' BEGIN SELECT RAISE(FAIL,'fixture save denied'); END;").unwrap();
        assert!(mutate_at(
            &s,
            &owner,
            request(edit(&item, "不能保存", false, json!([]))),
            1002
        )
        .is_err());
        assert_eq!(status(&s, &owner).unwrap(), before);
        assert_eq!(
            get(&s, &owner, item["id"].as_str().unwrap()).unwrap()["content"],
            "保存原值"
        );
        s.db.lock()
            .unwrap()
            .execute_batch("DROP TRIGGER refuse_clipboard")
            .unwrap();
        let other = id();
        s.save_preference(&format!("clipboard.history/{other}"), "broken-cipher")
            .unwrap();
        assert!(status(&s, &other).is_err());
        assert!(mutate_at(&s, &other, request(config(0, true)), 1003).is_err());
        let raw: String =
            s.db.lock()
                .unwrap()
                .query_row(
                    "SELECT value FROM app_settings WHERE key=?1",
                    [format!("clipboard.history/{other}")],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(raw, "broken-cipher");
    }

    #[test]
    fn clipboard_history_opening_without_new_copy_expires_only_unused_nonfavorites() {
        let s = store();
        let owner = id();
        change(&s, &owner, config(0, true), 1000);
        let old = add(&s, &owner, "过期记录", 1001);
        let favorite = add(&s, &owner, "过期收藏", 1001);
        change(
            &s,
            &owner,
            edit(&favorite, "过期收藏", true, json!([])),
            1002,
        );
        let recent = add(&s, &owner, "稍后使用", 1001);
        change(
            &s,
            &owner,
            json!({"type":"used","id":recent["id"],"expectedItemRevision":recent["revision"]}),
            10000,
        );
        assert_eq!(
            prune(&s, &owner, 1001 + 12 * 60 * 60 * 1000).unwrap()["removed"],
            0
        );
        assert_eq!(
            prune(&s, &owner, 1002 + 12 * 60 * 60 * 1000).unwrap()["removed"],
            1
        );
        assert!(get(&s, &owner, old["id"].as_str().unwrap()).is_err());
        assert_eq!(list(&s, &owner)["total"], 2);
        change(&s, &owner, config(1, false), 1003 + 12 * 60 * 60 * 1000);
        assert_eq!(
            prune(&s, &owner, 10001 + 12 * 60 * 60 * 1000).unwrap()["removed"],
            1
        );
        assert_eq!(list(&s, &owner)["items"][0]["content"], "过期收藏");
    }

    #[test]
    fn clipboard_history_old_clear_after_receipt_rotation_still_cannot_erase_later_records() {
        let s = store();
        let owner = id();
        change(&s, &owner, config(0, true), 1000);
        add(&s, &owner, "旧", 1001);
        let rev = status(&s, &owner).unwrap()["revision"].clone();
        let clear = request(json!({"type":"clear","expectedRevision":rev}));
        mutate_at(&s, &owner, clear.clone(), 1002).unwrap();
        add(&s, &owner, "新", 1003);
        for n in 1..=65 {
            change(&s, &owner, config(n, true), 1004 + n);
        }
        let doc = load(&s.db.lock().unwrap(), &owner).unwrap();
        assert_eq!(doc.receipts.len(), 64);
        assert!(mutate_at(&s, &owner, clear, 2000)
            .unwrap_err()
            .contains("REVISION_CONFLICT"));
        assert_eq!(list(&s, &owner)["items"][0]["content"], "新");
    }

    #[test]
    fn clipboard_history_item_limit_and_unknown_protected_version_preserve_existing_state() {
        let s = store();
        let owner = id();
        let mut doc = Document::default();
        doc.revision = 1;
        doc.config_revision = 1;
        doc.settings.capture_enabled = true;
        doc.items = (0..MAX_ITEMS)
            .map(|n| Record {
                id: id(),
                revision: 1,
                content: format!("seed-{n}"),
                kind: "text".into(),
                labels: Vec::new(),
                favorite: n == 0,
                use_count: 0,
                create_time: 1000,
                update_time: 1000,
                latest_use_time: 1000,
            })
            .collect();
        save(&s.db.lock().unwrap(), &owner, &doc).unwrap();
        assert!(capture(&s, &owner, 1, "第1001条", 1001)
            .unwrap_err()
            .contains("CAPACITY_EXCEEDED"));
        assert_eq!(list(&s, &owner)["total"], 1000);
        let mut body = serde_json::to_value(&doc).unwrap();
        body["version"] = json!(2);
        let cipher =
            STANDARD.encode(crypt(&serde_json::to_vec(&body).unwrap(), &owner, false).unwrap());
        s.save_preference(&format!("clipboard.history/{owner}"), &cipher)
            .unwrap();
        assert!(status(&s, &owner)
            .unwrap_err()
            .starts_with("DATA_UNAVAILABLE:"));
        assert!(mutate_at(&s, &owner, request(config(1, false)), 1002).is_err());
        let raw: String =
            s.db.lock()
                .unwrap()
                .query_row(
                    "SELECT value FROM app_settings WHERE key=?1",
                    [format!("clipboard.history/{owner}")],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(raw, cipher);
    }
}
