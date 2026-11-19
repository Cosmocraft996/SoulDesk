//! AI直聘的百科、导图与模拟面试记录；只有显式生成预留能产生待发模型正文。
use crate::{
    job_surfaces::Knowledge,
    store::{err, AppResult},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
const MAX_SAFE: u64 = 9_007_199_254_740_991;
const DATA: &str = "DATA_UNAVAILABLE: 知识记录损坏或版本无效，原数据未覆盖";
const STORAGE: &str = "STORAGE_ERROR: 知识记录保存失败，本次事务未提交";
const EXPIRED: &str = "PRESENTATION_EXPIRED: 知识生成已停止、清空或替换，旧结果未写入";
fn uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|u| u.to_string() == value)
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn text(value: &str, max: usize) -> bool {
    value.len() <= max
        && !value
            .chars()
            .any(|c| c.is_control() && !['\n', '\r', '\t'].contains(&c))
}
pub(crate) fn valid_output(value: &str) -> bool {
    text(value, 512 * 1024)
}
fn next(value: u64) -> AppResult<u64> {
    value
        .checked_add(1)
        .filter(|n| *n <= MAX_SAFE)
        .ok_or_else(|| "LIMIT: 知识记录版本达到上限".into())
}
fn prefix(instance: &str) -> AppResult<String> {
    if !uuid(instance) {
        return Err("INVALID_ARGUMENT: 知识实例标识无效".into());
    }
    Ok(format!("job.chat/{instance}/"))
}
fn key(instance: &str, context: &Knowledge, kind: Kind) -> AppResult<String> {
    Ok(format!(
        "{}{}/{}",
        prefix(instance)?,
        context.id()?,
        kind.name()
    ))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    Wiki,
    Mindmap,
    Interview,
}
impl Kind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Wiki => "wiki",
            Self::Mindmap => "mindmap",
            Self::Interview => "interview",
        }
    }
    fn prompt(self) -> &'static str {
        match self {
            Self::Wiki => "wiki",
            Self::Mindmap => "mindMap",
            Self::Interview => "interviewer",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Phase {
    Running,
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Role {
    User,
    Assistant,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Message {
    role: Role,
    content: String,
    hidden: bool,
    at: u64,
    request_id: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: String,
    fingerprint: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    pub id: String,
    pub model_revision: u64,
    pub phase: Phase,
    pub partial: String,
    pub error: Option<String>,
    started_at: u64,
    updated_at: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Document {
    version: u8,
    pub revision: u64,
    pub context: Knowledge,
    pub kind: Kind,
    messages: Vec<Message>,
    pub job: Option<Request>,
    receipts: Vec<Receipt>,
}
fn greeting(context: &Knowledge) -> String {
    format!(
        "你好，我是AI面试官，我将面试你对{}掌握情况，准备好了吗？",
        context.knowledge
    )
}
impl Document {
    fn new(context: &Knowledge, kind: Kind) -> Self {
        Self {
            version: 1,
            revision: 0,
            context: context.clone(),
            kind,
            messages: if kind == Kind::Interview {
                vec![Message {
                    role: Role::Assistant,
                    content: greeting(context),
                    hidden: false,
                    at: 0,
                    request_id: None,
                }]
            } else {
                Vec::new()
            },
            job: None,
            receipts: Vec::new(),
        }
    }
    pub(crate) fn snapshot(&self) -> Value {
        let latest = self
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant && m.request_id.is_some())
            .map(|m| m.content.as_str());
        json!({"revision":self.revision,"contextId":self.context.id().ok(),"context":self.context,"kind":self.kind,"messages":self.messages,"latestText":latest.unwrap_or(""),"generated":latest.is_some(),"job":self.job})
    }
}
pub(crate) fn load(
    db: &Connection,
    instance: &str,
    context: &Knowledge,
    kind: Kind,
) -> AppResult<Document> {
    let raw: Option<String> = db
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [key(instance, context, kind)?],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| DATA)?;
    let Some(raw) = raw else {
        return Ok(Document::new(context, kind));
    };
    if raw.len() > 2 * 1024 * 1024 {
        return Err(DATA.into());
    }
    let d: Document = serde_json::from_str(&raw).map_err(|_| DATA)?;
    let mut ids = HashSet::new();
    if d.version != 1
        || d.revision > MAX_SAFE
        || d.context != *context
        || d.kind != kind
        || d.messages.len() > 100
        || d.receipts.len() > 64
        || d.receipts.len() as u64 > d.revision
        || d.receipts
            .iter()
            .any(|r| !uuid(&r.id) || !hash(&r.fingerprint) || !ids.insert(&r.id))
        || d.messages.iter().enumerate().any(|(n, m)| {
            !text(&m.content, 512 * 1024)
                || m.content.trim().is_empty()
                || m.at > MAX_SAFE
                || match &m.request_id {
                    Some(id) => !uuid(id),
                    None => {
                        !(n == 0
                            && kind == Kind::Interview
                            && m.role == Role::Assistant
                            && m.content == greeting(context))
                    }
                }
        })
        || d.job.as_ref().is_some_and(|r| {
            !uuid(&r.id)
                || r.model_revision == 0
                || r.model_revision > MAX_SAFE
                || !text(&r.partial, 512 * 1024)
                || r.started_at > r.updated_at
                || r.updated_at > MAX_SAFE
                || r.error.as_ref().is_some_and(|e| !text(e, 2000))
                || !d.receipts.iter().any(|receipt| receipt.id == r.id)
                || r.phase == Phase::Completed
                    && (!r.partial.is_empty()
                        || r.error.is_some()
                        || !d.messages.last().is_some_and(|m| {
                            m.role == Role::Assistant
                                && m.request_id.as_deref() == Some(r.id.as_str())
                        }))
                || r.phase != Phase::Completed
                    && !d.messages.last().is_some_and(|m| {
                        m.role == Role::User && m.request_id.as_deref() == Some(r.id.as_str())
                    })
                || r.phase == Phase::Running && r.error.is_some()
        })
        || d.job.is_none() && d.messages != Document::new(context, kind).messages
        || d.revision == 0 && (!d.receipts.is_empty() || d.job.is_some())
    {
        return Err(DATA.into());
    }
    Ok(d)
}
fn write(db: &Connection, instance: &str, d: &Document) -> AppResult<()> {
    let raw = serde_json::to_string(d).map_err(|_| STORAGE)?;
    if raw.len() > 2 * 1024 * 1024 {
        return Err("CAPACITY_EXCEEDED: 知识记录超过2MiB，未覆盖".into());
    }
    db.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key(instance,&d.context,d.kind)?,raw]).map_err(|_|STORAGE)?;
    Ok(())
}
fn capacity(db: &Connection, instance: &str) -> AppResult<()> {
    // 每个运行请求预留约1MiB转义后文本及元数据空间，已持久化片段不重复计数；不会在快满时发出无法保存结果的请求。
    let(count,used):(u64,u64)=db.query_row("SELECT COUNT(*),COALESCE(SUM(length(CAST(value AS BLOB))+CASE WHEN json_extract(value,'$.job.phase')='running' THEN max(0,1052672-(length(CAST(json_quote(json_extract(value,'$.job.partial')) AS BLOB))-2)) ELSE 0 END),0) FROM app_settings WHERE key LIKE ?1",[format!("{}%",prefix(instance)?)],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|DATA)?;
    if count > 384 || used > 32 * 1024 * 1024 {
        return Err("CAPACITY_EXCEEDED: 知识库达到容量上限，请先整理记录；未发出新请求".into());
    }
    Ok(())
}
fn receipt(d: &mut Document, id: &str, fingerprint: String) {
    d.receipts.push(Receipt {
        id: id.into(),
        fingerprint,
    });
    if d.receipts.len() > 64 {
        d.receipts.remove(0);
    }
}
fn fingerprint(value: &impl Serialize) -> AppResult<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(err)?)
    ))
}
fn filled(template: &str, context: &Knowledge) -> AppResult<String> {
    let mut rest = template;
    let mut result = String::new();
    while let Some(start) = rest.find("@{") {
        result.push_str(&rest[..start]);
        let token = &rest[start + 2..];
        let end = token
            .find('}')
            .ok_or("DATA_UNAVAILABLE: 原知识提示词占位符无效")?;
        result.push_str(match &token[..end] {
            "knowledge" => &context.knowledge,
            "jobName" => &context.job_name,
            "industry" => &context.industry,
            "extra" => &context.extra,
            _ => return Err("DATA_UNAVAILABLE: 未知原知识提示词占位符".into()),
        });
        rest = &token[end + 1..];
    }
    result.push_str(rest);
    Ok(result)
}
fn base(context: &Knowledge) -> String {
    let mut parts = vec![format!("我想了解的职业知识/技能：{}。", context.knowledge)];
    if !context.job_name.is_empty() {
        parts.push(format!("我想找一个关于{}的工作。", context.job_name));
    }
    if !context.industry.is_empty() {
        parts.push(format!("目标行业：{}。", context.industry));
    }
    if !context.extra.is_empty() {
        parts.push(format!("其他说明：{}。", context.extra));
    }
    parts.join("\n")
}
fn input(d: &Document, text_value: Option<&str>) -> AppResult<Message> {
    let prompts: Value = serde_json::from_str(include_str!("../component-assets/job-prompts.json"))
        .map_err(|_| "DATA_UNAVAILABLE: 原知识提示词资源无效")?;
    let content = if d.kind == Kind::Interview {
        let value = text_value
            .filter(|v| !v.trim().is_empty() && v.chars().count() <= 10000 && text(v, 40000))
            .ok_or("INVALID_ARGUMENT: 面试输入须为1到10000字")?;
        value.to_owned()
    } else {
        if text_value.is_some() {
            return Err("INVALID_ARGUMENT: 百科/导图生成不接收聊天文本，请使用上下文说明".into());
        }
        filled(
            prompts[d.kind.prompt()]
                .as_str()
                .ok_or("DATA_UNAVAILABLE: 原知识提示词缺失")?,
            &d.context,
        )?
    };
    Ok(Message {
        role: Role::User,
        content,
        hidden: d.kind != Kind::Interview,
        at: 0,
        request_id: None,
    })
}
fn body(d: &Document, model: &str) -> AppResult<Value> {
    if model.trim().is_empty() || model.len() > 200 || model.chars().any(char::is_control) {
        return Err("INVALID_ARGUMENT: 模型名称无效".into());
    }
    let mut system = base(&d.context);
    if d.kind == Kind::Interview {
        let prompts: Value =
            serde_json::from_str(include_str!("../component-assets/job-prompts.json"))
                .map_err(|_| "DATA_UNAVAILABLE: 原面试提示词资源无效")?;
        system.push('\n');
        system.push_str(&filled(
            prompts["interviewer"]
                .as_str()
                .ok_or("DATA_UNAVAILABLE: 原面试提示词缺失")?,
            &d.context,
        )?);
    }
    let mut messages = vec![json!({"role":"system","content":system})];
    messages.extend(
        d.messages
            .iter()
            .map(|m| json!({"role":m.role,"content":m.content})),
    );
    let value = json!({"model":model.trim(),"stream":true,"max_tokens":8192,"messages":messages});
    if serde_json::to_vec(&value).map_err(err)?.len() > 64 * 1024 {
        return Err(
            "CAPACITY_EXCEEDED: 完整聊天历史超过64KiB，请清空后重新开始；未截断或请求模型".into(),
        );
    }
    Ok(value)
}
pub(crate) struct Reserved {
    pub document: Document,
    pub body: Option<Value>,
}
pub(crate) fn reserve(
    db: &mut Connection,
    instance: &str,
    context: &Knowledge,
    kind: Kind,
    id: &str,
    expected: u64,
    model_revision: u64,
    package_digest: &str,
    model: &str,
    text_value: Option<&str>,
    now: u64,
) -> AppResult<Reserved> {
    if !uuid(id)
        || expected > MAX_SAFE
        || model_revision == 0
        || model_revision > MAX_SAFE
        || now > MAX_SAFE
    {
        return Err("INVALID_ARGUMENT: 知识请求身份、版本或时间无效".into());
    }
    let signature = fingerprint(&(
        "generate",
        expected,
        kind,
        context,
        model_revision,
        package_digest,
        text_value,
    ))?;
    let tx = db.transaction().map_err(|_| STORAGE)?;
    let mut d = load(&tx, instance, context, kind)?;
    if let Some(r) = d.receipts.iter().find(|r| r.id == id) {
        if r.fingerprint != signature {
            return Err("REQUEST_CONFLICT: 同一知识请求不能改变输入或模型配置".into());
        }
        return Ok(Reserved {
            document: d,
            body: None,
        });
    }
    if d.revision != expected {
        return Err("REVISION_CONFLICT: 知识记录已有新版本，请保留输入后重新读取".into());
    }
    if d.job.as_ref().is_some_and(|r| r.phase == Phase::Running) {
        return Err("BUSY: 当前知识任务正在生成，请先停止".into());
    }
    if d.messages.len() > 98 {
        return Err("CAPACITY_EXCEEDED: 面试或生成历史过长，请清空后重新开始".into());
    }
    let mut message = input(&d, text_value)?;
    message.at = now;
    message.request_id = Some(id.into());
    d.messages.push(message);
    let request = body(&d, model)?;
    d.revision = next(d.revision)?;
    d.job = Some(Request {
        id: id.into(),
        model_revision,
        phase: Phase::Running,
        partial: String::new(),
        error: None,
        started_at: now,
        updated_at: now,
    });
    receipt(&mut d, id, signature);
    write(&tx, instance, &d)?;
    capacity(&tx, instance)?;
    tx.commit().map_err(|_| STORAGE)?;
    Ok(Reserved {
        document: d,
        body: Some(request),
    })
}
pub(crate) fn update(
    db: &mut Connection,
    instance: &str,
    context: &Knowledge,
    kind: Kind,
    id: &str,
    model_revision: u64,
    partial: &str,
    phase: Phase,
    error: Option<&str>,
    now: u64,
) -> AppResult<Document> {
    if !text(partial, 512 * 1024)
        || phase == Phase::Completed && (partial.trim().is_empty() || error.is_some())
        || phase == Phase::Running && error.is_some()
        || error.is_some_and(|e| !text(e, 2000))
        || now > MAX_SAFE
    {
        return Err("DATA_UNAVAILABLE: 知识输出为空、超长或无效，未写入完成记录".into());
    }
    let tx = db.transaction().map_err(|_| STORAGE)?;
    let mut d = load(&tx, instance, context, kind)?;
    let job = d
        .job
        .as_mut()
        .filter(|r| r.id == id && r.model_revision == model_revision && r.phase == Phase::Running)
        .ok_or(EXPIRED)?;
    job.phase = phase;
    job.partial = partial.into();
    job.error = error.map(str::to_owned);
    job.updated_at = now.max(job.updated_at);
    if phase == Phase::Completed {
        job.partial.clear();
        job.error = None;
        d.messages.push(Message {
            role: Role::Assistant,
            content: partial.into(),
            hidden: false,
            at: now,
            request_id: Some(id.into()),
        });
    }
    if phase != Phase::Running {
        d.revision = next(d.revision)?;
    }
    write(&tx, instance, &d)?;
    capacity(&tx, instance)?;
    tx.commit().map_err(|_| STORAGE)?;
    Ok(d)
}
pub(crate) fn clear(
    db: &mut Connection,
    instance: &str,
    context: &Knowledge,
    kind: Kind,
    id: &str,
    expected: u64,
) -> AppResult<(Document, bool)> {
    if !uuid(id) || expected > MAX_SAFE {
        return Err("INVALID_ARGUMENT: 清空知识记录请求无效".into());
    }
    let signature = fingerprint(&("clear", expected, kind, context))?;
    let tx = db.transaction().map_err(|_| STORAGE)?;
    let mut d = load(&tx, instance, context, kind)?;
    if let Some(r) = d.receipts.iter().find(|r| r.id == id) {
        if r.fingerprint != signature {
            return Err("REQUEST_CONFLICT: 同一请求标识不能用于其他知识操作".into());
        }
        return Ok((d, true));
    }
    if d.revision != expected {
        return Err("REVISION_CONFLICT: 知识记录已有新版本，未清空".into());
    }
    let old_revision = d.revision;
    let receipts = d.receipts;
    d = Document::new(context, kind);
    d.receipts = receipts;
    d.revision = next(old_revision)?;
    receipt(&mut d, id, signature);
    write(&tx, instance, &d)?;
    tx.commit().map_err(|_| STORAGE)?;
    Ok((d, false))
}
pub(crate) fn recover(
    db: &mut Connection,
    instance: &str,
    context: &Knowledge,
    kind: Kind,
    now: u64,
) -> AppResult<Document> {
    let d = load(db, instance, context, kind)?;
    if let Some(r) = d.job.as_ref().filter(|r| r.phase == Phase::Running) {
        return update(
            db,
            instance,
            context,
            kind,
            &r.id,
            r.model_revision,
            &r.partial,
            Phase::Interrupted,
            Some("上次生成已中断，请手动决定是否重新生成；未自动补发请求"),
            now,
        );
    }
    Ok(d)
}
pub(crate) fn remove_data(db: &Connection, instance: &str) -> AppResult<()> {
    db.execute(
        "DELETE FROM app_settings WHERE key LIKE ?1",
        [format!("{}%", prefix(instance)?)],
    )
    .map_err(|_| "STORAGE_ERROR: 知识记录清理失败")?;
    Ok(())
}
pub(crate) fn running(db: &Connection, instance: &str) -> AppResult<Vec<(Knowledge, Kind)>> {
    let mut stmt=db.prepare("SELECT key,value FROM app_settings WHERE key LIKE ?1 AND json_extract(value,'$.job.phase')='running' LIMIT 385").map_err(|_|DATA)?;
    let rows = stmt
        .query_map([format!("{}%", prefix(instance)?)], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|_| DATA)?;
    let mut result = Vec::new();
    for row in rows {
        let (k, raw) = row.map_err(|_| DATA)?;
        if raw.len() > 2 * 1024 * 1024 {
            return Err(DATA.into());
        }
        let d: Document = serde_json::from_str(&raw).map_err(|_| DATA)?;
        if k != key(instance, &d.context, d.kind)? {
            return Err(DATA.into());
        }
        load(db, instance, &d.context, d.kind)?;
        result.push((d.context, d.kind));
    }
    if result.len() > 384 {
        return Err(DATA.into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn uid() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn context() -> Knowledge {
        Knowledge {
            knowledge: "Vue".into(),
            job_name: "前端开发".into(),
            industry: "互联网".into(),
            extra: "性能与测试".into(),
        }
    }
    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE app_settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);")
            .unwrap();
        c
    }
    fn start(
        db: &mut Connection,
        i: &str,
        c: &Knowledge,
        k: Kind,
        id: &str,
        expected: u64,
    ) -> Reserved {
        reserve(
            db,
            i,
            c,
            k,
            id,
            expected,
            1,
            &"a".repeat(64),
            "model",
            if k == Kind::Interview {
                Some("开始")
            } else {
                None
            },
            1,
        )
        .unwrap()
    }
    #[test]
    fn opening_records_never_reserves_a_request_or_writes_to_storage() {
        let db = db();
        let i = uid();
        for k in [Kind::Wiki, Kind::Mindmap, Kind::Interview] {
            let d = load(&db, &i, &context(), k).unwrap();
            assert!(d.job.is_none());
            assert_eq!(d.revision, 0);
            assert_eq!(d.messages.len(), usize::from(k == Kind::Interview));
        }
        let count: u64 = db
            .query_row("SELECT COUNT(*) FROM app_settings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
    #[test]
    fn generation_preserves_original_prompt_history_and_every_context_field() {
        let mut db = db();
        let i = uid();
        let id = uid();
        let d = start(&mut db, &i, &context(), Kind::Wiki, &id, 0);
        let b = d.body.unwrap();
        assert!(b["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("前端开发"));
        assert!(b["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("性能与测试"));
        assert!(d.document.messages[0].hidden);
        assert_eq!(b["stream"], true);
        let d = update(
            &mut db,
            &i,
            &context(),
            Kind::Wiki,
            &id,
            1,
            "# Vue百科",
            Phase::Completed,
            None,
            2,
        )
        .unwrap();
        assert_eq!(d.revision, 2);
        assert_eq!(d.snapshot()["latestText"], "# Vue百科");
        let next = start(&mut db, &i, &context(), Kind::Wiki, &uid(), 2);
        assert_eq!(next.body.unwrap()["messages"].as_array().unwrap().len(), 4);
    }
    #[test]
    fn interview_starts_with_a_local_greeting_and_uses_complete_messages() {
        let mut db = db();
        let i = uid();
        let c = context();
        let d = start(&mut db, &i, &c, Kind::Interview, &uid(), 0);
        let body = d.body.unwrap();
        let m = body["messages"].as_array().unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!(m[0]["role"], "system");
        assert!(m[0]["content"].as_str().unwrap().contains("每次只出一道题"));
        assert_eq!(m[1]["role"], "assistant");
        assert_eq!(m[2]["content"], "开始");
    }
    #[test]
    fn replay_never_returns_a_second_body_and_old_callbacks_cannot_cross_clear() {
        let mut db = db();
        let i = uid();
        let c = context();
        let id = uid();
        let d = start(&mut db, &i, &c, Kind::Wiki, &id, 0);
        assert!(d.body.is_some());
        assert!(start(&mut db, &i, &c, Kind::Wiki, &id, 0).body.is_none());
        let (d, _) = clear(&mut db, &i, &c, Kind::Wiki, &uid(), 1).unwrap();
        assert_eq!(d.revision, 2);
        assert!(start(&mut db, &i, &c, Kind::Wiki, &id, 0).body.is_none());
        assert!(update(
            &mut db,
            &i,
            &c,
            Kind::Wiki,
            &id,
            1,
            "旧答案",
            Phase::Completed,
            None,
            3
        )
        .is_err());
        assert!(load(&db, &i, &c, Kind::Wiki).unwrap().messages.is_empty());
    }
    #[test]
    fn model_versions_partial_output_and_interrupted_work_are_preserved_without_completion() {
        let mut db = db();
        let i = uid();
        let c = context();
        let id = uid();
        start(&mut db, &i, &c, Kind::Mindmap, &id, 0);
        update(
            &mut db,
            &i,
            &c,
            Kind::Mindmap,
            &id,
            1,
            "# 部分导图",
            Phase::Running,
            None,
            2,
        )
        .unwrap();
        assert!(update(
            &mut db,
            &i,
            &c,
            Kind::Mindmap,
            &id,
            2,
            "错模型结果",
            Phase::Completed,
            None,
            3
        )
        .is_err());
        let d = recover(&mut db, &i, &c, Kind::Mindmap, 4).unwrap();
        assert_eq!(d.job.as_ref().unwrap().phase, Phase::Interrupted);
        assert_eq!(d.job.as_ref().unwrap().partial, "# 部分导图");
        assert_eq!(d.snapshot()["generated"], false);
        assert!(d.messages.iter().all(|m| m.role != Role::Assistant));
        assert!(update(
            &mut db,
            &i,
            &c,
            Kind::Mindmap,
            &id,
            1,
            "迟到结果",
            Phase::Completed,
            None,
            5
        )
        .is_err());
    }
    #[test]
    fn contexts_tasks_and_instances_never_share_records() {
        let mut db = db();
        let i = uid();
        let mut c = context();
        start(&mut db, &i, &c, Kind::Wiki, &uid(), 0);
        c.extra = "另一说明".into();
        assert!(load(&db, &i, &c, Kind::Wiki).unwrap().job.is_none());
        assert!(load(&db, &i, &context(), Kind::Mindmap)
            .unwrap()
            .job
            .is_none());
        assert!(load(&db, &uid(), &context(), Kind::Wiki)
            .unwrap()
            .job
            .is_none());
    }
    #[test]
    fn failed_reservation_and_failed_completion_leave_no_partial_transaction() {
        let mut db = db();
        let i = uid();
        let c = context();
        db.execute_batch("CREATE TEMP TRIGGER reject_save BEFORE INSERT ON app_settings BEGIN SELECT RAISE(ABORT,'故障注入'); END;").unwrap();
        assert!(reserve(
            &mut db,
            &i,
            &c,
            Kind::Wiki,
            &uid(),
            0,
            1,
            "digest",
            "model",
            None,
            1
        )
        .is_err());
        assert!(load(&db, &i, &c, Kind::Wiki).unwrap().job.is_none());
        db.execute_batch("DROP TRIGGER reject_save;").unwrap();
        let id = uid();
        start(&mut db, &i, &c, Kind::Wiki, &id, 0);
        db.execute_batch("CREATE TEMP TRIGGER reject_update BEFORE UPDATE ON app_settings BEGIN SELECT RAISE(ABORT,'故障注入'); END;").unwrap();
        assert!(update(
            &mut db,
            &i,
            &c,
            Kind::Wiki,
            &id,
            1,
            "完整答案",
            Phase::Completed,
            None,
            2
        )
        .is_err());
        let d = load(&db, &i, &c, Kind::Wiki).unwrap();
        assert_eq!(d.job.unwrap().phase, Phase::Running);
        assert_eq!(d.messages.len(), 1);
    }
    #[test]
    fn placeholder_values_are_substituted_once_and_invalid_or_oversized_inputs_are_rejected() {
        let mut c = context();
        c.knowledge = "@{extra}".into();
        assert_eq!(
            filled("@{knowledge}|@{extra}", &c).unwrap(),
            "@{extra}|性能与测试"
        );
        let mut db = db();
        let i = uid();
        assert!(reserve(
            &mut db,
            &i,
            &c,
            Kind::Wiki,
            &uid(),
            0,
            1,
            "digest",
            "model",
            Some("任意聊天"),
            1
        )
        .is_err());
        assert!(reserve(
            &mut db,
            &i,
            &c,
            Kind::Interview,
            &uid(),
            0,
            1,
            "digest",
            "model",
            Some(" "),
            1
        )
        .is_err());
        let id = uid();
        start(&mut db, &i, &c, Kind::Wiki, &id, 0);
        assert!(update(
            &mut db,
            &i,
            &c,
            Kind::Wiki,
            &id,
            1,
            &"x".repeat(512 * 1024 + 1),
            Phase::Completed,
            None,
            2
        )
        .is_err());
        assert!(update(
            &mut db,
            &i,
            &c,
            Kind::Wiki,
            &id,
            1,
            " ",
            Phase::Completed,
            None,
            2
        )
        .is_err());
    }
    #[test]
    fn full_history_limit_is_checked_before_reservation_and_never_silently_truncates() {
        let mut db = db();
        let i = uid();
        let c = context();
        let id = uid();
        start(&mut db, &i, &c, Kind::Interview, &id, 0);
        update(
            &mut db,
            &i,
            &c,
            Kind::Interview,
            &id,
            1,
            &"长".repeat(30000),
            Phase::Completed,
            None,
            2,
        )
        .unwrap();
        assert!(
            matches!(reserve(&mut db,&i,&c,Kind::Interview,&uid(),2,1,"digest","model",Some("继续"),3),Err(e)if e.starts_with("CAPACITY_EXCEEDED:"))
        );
        let d = load(&db, &i, &c, Kind::Interview).unwrap();
        assert_eq!(d.revision, 2);
        assert_eq!(d.job.unwrap().id, id);
    }
    #[test]
    fn reserved_capacity_is_not_counted_twice_as_streamed_text_is_saved() {
        let mut db = db();
        let i = uid();
        let mut accepted = Vec::new();
        for n in 0..40 {
            let mut c = context();
            c.extra = format!("上下文{n}");
            let id = uid();
            match reserve(
                &mut db,
                &i,
                &c,
                Kind::Wiki,
                &id,
                0,
                1,
                "digest",
                "model",
                None,
                1,
            ) {
                Ok(_) => accepted.push((c, id)),
                Err(e) => {
                    assert!(e.starts_with("CAPACITY_EXCEEDED:"));
                    assert!(load(&db, &i, &c, Kind::Wiki).unwrap().job.is_none());
                    break;
                }
            }
        }
        assert!(!accepted.is_empty() && accepted.len() < 40);
        let (c, id) = &accepted[0];
        let partial = "x".repeat(200000);
        update(
            &mut db,
            &i,
            c,
            Kind::Wiki,
            id,
            1,
            &partial,
            Phase::Running,
            None,
            2,
        )
        .unwrap();
        update(
            &mut db,
            &i,
            c,
            Kind::Wiki,
            id,
            1,
            &partial,
            Phase::Completed,
            None,
            3,
        )
        .unwrap();
        assert_eq!(
            load(&db, &i, c, Kind::Wiki).unwrap().job.unwrap().phase,
            Phase::Completed
        );
    }
}
