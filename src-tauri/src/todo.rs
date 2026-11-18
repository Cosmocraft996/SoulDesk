//! 固定待办数据与提醒后台；隐藏视图不取消已保存事项。
use crate::{
    store::{err, now_ms, AppResult, Store},
    widgets, Core,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Datelike, Days, Local, NaiveDate, SecondsFormat, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    sync::atomic::{AtomicBool, Ordering},
};
use tauri::{AppHandle, Manager};
use uuid::Uuid;
const CAPABILITY: &str = "todos.manage";
const LIMIT: usize = 512;
const BYTE_LIMIT: usize = 4 * 1024 * 1024;
const REMINDER_WINDOW: i64 = 30_000;
fn normal() -> String {
    "normal".into()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Todo {
    id: String,
    title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    due_date_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reminder_date_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed_date_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    start_date_time: Option<String>,
    #[serde(default)]
    created_date_time: String,
    #[serde(default)]
    last_modified_date_time: String,
    #[serde(default)]
    is_reminder_on: bool,
    #[serde(default)]
    recurrence: String,
    #[serde(default = "normal")]
    importance: String,
    #[serde(default)]
    order: u32,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Reminder {
    todo_id: String,
    at: i64,
    signature: String,
    nonce: String,
    delivery: String,
    error: Option<String>,
    expires_at: i64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: String,
    fingerprint: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    revision: u64,
    todos: Vec<Todo>,
    reminders: Vec<Reminder>,
    receipts: Vec<Receipt>,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            todos: Vec::new(),
            reminders: Vec::new(),
            receipts: Vec::new(),
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Change {
    request_id: String,
    expected_revision: u64,
    operation: String,
    todo: Option<Todo>,
    id: Option<String>,
    ids: Option<Vec<String>>,
}
#[derive(Clone, Debug)]
struct Pending {
    todo_id: String,
    title: String,
    nonce: String,
    signature: String,
    at: i64,
    expires_at: i64,
}
#[derive(Default)]
pub struct Service {
    busy: AtomicBool,
}
fn key(id: &str) -> String {
    format!("todo.document:{}", URL_SAFE_NO_PAD.encode(id))
}
fn hash(value: &Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}
fn signature(t: &Todo) -> String {
    hash(&json!([
        t.id,
        t.title,
        t.due_date_time,
        t.reminder_date_time,
        t.completed_date_time,
        t.is_reminder_on,
        t.recurrence
    ]))
}
fn date(value: &str) -> AppResult<DateTime<Utc>> {
    if value.len() > 40 {
        return Err("INVALID_ARGUMENT: 待办日期无效".into());
    }
    let d = DateTime::parse_from_rfc3339(value)
        .map_err(err)?
        .with_timezone(&Utc);
    if !(1..=9999).contains(&d.year()) || d.timestamp_subsec_nanos() >= 1_000_000_000 {
        return Err("INVALID_ARGUMENT: 待办日期超出范围".into());
    }
    Ok(d)
}
fn iso(at: i64) -> AppResult<String> {
    Ok(DateTime::<Utc>::from_timestamp_millis(at)
        .ok_or("待办时间超出范围")?
        .to_rfc3339_opts(SecondsFormat::Millis, true))
}
fn validate(t: &mut Todo) -> AppResult<()> {
    Uuid::parse_str(&t.id).map_err(|_| "INVALID_ARGUMENT: 待办标识须为UUID".to_owned())?;
    if t.title.trim().is_empty() || t.title.chars().count() > 4096 {
        return Err("INVALID_ARGUMENT: 待办内容须为1～4096字".into());
    }
    if ![
        "",
        "RRULE:FREQ=DAILY",
        "RRULE:FREQ=WEEKLY",
        "RRULE:FREQ=MONTHLY",
        "RRULE:FREQ=YEARLY",
    ]
    .contains(&t.recurrence.as_str())
    {
        return Err("INVALID_ARGUMENT: 不支持此待办重复规则".into());
    }
    if !["normal", "high", "low"].contains(&t.importance.as_str()) {
        return Err("INVALID_ARGUMENT: 待办重要性无效".into());
    }
    for value in [
        &mut t.due_date_time,
        &mut t.reminder_date_time,
        &mut t.completed_date_time,
        &mut t.start_date_time,
    ]
    .into_iter()
    .flatten()
    {
        *value = date(value)?.to_rfc3339_opts(SecondsFormat::Millis, true);
    }
    for value in [&mut t.created_date_time, &mut t.last_modified_date_time] {
        if !value.is_empty() {
            *value = date(value)?.to_rfc3339_opts(SecondsFormat::Millis, true);
        }
    }
    if !t.recurrence.is_empty() && t.due_date_time.is_none() {
        return Err("INVALID_ARGUMENT: 重复待办需要到期日期".into());
    }
    if t.is_reminder_on && t.reminder_date_time.is_none() {
        return Err("INVALID_ARGUMENT: 开启提醒需要提醒时间".into());
    }
    Ok(())
}
fn access(db: &Connection, id: &str) -> AppResult<()> {
    if !crate::shared_library::native_active(db, id, CAPABILITY)? {
        return Err("CAPABILITY_DENIED: 待办数据源未启用、已暂停或已移除".into());
    }
    Ok(())
}
fn load(db: &Connection, id: &str) -> AppResult<Document> {
    let raw: Option<String> = db
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [key(id)],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    let Some(raw) = raw else {
        return Ok(Document::default());
    };
    if raw.len() > BYTE_LIMIT {
        return Err("待办数据超过4 MiB，未修改原数据".into());
    }
    let mut d: Document = serde_json::from_str(&raw).map_err(err)?;
    if d.version != 1
        || d.todos.len() > LIMIT
        || d.receipts.len() > 128
        || d.reminders.len() > LIMIT
    {
        return Err("待办数据版本或数量无效，未修改原数据".into());
    }
    let mut ids = HashSet::new();
    for t in &mut d.todos {
        validate(t)?;
        if !ids.insert(t.id.clone()) {
            return Err("待办标识重复，未修改原数据".into());
        }
    }
    Ok(d)
}
fn persist(db: &Connection, id: &str, d: &Document) -> AppResult<()> {
    let raw = serde_json::to_string(d).map_err(err)?;
    if raw.len() > BYTE_LIMIT {
        return Err("INVALID_ARGUMENT: 待办数据超过4 MiB，未保存".into());
    }
    db.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE app_settings.value<>excluded.value",params![key(id),raw]).map_err(err)?;
    crate::shared_library::persist_native_revision(db, id, d.revision)?;
    Ok(())
}
pub(crate) fn pause_source(db: &Connection, id: &str) -> AppResult<()> {
    let mut d = load(db, id)?;
    for reminder in &mut d.reminders {
        reminder.nonce = Uuid::new_v4().to_string();
        if reminder.delivery == "sending" {
            reminder.delivery = "unknown".into();
            reminder.error = Some("切换前提醒递送结果未确定，未自动重发".into());
        }
    }
    d.revision = d.revision.checked_add(1).ok_or("待办版本超出范围")?;
    persist(db, id, &d)
}
pub(crate) fn copy_source(db: &Connection, source: &str, target: &str) -> AppResult<()> {
    let mut d = load(db, source)?;
    d.revision = 1;
    d.receipts.clear();
    for reminder in &mut d.reminders {
        reminder.nonce = Uuid::new_v4().to_string();
        if reminder.delivery == "sending" {
            reminder.delivery = "unknown".into();
            reminder.error = Some("复制前提醒递送结果未确定，未自动重发".into());
        }
    }
    persist(db, target, &d)
}
fn snapshot(d: &Document, now: i64) -> Value {
    let mut todos = d.todos.clone();
    todos.sort_by(
        |a, b| match (&a.completed_date_time, &b.completed_date_time) {
            (Some(a), Some(b)) => b.cmp(a),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => {
                a.order
                    .cmp(&b.order)
                    .then_with(|| match (&a.due_date_time, &b.due_date_time) {
                        (Some(a), Some(b)) => a.cmp(b),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => b.created_date_time.cmp(&a.created_date_time),
                    })
            }
        },
    );
    json!({"version":d.version,"revision":d.revision,"todos":todos,"sampledAt":now,"reminders":d.reminders.iter().map(|r|json!({"todoId":r.todo_id,"at":r.at,"delivery":r.delivery,"error":r.error})).collect::<Vec<_>>()})
}
fn read(store: &Store, id: &str, now: i64) -> AppResult<Value> {
    let db = store.db.lock().map_err(err)?;
    access(&db, id)?;
    Ok(snapshot(&load(&db, id)?, now))
}
fn synchronize(d: &mut Document) -> AppResult<()> {
    let active = d
        .todos
        .iter()
        .filter(|t| t.completed_date_time.is_none() && t.is_reminder_on)
        .map(|t| {
            Ok((
                t.id.clone(),
                date(t.reminder_date_time.as_deref().ok_or("缺少提醒时间")?)?.timestamp_millis(),
                signature(t),
            ))
        })
        .collect::<AppResult<Vec<_>>>()?;
    d.reminders.retain(|r| {
        active
            .iter()
            .any(|(id, at, s)| r.todo_id == *id && r.at == *at && r.signature == *s)
    });
    for (id, at, signature) in active {
        if !d.reminders.iter().any(|r| r.todo_id == id) {
            d.reminders.push(Reminder {
                todo_id: id,
                at,
                signature,
                nonce: Uuid::new_v4().to_string(),
                delivery: "waiting".into(),
                error: None,
                expires_at: at.checked_add(REMINDER_WINDOW).ok_or("提醒日期超出范围")?,
            });
        }
    }
    Ok(())
}
fn next_due(due: &str, rule: &str, now: i64) -> AppResult<String> {
    let anchor = date(due)?.with_timezone(&Local);
    let after = now.max(anchor.timestamp_millis());
    let current = Local
        .timestamp_millis_opt(after)
        .single()
        .ok_or("无法转换当前本地日期")?;
    let day_step = match rule {
        "RRULE:FREQ=DAILY" => Some(1),
        "RRULE:FREQ=WEEKLY" => Some(7),
        _ => None,
    };
    let base = if let Some(step) = day_step {
        (current.date_naive() - anchor.date_naive())
            .num_days()
            .max(0)
            / step
    } else {
        let m = i64::from(current.year() - anchor.year()) * 12 + i64::from(current.month())
            - i64::from(anchor.month());
        if rule == "RRULE:FREQ=YEARLY" {
            m.max(0) / 12
        } else if rule == "RRULE:FREQ=MONTHLY" {
            m.max(0)
        } else {
            return Err("不支持的重复规则".into());
        }
    };
    for offset in 0..16 {
        let index = base + offset;
        let next = if let Some(step) = day_step {
            anchor
                .date_naive()
                .checked_add_days(Days::new((index * step) as u64))
        } else {
            let months = i64::from(anchor.year()) * 12
                + i64::from(anchor.month0())
                + index * if rule == "RRULE:FREQ=YEARLY" { 12 } else { 1 };
            let year = i32::try_from(months.div_euclid(12)).map_err(err)?;
            if !(1..=9999).contains(&year) {
                return Err("下一次重复日期超出范围".into());
            }
            NaiveDate::from_ymd_opt(year, months.rem_euclid(12) as u32 + 1, anchor.day())
        };
        if let Some(next) = next {
            let local = Local
                .from_local_datetime(&next.and_time(anchor.time()))
                .single()
                .ok_or("下一次重复时间在系统时区中不存在或不唯一，未完成原事项")?;
            if local.timestamp_millis() > after {
                return Ok(local
                    .with_timezone(&Utc)
                    .to_rfc3339_opts(SecondsFormat::Millis, true));
            }
        }
    }
    Err("无法确定下一次重复日期，未完成原事项".into())
}
fn complete(d: &mut Document, id: &str, now: i64) -> AppResult<()> {
    let index = d
        .todos
        .iter()
        .position(|t| t.id == id)
        .ok_or("INVALID_ARGUMENT: 待办不存在")?;
    if d.todos[index].completed_date_time.is_some() {
        return Ok(());
    }
    let original = d.todos[index].clone();
    let mut next = None;
    if !original.recurrence.is_empty() {
        if d.todos.len() >= LIMIT {
            return Err("INVALID_ARGUMENT: 待办数量已达512项，未完成或生成下一次事项".into());
        }
        let old_due = original
            .due_date_time
            .as_deref()
            .ok_or("重复事项缺少日期")?;
        let due = next_due(old_due, &original.recurrence, now)?;
        let mut t = original.clone();
        t.id = Uuid::new_v4().to_string();
        t.due_date_time = Some(due.clone());
        t.start_date_time = Some(due.clone());
        t.created_date_time = iso(now)?;
        t.last_modified_date_time = iso(now)?;
        if t.is_reminder_on {
            let offset = date(old_due)?
                .timestamp_millis()
                .checked_sub(
                    date(t.reminder_date_time.as_deref().ok_or("缺少提醒时间")?)?
                        .timestamp_millis(),
                )
                .ok_or("提醒间隔超出范围")?;
            let at = date(&due)?
                .timestamp_millis()
                .checked_sub(offset)
                .ok_or("提醒时间超出范围")?;
            t.reminder_date_time = Some(iso(at)?);
            if at <= now {
                t.is_reminder_on = false;
            }
        }
        next = Some(t);
    }
    d.todos[index].completed_date_time = Some(iso(now)?);
    d.todos[index].last_modified_date_time = iso(now)?;
    if let Some(next) = next {
        d.todos.push(next)
    };
    synchronize(d)
}
fn change(store: &Store, id: &str, args: Value, now: i64) -> AppResult<Value> {
    let request: Change = serde_json::from_value(args.clone()).map_err(err)?;
    Uuid::parse_str(&request.request_id).map_err(err)?;
    let fingerprint = hash(&args);
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    access(&tx, id)?;
    let mut d = load(&tx, id)?;
    if let Some(receipt) = d.receipts.iter().find(|r| r.id == request.request_id) {
        return if receipt.fingerprint == fingerprint {
            Ok(snapshot(&d, now))
        } else {
            Err("INVALID_ARGUMENT: 请求标识已用于其他待办操作".into())
        };
    }
    if d.revision != request.expected_revision {
        return Err("REVISION_CONFLICT: 待办已更新，请重新读取后再保存".into());
    }
    match request.operation.as_str() {
        "save" if request.id.is_none() && request.ids.is_none() => {
            let mut t = request.todo.ok_or("缺少待办内容")?;
            validate(&mut t)?;
            if t.completed_date_time.is_some() {
                return Err("INVALID_ARGUMENT: 请使用完成操作更新历史".into());
            }
            if t.is_reminder_on
                && date(t.reminder_date_time.as_deref().ok_or("缺少提醒时间")?)?.timestamp_millis()
                    <= now
            {
                return Err("INVALID_ARGUMENT: 提醒时间必须晚于当前时间".into());
            }
            t.last_modified_date_time = iso(now)?;
            if let Some(index) = d.todos.iter().position(|old| old.id == t.id) {
                if d.todos[index].completed_date_time.is_some() {
                    return Err("INVALID_ARGUMENT: 已完成待办请先恢复再编辑".into());
                }
                t.created_date_time = d.todos[index].created_date_time.clone();
                d.todos[index] = t;
            } else {
                if d.todos.len() >= LIMIT {
                    return Err("INVALID_ARGUMENT: 待办数量最多512项".into());
                }
                t.created_date_time = iso(now)?;
                d.todos.push(t);
            }
        }
        "complete" | "reopen" | "delete" if request.todo.is_none() && request.ids.is_none() => {
            let target = request.id.as_deref().ok_or("缺少待办标识")?;
            if request.operation == "complete" {
                // 此阶段只修改内存，尚未持久化；日期等拒绝不能冒充提交结果未知。
                complete(&mut d, target, now).map_err(|error| {
                    if error.starts_with("INVALID_ARGUMENT:") {
                        error
                    } else {
                        format!("INVALID_ARGUMENT: {error}")
                    }
                })?;
            } else if request.operation == "delete" {
                let index = d
                    .todos
                    .iter()
                    .position(|t| t.id == target)
                    .ok_or("INVALID_ARGUMENT: 待办不存在")?;
                d.todos.remove(index);
            } else {
                let t = d
                    .todos
                    .iter_mut()
                    .find(|t| t.id == target)
                    .ok_or("INVALID_ARGUMENT: 待办不存在")?;
                t.completed_date_time = None;
                t.last_modified_date_time = iso(now)?;
            }
        }
        "reorder" if request.todo.is_none() && request.id.is_none() => {
            let ids = request.ids.ok_or("缺少排序列表")?;
            let active: HashSet<_> = d
                .todos
                .iter()
                .filter(|t| t.completed_date_time.is_none())
                .map(|t| t.id.as_str())
                .collect();
            let given: HashSet<_> = ids.iter().map(String::as_str).collect();
            if ids.len() != active.len() || given.len() != ids.len() || given != active {
                return Err("INVALID_ARGUMENT: 排序必须且只能包含全部未完成待办".into());
            }
            for (index, id) in ids.iter().enumerate() {
                let t = d
                    .todos
                    .iter_mut()
                    .find(|t| t.id == *id)
                    .ok_or("INVALID_ARGUMENT: 待办不存在")?;
                t.order = index as u32;
                t.last_modified_date_time = iso(now)?;
            }
        }
        _ => return Err("INVALID_ARGUMENT: 待办操作或参数无效".into()),
    }
    synchronize(&mut d)?;
    d.revision = d.revision.checked_add(1).ok_or("待办版本号超出范围")?;
    d.receipts.push(Receipt {
        id: request.request_id,
        fingerprint,
    });
    if d.receipts.len() > 128 {
        d.receipts.remove(0);
    }
    persist(&tx, id, &d)?;
    tx.commit().map_err(err)?;
    Ok(snapshot(&d, now))
}
fn reserve(store: &Store, id: &str, now: i64) -> AppResult<Vec<Pending>> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    if !crate::shared_library::native_active(&tx, id, CAPABILITY)? {
        return Ok(Vec::new());
    }
    let mut d = load(&tx, id)?;
    let mut result = Vec::new();
    for r in &mut d.reminders {
        if r.delivery == "sending" {
            r.delivery = "unknown".into();
            r.error = Some("上次提醒递送结果未确定，未自动重发".into());
        }
        if r.delivery != "waiting" {
            continue;
        }
        if now >= r.expires_at {
            r.delivery = "expired".into();
            continue;
        }
        if now >= r.at {
            if let Some(t) = d.todos.iter().find(|t| {
                t.id == r.todo_id
                    && t.is_reminder_on
                    && t.completed_date_time.is_none()
                    && signature(t) == r.signature
            }) {
                r.delivery = "sending".into();
                result.push(Pending {
                    todo_id: t.id.clone(),
                    title: t.title.clone(),
                    nonce: r.nonce.clone(),
                    signature: r.signature.clone(),
                    at: r.at,
                    expires_at: r.expires_at,
                });
            }
        }
    }
    if !d.reminders.is_empty() {
        persist(&tx, id, &d)?;
    }
    tx.commit().map_err(err)?;
    Ok(result)
}
fn current(d: &Document, p: &Pending, now: i64) -> bool {
    now >= p.at
        && now < p.expires_at
        && d.reminders.iter().any(|r| {
            r.todo_id == p.todo_id
                && r.nonce == p.nonce
                && r.signature == p.signature
                && r.at == p.at
                && matches!(r.delivery.as_str(), "sending" | "accepted")
        })
        && d.todos.iter().any(|t| {
            t.id == p.todo_id
                && t.is_reminder_on
                && t.completed_date_time.is_none()
                && signature(t) == p.signature
        })
}
#[cfg(test)]
fn tick(
    store: &Store,
    id: &str,
    now: i64,
    send: &mut impl FnMut(Pending) -> AppResult<()>,
) -> AppResult<()> {
    tick_with_clock(store, id, &mut || Ok(now), send)
}
fn tick_with_clock(
    store: &Store,
    id: &str,
    clock: &mut impl FnMut() -> AppResult<i64>,
    send: &mut impl FnMut(Pending) -> AppResult<()>,
) -> AppResult<()> {
    let _native = store.native_actions.lock().map_err(err)?;
    for p in reserve(store, id, clock()?)? {
        let now = clock()?;
        {
            let mut db = store.db.lock().map_err(err)?;
            let tx = db.transaction().map_err(err)?;
            if !crate::shared_library::native_active(&tx, id, CAPABILITY)? {
                return Ok(());
            }
            access(&tx, id)?;
            let mut d = load(&tx, id)?;
            if now < p.at {
                if let Some(r) = d
                    .reminders
                    .iter_mut()
                    .find(|r| r.nonce == p.nonce && r.delivery == "sending")
                {
                    r.delivery = "waiting".into();
                    persist(&tx, id, &d)?;
                }
                tx.commit().map_err(err)?;
                continue;
            }
            if now >= p.expires_at {
                if let Some(r) = d
                    .reminders
                    .iter_mut()
                    .find(|r| r.nonce == p.nonce && r.delivery == "sending")
                {
                    r.delivery = "expired".into();
                    persist(&tx, id, &d)?;
                }
                tx.commit().map_err(err)?;
                continue;
            }
            if !current(&d, &p, now) {
                continue;
            }
            tx.commit().map_err(err)?;
        }
        let result = send(p.clone());
        let mut db = store.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let mut d = load(&tx, id)?;
        if let Some(r) = d
            .reminders
            .iter_mut()
            .find(|r| r.todo_id == p.todo_id && r.nonce == p.nonce && r.delivery == "sending")
        {
            r.delivery = if result.is_ok() { "accepted" } else { "failed" }.into();
            r.error = result.err();
            persist(&tx, id, &d)?;
        }
        tx.commit().map_err(err)?;
    }
    Ok(())
}
fn acknowledge(store: &Store, id: &str, p: &Pending, finish: bool, now: i64) -> AppResult<bool> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    if !crate::shared_library::native_active(&tx, id, CAPABILITY)? {
        return Ok(false);
    }
    let mut d = load(&tx, id)?;
    if !current(&d, p, now) {
        return Ok(false);
    }
    if finish {
        complete(&mut d, &p.todo_id, now)?;
        d.revision = d.revision.checked_add(1).ok_or("待办版本超出范围")?;
    } else if let Some(r) = d.reminders.iter_mut().find(|r| r.nonce == p.nonce) {
        r.delivery = "dismissed".into();
    }
    persist(&tx, id, &d)?;
    tx.commit().map_err(err)?;
    Ok(true)
}
pub fn remove_data(db: &Connection, id: &str) -> AppResult<()> {
    db.execute("DELETE FROM app_settings WHERE key=?1", [key(id)])
        .map_err(err)?;
    Ok(())
}
pub(crate) fn authorized(
    store: &Store,
    sessions: &widgets::Sessions,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
    now: i64,
) -> AppResult<Value> {
    let _native = store.native_actions.lock().map_err(err)?;
    widgets::with_capability(store, sessions, window, token, CAPABILITY, |session| {
        let id = {
            let db = store.db.lock().map_err(err)?;
            crate::shared_library::native_source(&db, &session.instance, CAPABILITY)?
        };
        let value = match method {
            "todo.get" if args.is_null() || args.as_object().is_some_and(|o| o.is_empty()) => {
                read(store, &id, now)
            }
            "todo.change" => change(store, &id, args, now),
            _ => Err("INVALID_ARGUMENT: 待办接口或参数无效".into()),
        }?;
        crate::shared_library::native_cursor(session, &id, &value);
        Ok(value)
    })
}
pub async fn gateway(
    app: &AppHandle,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    let handle = app.clone();
    let (window, token, method) = (window.to_owned(), token.to_owned(), method.to_owned());
    tauri::async_runtime::spawn_blocking(move || {
        let core = handle.state::<Core>();
        authorized(
            &core.store,
            &core.sessions,
            &window,
            &token,
            &method,
            args,
            i64::try_from(now_ms()).map_err(err)?,
        )
    })
    .await
    .map_err(err)?
}
impl Service {
    pub fn dispatch(&self, app: &AppHandle) {
        if self.busy.swap(true, Ordering::AcqRel) {
            return;
        }
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            struct Release(AppHandle);
            impl Drop for Release {
                fn drop(&mut self) {
                    self.0
                        .state::<Core>()
                        .todo
                        .busy
                        .store(false, Ordering::Release);
                }
            }
            let _release = Release(handle.clone());
            let core = handle.state::<Core>();
            let result = (|| -> AppResult<()> {
                let ids = {
                    let db = core.store.db.lock().map_err(err)?;
                    crate::shared_library::native_sources(&db, CAPABILITY)?
                };
                for id in ids {
                    if core.quitting.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Err(error) = tick_with_clock(
                        &core.store,
                        &id,
                        &mut || i64::try_from(now_ms()).map_err(err),
                        &mut |p| notify(&handle, &id, &p),
                    ) {
                        core.report(format!("待办提醒：{error}"));
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                core.report(format!("待办提醒：{error}"));
            }
        });
    }
}
#[cfg(windows)]
fn notify(app: &AppHandle, id: &str, pending: &Pending) -> AppResult<()> {
    use tauri_winrt_notification::{Duration, Toast};
    let core = app.state::<Core>();
    let store = std::sync::Arc::downgrade(&core.store);
    let last_error = std::sync::Arc::downgrade(&core.last_error);
    let id = id.to_owned();
    let pending = pending.clone();
    Toast::new(&app.config().identifier)
        .title("待办事项提醒")
        .text1(&pending.title)
        .duration(Duration::Long)
        .add_button("知道了", "close")
        .add_button("已完成", "complete")
        .on_activated(move |action| {
            if let (Some(store), Some(action)) = (store.upgrade(), action) {
                if ["close", "complete"].contains(&action.as_str()) {
                    if let Err(error) = i64::try_from(now_ms())
                        .map_err(err)
                        .and_then(|at| acknowledge(&store, &id, &pending, action == "complete", at))
                    {
                        if let Some(slot) = last_error.upgrade() {
                            if let Ok(mut v) = slot.lock() {
                                *v = Some(format!("待办提醒回执失败：{error}"));
                            }
                        }
                    }
                }
            }
            Ok(())
        })
        .show()
        .map_err(err)
}
#[cfg(not(windows))]
fn notify(_app: &AppHandle, _id: &str, _pending: &Pending) -> AppResult<()> {
    Err("当前平台未实现待办系统通知".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_todo_pause_invalidates_old_notification_nonce_and_copy_does_not_resend() {
        let (s, a, b) = fixture();
        let mut todo = item("待提醒事项");
        todo["isReminderOn"] = json!(true);
        todo["reminderDateTime"] = json!(iso(now() + 10000).unwrap());
        save(&s, &a, todo);
        let pending = reserve(&s, &a, now() + 10000).unwrap().remove(0);
        {
            let db = s.db.lock().unwrap();
            pause_source(&db, &a).unwrap();
            copy_source(&db, &a, &b).unwrap();
        }
        assert!(!acknowledge(&s, &a, &pending, true, now() + 10001).unwrap());
        tick(&s, &b, now() + 10001, &mut |_| {
            panic!("不得重发复制前不确定的提醒")
        })
        .unwrap();
        assert_eq!(
            read(&s, &b, now()).unwrap()["reminders"][0]["delivery"],
            "unknown"
        );
    }
    #[test]
    fn native_todo_delivery_serializes_source_switch_without_holding_sqlite() {
        let (s, a, _) = fixture();
        let mut todo = item("通知临界区");
        todo["isReminderOn"] = json!(true);
        todo["reminderDateTime"] = json!(iso(now() + 10000).unwrap());
        save(&s, &a, todo);
        let mut sent = 0;
        tick(&s, &a, now() + 10000, &mut |_| {
            assert!(s.native_actions.try_lock().is_err());
            assert!(s.db.try_lock().is_ok(), "递送期间不得持SQLite锁");
            sent += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(sent, 1);
    }
    use serde_json::{json, Value};
    use uuid::Uuid;
    fn fixture() -> (crate::store::Store, String, String) {
        let store = crate::store::Store::open(
            std::env::temp_dir().join(format!("souldesk-todo-{}", Uuid::new_v4())),
        )
        .unwrap();
        let manifest = json!({"schemaVersion":1,"id":"local.test.todo","name":"待办检查","version":"0.1.0","hostApiMajor":1,"entry":"dist/index.html","defaultSize":{"width":400,"height":400},"minSize":{"width":240,"height":200},"capabilities":["todos.manage"],"settingsSchema":{"type":"object","additionalProperties":false,"properties":{}}});
        store.db.lock().unwrap().execute("INSERT INTO widget_packages(id,manifest,directory,digest) VALUES(?1,?2,'test-todo','test-todo')",rusqlite::params!["local.test.todo",manifest.to_string()]).unwrap();
        let a = crate::widgets::create_instance(&store, "local.test.todo").unwrap();
        let b = crate::widgets::create_instance(&store, "local.test.todo").unwrap();
        (store, a, b)
    }
    fn now() -> i64 {
        chrono::DateTime::parse_from_rfc3339("2026-11-15T00:00:00Z")
            .unwrap()
            .timestamp_millis()
    }
    fn item(title: &str) -> Value {
        json!({"id":Uuid::new_v4().to_string(),"title":title,"isReminderOn":false,"recurrence":"","importance":"normal","order":0})
    }
    fn request(revision: u64, operation: &str) -> Value {
        json!({"requestId":Uuid::new_v4().to_string(),"expectedRevision":revision,"operation":operation})
    }
    fn save(store: &crate::store::Store, id: &str, todo: Value) -> Value {
        let mut r = request(
            read(store, id, now()).unwrap()["revision"]
                .as_u64()
                .unwrap(),
            "save",
        );
        r["todo"] = todo;
        change(store, id, r, now()).unwrap()
    }
    #[test]
    fn todo_changes_are_revisioned_retry_safe_and_isolated() {
        let (s, a, b) = fixture();
        let t = item("检查中文待办");
        let mut r = request(0, "save");
        r["todo"] = t;
        let first = change(&s, &a, r.clone(), now()).unwrap();
        assert_eq!(first["revision"], 1);
        assert_eq!(first["todos"].as_array().unwrap().len(), 1);
        assert_eq!(change(&s, &a, r.clone(), now()).unwrap()["revision"], 1);
        r["todo"]["title"] = json!("不同重试");
        assert!(change(&s, &a, r, now()).is_err());
        assert!(change(&s, &a, request(0, "delete"), now()).is_err());
        assert!(read(&s, &b, now()).unwrap()["todos"]
            .as_array()
            .unwrap()
            .is_empty());
        let root = s.root.clone();
        drop(s);
        assert_eq!(
            read(&crate::store::Store::open(root).unwrap(), &a, now()).unwrap()["todos"][0]
                ["title"],
            "检查中文待办"
        );
    }
    #[test]
    fn invalid_items_and_unknown_arguments_leave_saved_document_unchanged() {
        let (s, a, _) = fixture();
        let valid = item("保留");
        save(&s, &a, valid.clone());
        let before = read(&s, &a, now()).unwrap();
        for (key, value) in [
            ("title", json!(" ")),
            ("title", json!("字".repeat(4097))),
            ("id", json!("not-uuid")),
            ("dueDateTime", json!("2026-02-30T00:00:00Z")),
            ("recurrence", json!("RRULE:FREQ=SECONDLY")),
            ("isReminderOn", json!(true)),
            ("reminderDateTime", json!("bad")),
        ] {
            let mut t = valid.clone();
            t[key] = value;
            let mut r = request(1, "save");
            r["todo"] = t;
            assert!(change(&s, &a, r, now()).is_err(), "{key}");
        }
        let mut r = request(1, "save");
        r["todo"] = valid;
        r["instance"] = json!("other");
        assert!(change(&s, &a, r, now()).is_err());
        assert_eq!(read(&s, &a, now()).unwrap(), before);
    }
    #[test]
    fn completion_history_reopen_delete_and_reorder_are_transactional() {
        let (s, a, _) = fixture();
        let one = item("第一项");
        let two = item("第二项");
        let id = one["id"].clone();
        save(&s, &a, one);
        save(&s, &a, two.clone());
        let mut r = request(2, "reorder");
        r["ids"] = json!([two["id"], id]);
        let d = change(&s, &a, r, now()).unwrap();
        assert_eq!(d["todos"][0]["id"], two["id"]);
        let mut r = request(3, "complete");
        r["id"] = id.clone();
        let d = change(&s, &a, r, now()).unwrap();
        assert!(d["todos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == id)
            .unwrap()["completedDateTime"]
            .is_string());
        let mut r = request(4, "reopen");
        r["id"] = id.clone();
        let d = change(&s, &a, r, now()).unwrap();
        assert!(d["todos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == id)
            .unwrap()["completedDateTime"]
            .is_null());
        let mut r = request(5, "delete");
        r["id"] = id;
        let d = change(&s, &a, r, now()).unwrap();
        assert_eq!(d["todos"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn completing_a_repeat_creates_only_one_next_item_and_shifts_reminder() {
        let (s, a, _) = fixture();
        let mut t = item("重复");
        t["dueDateTime"] = json!("2026-11-15T12:00:00Z");
        t["reminderDateTime"] = json!("2026-11-15T11:00:00Z");
        t["isReminderOn"] = json!(true);
        t["recurrence"] = json!("RRULE:FREQ=DAILY");
        let id = t["id"].clone();
        save(&s, &a, t);
        let mut r = request(1, "complete");
        r["id"] = id.clone();
        let d = change(&s, &a, r.clone(), now()).unwrap();
        assert_eq!(d["todos"].as_array().unwrap().len(), 2);
        assert_eq!(change(&s, &a, r, now()).unwrap()["revision"], 2);
        let next = d["todos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] != id)
            .unwrap();
        assert_eq!(next["dueDateTime"], "2026-11-16T12:00:00.000Z");
        assert_eq!(next["reminderDateTime"], "2026-11-16T11:00:00.000Z");
        assert!(next["completedDateTime"].is_null());
    }
    #[test]
    fn monthly_repeats_skip_absent_days_and_daily_repeats_keep_local_date() {
        use chrono::{TimeZone, Timelike};
        let start = chrono::Local
            .with_ymd_and_hms(2026, 1, 31, 9, 40, 0)
            .single()
            .unwrap();
        let next = next_due(
            &start.to_rfc3339(),
            "RRULE:FREQ=MONTHLY",
            start.timestamp_millis(),
        )
        .unwrap();
        let local = chrono::DateTime::parse_from_rfc3339(&next)
            .unwrap()
            .with_timezone(&chrono::Local);
        assert_eq!(
            local.format("%Y-%m-%d %H:%M").to_string(),
            "2026-03-31 09:40"
        );
        let daily = next_due(
            &start.to_rfc3339(),
            "RRULE:FREQ=DAILY",
            start.timestamp_millis(),
        )
        .unwrap();
        let local = chrono::DateTime::parse_from_rfc3339(&daily)
            .unwrap()
            .with_timezone(&chrono::Local);
        assert_eq!(local.hour(), 9);
        assert_eq!(local.minute(), 40);
        assert_eq!(local.format("%Y-%m-%d").to_string(), "2026-02-01");
    }
    #[test]
    fn reminders_fire_with_no_sessions_and_are_not_repeated() {
        let (s, a, b) = fixture();
        let mut t = item("后台事项");
        t["isReminderOn"] = json!(true);
        t["reminderDateTime"] = json!("2026-11-15T00:00:10Z");
        save(&s, &a, t);
        let mut sent = Vec::new();
        tick(&s, &a, now() + 9999, &mut |p| {
            sent.push(p);
            Ok(())
        })
        .unwrap();
        assert!(sent.is_empty());
        tick(&s, &a, now() + 10000, &mut |p| {
            sent.push(p);
            Ok(())
        })
        .unwrap();
        tick(&s, &a, now() + 11000, &mut |p| {
            sent.push(p);
            Ok(())
        })
        .unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].title, "后台事项");
        assert_eq!(
            read(&s, &a, now() + 11000).unwrap()["reminders"][0]["delivery"],
            "accepted"
        );
        assert!(read(&s, &b, now()).unwrap()["todos"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn notification_failure_and_interrupted_delivery_are_explicit() {
        let (s, a, _) = fixture();
        let mut t = item("递送失败");
        t["isReminderOn"] = json!(true);
        t["reminderDateTime"] = json!("2026-11-15T00:00:10Z");
        save(&s, &a, t);
        tick(&s, &a, now() + 10000, &mut |_| Err("系统拒绝".into())).unwrap();
        let d = read(&s, &a, now() + 10000).unwrap();
        assert_eq!(d["reminders"][0]["delivery"], "failed");
        assert_eq!(d["reminders"][0]["error"], "系统拒绝");
        let (s, a, _) = fixture();
        let mut t = item("进程中断");
        t["isReminderOn"] = json!(true);
        t["reminderDateTime"] = json!("2026-11-15T00:00:10Z");
        save(&s, &a, t);
        assert_eq!(reserve(&s, &a, now() + 10000).unwrap().len(), 1);
        tick(&s, &a, now() + 10001, &mut |_| panic!("不得重发不确定通知")).unwrap();
        assert_eq!(
            read(&s, &a, now()).unwrap()["reminders"][0]["delivery"],
            "unknown"
        );
    }
    #[test]
    fn stale_or_replayed_notification_buttons_cannot_complete_other_items() {
        let (s, a, b) = fixture();
        let mut t = item("通知按钮");
        t["isReminderOn"] = json!(true);
        t["reminderDateTime"] = json!("2026-11-15T00:00:10Z");
        save(&s, &a, t.clone());
        let p = reserve(&s, &a, now() + 10000).unwrap().remove(0);
        assert!(!acknowledge(&s, &b, &p, true, now() + 10001).unwrap());
        let mut edited = t.clone();
        edited["title"] = json!("已经编辑");
        save(&s, &a, edited);
        assert!(!acknowledge(&s, &a, &p, true, now() + 10002).unwrap());
        let mut t = item("可完成");
        t["isReminderOn"] = json!(true);
        t["reminderDateTime"] = json!("2026-11-15T00:00:10Z");
        let id = t["id"].clone();
        save(&s, &a, t);
        let p = reserve(&s, &a, now() + 10000)
            .unwrap()
            .into_iter()
            .find(|p| json!(p.todo_id) == id)
            .unwrap();
        assert!(acknowledge(&s, &a, &p, true, now() + 10001).unwrap());
        assert!(!acknowledge(&s, &a, &p, true, now() + 10002).unwrap());
        assert!(read(&s, &a, now()).unwrap()["todos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == id)
            .unwrap()["completedDateTime"]
            .is_string());
    }
    #[test]
    fn late_reminders_expire_and_removing_instance_data_cancels_buttons() {
        let (s, a, _) = fixture();
        let mut t = item("过时提醒");
        t["isReminderOn"] = json!(true);
        t["reminderDateTime"] = json!("2026-11-15T00:00:10Z");
        save(&s, &a, t);
        tick(&s, &a, now() + 40001, &mut |_| panic!("不得补发过时提醒")).unwrap();
        assert_eq!(
            read(&s, &a, now()).unwrap()["reminders"][0]["delivery"],
            "expired"
        );
        remove_data(&s.db.lock().unwrap(), &a).unwrap();
        assert!(read(&s, &a, now()).unwrap()["todos"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn notification_batch_rechecks_expiration_before_each_send() {
        let (s, a, _) = fixture();
        for title in ["先发", "后发"] {
            let mut t = item(title);
            t["isReminderOn"] = json!(true);
            t["reminderDateTime"] = json!("2026-11-15T00:00:10Z");
            save(&s, &a, t);
        }
        let mut times =
            std::collections::VecDeque::from([now() + 10000, now() + 10000, now() + 40000]);
        let mut sent = Vec::new();
        tick_with_clock(
            &s,
            &a,
            &mut || Ok(times.pop_front().unwrap_or(now() + 40000)),
            &mut |p| {
                sent.push(p);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(sent.len(), 1, "批量通知必须在每次发送前重新检查有效期");
        let d = read(&s, &a, now() + 40000).unwrap();
        assert_eq!(d["reminders"][0]["delivery"], "accepted");
        assert_eq!(d["reminders"][1]["delivery"], "expired");
    }
    #[test]
    fn clock_rollback_before_send_keeps_the_future_reminder_schedulable() {
        let (s, a, _) = fixture();
        let mut t = item("时钟回拨");
        t["isReminderOn"] = json!(true);
        t["reminderDateTime"] = json!("2026-11-15T00:00:10Z");
        save(&s, &a, t);
        let mut times = std::collections::VecDeque::from([now() + 10000, now() + 5000]);
        tick_with_clock(
            &s,
            &a,
            &mut || Ok(times.pop_front().unwrap_or(now() + 5000)),
            &mut |_| panic!("回拨后尚未到期，不得发送"),
        )
        .unwrap();
        assert_eq!(
            read(&s, &a, now() + 5000).unwrap()["reminders"][0]["delivery"],
            "waiting"
        );
        let mut calls = 0;
        tick(&s, &a, now() + 10000, &mut |_| {
            calls += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(calls, 1);
    }
    #[test]
    fn storage_failures_roll_back_edits_and_prevent_unreserved_notifications() {
        let (s, a, _) = fixture();
        let mut t = item("保存失败保留");
        t["isReminderOn"] = json!(true);
        t["reminderDateTime"] = json!("2026-11-15T00:00:10Z");
        save(&s, &a, t.clone());
        let before = read(&s, &a, now()).unwrap();
        s.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_todo_update BEFORE UPDATE ON app_settings WHEN substr(new.key,1,14)='todo.document:' BEGIN SELECT RAISE(FAIL,'test storage refused'); END;").unwrap();
        t["title"] = json!("不得假保存");
        let mut r = request(1, "save");
        r["todo"] = t;
        assert!(change(&s, &a, r, now()).is_err());
        assert_eq!(read(&s, &a, now()).unwrap(), before);
        assert!(tick(&s, &a, now() + 10000, &mut |_| panic!(
            "预约未保存，不得递送通知"
        ))
        .is_err());
        assert_eq!(read(&s, &a, now()).unwrap(), before);
    }
    #[test]
    fn corrupt_or_future_documents_are_not_recreated_or_overwritten() {
        let (s, a, _) = fixture();
        let raw = "{\"version\":2,\"revision\":9,\"todos\":[],\"reminders\":[],\"receipts\":[]}";
        s.db.lock()
            .unwrap()
            .execute(
                "INSERT INTO app_settings(key,value)VALUES(?1,?2)",
                rusqlite::params![key(&a), raw],
            )
            .unwrap();
        assert!(read(&s, &a, now()).is_err());
        let mut r = request(9, "save");
        r["todo"] = item("不得覆盖未来版本");
        assert!(change(&s, &a, r, now()).is_err());
        assert_eq!(
            s.db.lock()
                .unwrap()
                .query_row(
                    "SELECT value FROM app_settings WHERE key=?1",
                    [key(&a)],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            raw
        );
    }
    #[test]
    fn definite_quota_and_stale_item_rejections_are_distinguishable_from_timeouts() {
        let (s, a, _) = fixture();
        let mut document = Document::default();
        for _ in 0..LIMIT {
            let mut t: Todo = serde_json::from_value(item("已有事项")).unwrap();
            t.created_date_time = iso(now()).unwrap();
            t.last_modified_date_time = iso(now()).unwrap();
            document.todos.push(t);
        }
        persist(&s.db.lock().unwrap(), &a, &document).unwrap();
        let mut r = request(0, "save");
        r["todo"] = item("超过限额");
        let error = change(&s, &a, r, now()).unwrap_err();
        assert!(error.starts_with("INVALID_ARGUMENT:"), "{error}");
        let mut r = request(0, "delete");
        r["id"] = json!(Uuid::new_v4().to_string());
        let error = change(&s, &a, r, now()).unwrap_err();
        assert!(error.starts_with("INVALID_ARGUMENT:"), "{error}");
        assert_eq!(read(&s, &a, now()).unwrap()["revision"], 0);
    }
    #[test]
    fn impossible_repeat_completion_is_a_definite_rejection_and_keeps_item_editable() {
        let (s, a, _) = fixture();
        let mut t = item("日期超界");
        t["dueDateTime"] = json!("9999-12-31T00:00:00Z");
        t["recurrence"] = json!("RRULE:FREQ=MONTHLY");
        let id = t["id"].clone();
        save(&s, &a, t);
        let mut r = request(1, "complete");
        r["id"] = id.clone();
        let error = change(&s, &a, r, now()).unwrap_err();
        assert!(error.starts_with("INVALID_ARGUMENT:"), "{error}");
        assert_eq!(read(&s, &a, now()).unwrap()["revision"], 1);
        let mut r = request(1, "delete");
        r["id"] = id;
        assert!(change(&s, &a, r, now()).unwrap()["todos"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn gateway_binds_current_capability_and_rejects_preview_and_other_windows() {
        let (s, a, _) = fixture();
        let sessions = crate::widgets::Sessions::default();
        sessions.0.lock().unwrap().insert(
            "formal".into(),
            crate::widgets::Session {
                instance: a.clone(),
                window: "float-a".into(),
                shared_cursor: None,
                preview: None,
            },
        );
        assert!(authorized(
            &s,
            &sessions,
            "other",
            "formal",
            "todo.get",
            Value::Null,
            now()
        )
        .is_err());
        assert!(authorized(
            &s,
            &sessions,
            "float-a",
            "formal",
            "todo.get",
            json!({"instance":"other"}),
            now()
        )
        .is_err());
        assert_eq!(
            authorized(
                &s,
                &sessions,
                "float-a",
                "formal",
                "todo.get",
                Value::Null,
                now()
            )
            .unwrap()["revision"],
            0
        );
        let staged = crate::widgets::stage_bytes(
            &s,
            crate::presets::find("local.fixture.surface")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        sessions.0.lock().unwrap().insert(
            "trial".into(),
            crate::widgets::Session {
                instance: a.clone(),
                window: "trial-todo".into(),
                shared_cursor: None,
                preview: Some(std::sync::Arc::new(std::sync::Mutex::new(
                    crate::widgets::Preview {
                        package: staged,
                        settings: json!({}),
                        storage: json!({}),
                        ready_since: None,
                        failure: None,
                    },
                ))),
            },
        );
        assert!(authorized(
            &s,
            &sessions,
            "trial-todo",
            "trial",
            "todo.get",
            Value::Null,
            now()
        )
        .is_err());
        s.db.lock().unwrap().execute("UPDATE widget_packages SET manifest=replace(manifest,'todos.manage','storage.instance')",[]).unwrap();
        assert!(authorized(
            &s,
            &sessions,
            "float-a",
            "formal",
            "todo.get",
            Value::Null,
            now()
        )
        .is_err());
    }
}
