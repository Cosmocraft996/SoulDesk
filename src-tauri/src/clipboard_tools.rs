//! 剪切板固定工具窗口与独立加密文档；没有网络或系统剪切板调用。
use crate::{
    ai_protocol::Task,
    clipboard_history,
    store::{err, AppResult},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Settings,
    Mindmap,
    Translate,
}
impl Kind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::Mindmap => "mindmap",
            Self::Translate => "translate",
        }
    }
    pub(crate) fn task(self) -> Option<Task> {
        match self {
            Self::Settings => None,
            Self::Mindmap => Some(Task::Mindmap),
            Self::Translate => Some(Task::Translate),
        }
    }
    pub(crate) fn scope(self, instance: &str) -> String {
        format!("{instance}/{}", self.name())
    }
}
pub(crate) fn kind(name: &str) -> AppResult<Kind> {
    match name {
        "settings" => Ok(Kind::Settings),
        "mindmap" => Ok(Kind::Mindmap),
        "translate" => Ok(Kind::Translate),
        _ => Err("INVALID_ARGUMENT: 仅支持固定剪切板工具".into()),
    }
}
pub(crate) fn role(label: &str, instance: &str) -> Option<Kind> {
    if uuid::Uuid::parse_str(instance).ok()?.to_string() != instance {
        return None;
    }
    for k in [Kind::Settings, Kind::Mindmap, Kind::Translate] {
        if let Some(id) = label.strip_prefix(&format!("clipboard-tool-{}-{instance}-", k.name())) {
            if uuid::Uuid::parse_str(id).is_ok_and(|v| v.to_string() == id) {
                return Some(k);
            }
        }
    }
    None
}
pub(crate) fn method_allowed(label: &str, method: &str) -> AppResult<()> {
    if !label.starts_with("clipboard-tool-") {
        return Ok(());
    }
    let common = matches!(
        method,
        "runtime.ready"
            | "settings.get"
            | "ai.status"
            | "ai.configure"
            | "ai.clear"
            | "ai.toolInfo"
            | "clipboard.hide"
    );
    let allowed = if label.starts_with("clipboard-tool-settings-") {
        common || matches!(method, "clipboard.status" | "clipboard.mutate")
    } else {
        common
            || matches!(
                method,
                "ai.begin"
                    | "ai.poll"
                    | "ai.cancel"
                    | "ai.document.get"
                    | "ai.document.set"
                    | "documents.save"
            )
    };
    if allowed {
        Ok(())
    } else {
        Err("CAPABILITY_DENIED: 剪切板工具仅开放固定任务、文档和关闭".into())
    }
}
#[derive(Clone)]
pub(crate) struct Tool {
    pub instance: String,
    pub id: String,
    pub label: String,
    pub kind: Kind,
    pub content: String,
    pub config_revision: u64,
}
#[derive(Default)]
pub(crate) struct Book {
    pub entries: HashMap<String, Tool>,
    pub closing: Vec<String>,
}
impl Book {
    pub(crate) fn begin(
        &mut self,
        instance: &str,
        kind: Kind,
        content: &str,
        config_revision: u64,
    ) -> AppResult<Tool> {
        if uuid::Uuid::parse_str(instance).is_err()
            || kind != Kind::Settings
                && (content.trim().is_empty()
                    || content.chars().count() > 10000
                    || content.contains('\0'))
            || kind == Kind::Settings && !content.is_empty()
        {
            return Err("INVALID_ARGUMENT: 工具文本须为1至10000字，不含空字符".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let p = Tool {
            instance: instance.into(),
            label: format!("clipboard-tool-{}-{instance}-{id}", kind.name()),
            id,
            kind,
            content: content.into(),
            config_revision,
        };
        if let Some(old) = self.entries.insert(kind.scope(instance), p.clone()) {
            self.closing.push(old.label);
        }
        Ok(p)
    }
    pub(crate) fn get(&self, label: &str, instance: &str) -> AppResult<Tool> {
        let k = role(label, instance).ok_or("CAPABILITY_DENIED: 工具角色无效")?;
        self.entries
            .get(&k.scope(instance))
            .filter(|p| p.label == label)
            .cloned()
            .ok_or("PRESENTATION_EXPIRED: 工具窗口已关闭或替换".into())
    }
    pub(crate) fn close(&mut self, label: &str) {
        self.entries.retain(|_, p| p.label != label);
    }
    pub(crate) fn forget(&mut self, instance: &str) {
        self.entries.retain(|_, p| {
            if p.instance == instance {
                self.closing.push(p.label.clone());
                false
            } else {
                true
            }
        });
    }
}
fn doc_key(instance: &str, kind: Kind) -> AppResult<String> {
    if uuid::Uuid::parse_str(instance).is_err() || kind.task().is_none() {
        return Err("CAPABILITY_DENIED: 非AI工具文档".into());
    }
    Ok(format!("clipboard.tool-document/{}", kind.scope(instance)))
}
pub(crate) fn document_get(db: &Connection, instance: &str, kind: Kind) -> AppResult<Value> {
    let key = doc_key(instance, kind)?;
    let raw: Option<String> = db
        .query_row("SELECT value FROM app_settings WHERE key=?1", [&key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(err)?;
    let Some(raw) = raw else {
        return Ok(json!({}));
    };
    if raw.len() > 3 * 1024 * 1024 {
        return Err("DATA_UNAVAILABLE: 工具文档超限，原数据未覆盖".into());
    }
    let encoded = raw
        .strip_prefix("dpapi-tool-v1:")
        .ok_or("DATA_UNAVAILABLE: 工具文档版本无效")?;
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| "DATA_UNAVAILABLE: 工具文档编码无效")?;
    let bytes = clipboard_history::crypt(&bytes, &key, true)
        .map_err(|_| "DATA_UNAVAILABLE: 工具文档损坏或不属于当前用户/工具")?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("DATA_UNAVAILABLE: 工具文档超限".into());
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "DATA_UNAVAILABLE: 工具文档内容无效")?;
    if !value.is_object() {
        return Err("DATA_UNAVAILABLE: 工具文档须为对象".into());
    }
    Ok(value)
}
pub(crate) fn document_set(
    db: &Connection,
    instance: &str,
    kind: Kind,
    value: Value,
) -> AppResult<Value> {
    let key = doc_key(instance, kind)?;
    document_get(db, instance, kind)?;
    let raw = serde_json::to_vec(&value).map_err(err)?;
    if !value.is_object() || raw.len() > 2 * 1024 * 1024 {
        return Err("CAPACITY_EXCEEDED: 工具文档须为不超过2MiB的对象".into());
    }
    let protected = clipboard_history::crypt(&raw, &key, false)?;
    let raw = format!("dpapi-tool-v1:{}", STANDARD.encode(protected));
    db.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![key,raw]).map_err(|_|"STORAGE_ERROR: 工具文档保存失败")?;
    Ok(value)
}
pub(crate) fn remove_data(db: &Connection, instance: &str) -> AppResult<()> {
    for k in [Kind::Mindmap, Kind::Translate] {
        db.execute(
            "DELETE FROM app_settings WHERE key=?1",
            [doc_key(instance, k)?],
        )
        .map_err(err)?;
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn uid() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    #[test]
    fn clipboard_tools_fixed_roles_parallel_tools_and_replacement_are_isolated() {
        let a = uid();
        let b = uid();
        let mut book = Book::default();
        let map = book.begin(&a, Kind::Mindmap, "文本", 1).unwrap();
        let translate = book.begin(&a, Kind::Translate, "翻译", 1).unwrap();
        let other = book.begin(&b, Kind::Mindmap, "其他", 0).unwrap();
        assert_eq!(role(&map.label, &a), Some(Kind::Mindmap));
        assert!(role(&map.label, &b).is_none());
        let new = book.begin(&a, Kind::Mindmap, "新内容", 1).unwrap();
        assert!(book.get(&map.label, &a).is_err());
        assert!(book.get(&translate.label, &a).is_ok());
        assert!(book.get(&new.label, &a).is_ok());
        assert!(book.get(&other.label, &b).is_ok());
        book.forget(&a);
        assert!(book.get(&translate.label, &a).is_err());
        assert!(book.get(&other.label, &b).is_ok());
        assert!(book.begin(&a, Kind::Translate, "", 0).is_err());
        assert!(book
            .begin(&a, Kind::Mindmap, &"a".repeat(10001), 0)
            .is_err());
    }
    #[test]
    fn clipboard_tools_methods_do_not_allow_history_system_or_arbitrary_network() {
        for k in [Kind::Settings, Kind::Mindmap, Kind::Translate] {
            let label = format!("clipboard-tool-{}-{}-{}", k.name(), uid(), uid());
            for m in [
                "clipboard.query",
                "clipboard.act",
                "clipboard.peek",
                "system.metrics",
                "storage.set",
                "documents.open",
                "window.hide",
            ] {
                assert!(method_allowed(&label, m).is_err(), "{m}");
            }
            assert!(method_allowed(&label, "ai.configure").is_ok());
            assert_eq!(
                method_allowed(&label, "ai.begin").is_ok(),
                k != Kind::Settings
            );
        }
    }
    #[test]
    fn clipboard_tools_documents_are_encrypted_and_separated_by_instance_and_task() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE app_settings(key TEXT PRIMARY KEY,value TEXT)")
            .unwrap();
        let a = uid();
        let b = uid();
        let map = json!({"content":"剪切板私密文本","result":"本地结果"});
        document_set(&db, &a, Kind::Mindmap, map.clone()).unwrap();
        document_set(&db, &a, Kind::Translate, json!({"content":"翻译"})).unwrap();
        assert_eq!(document_get(&db, &a, Kind::Mindmap).unwrap(), map);
        assert_eq!(document_get(&db, &b, Kind::Mindmap).unwrap(), json!({}));
        let raw: String = db
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                [doc_key(&a, Kind::Mindmap).unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!raw.contains("剪切板私密文本"));
        db.execute(
            "INSERT INTO app_settings(key,value)VALUES(?1,?2)",
            rusqlite::params![doc_key(&b, Kind::Mindmap).unwrap(), raw],
        )
        .unwrap();
        assert!(document_get(&db, &b, Kind::Mindmap).is_err());
        assert!(document_set(&db, &b, Kind::Mindmap, json!({})).is_err());
        remove_data(&db, &a).unwrap();
        assert!(document_get(&db, &a, Kind::Translate)
            .unwrap()
            .as_object()
            .unwrap()
            .is_empty());
        assert!(document_get(&db, &b, Kind::Mindmap).is_err());
    }
    #[test]
    fn clipboard_tools_document_failure_preserves_other_document_and_prior_bytes() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE app_settings(key TEXT PRIMARY KEY,value TEXT)")
            .unwrap();
        let a = uid();
        document_set(&db, &a, Kind::Translate, json!({"content":"旧"})).unwrap();
        assert!(document_set(
            &db,
            &a,
            Kind::Translate,
            json!({"content":"x".repeat(2*1024*1024)})
        )
        .is_err());
        assert_eq!(
            document_get(&db, &a, Kind::Translate).unwrap()["content"],
            "旧"
        );
        db.execute_batch("CREATE TRIGGER refuse_tool BEFORE INSERT ON app_settings BEGIN SELECT RAISE(FAIL,'denied'); END;").unwrap();
        assert!(document_set(&db, &a, Kind::Translate, json!({"content":"新"})).is_err());
        assert_eq!(
            document_get(&db, &a, Kind::Translate).unwrap()["content"],
            "旧"
        );
    }
}
