//! 可选本地共享库；原云同步不由此模块替代。
use crate::{
    store::{err, now_ms, AppResult},
    widgets::{Manifest, Session},
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Mindmap,
    Countdown,
    Birthday,
    Todo,
    Pomodoro,
}
impl Kind {
    fn field(self) -> &'static str {
        match self {
            Self::Mindmap => "documents",
            Self::Countdown => "events",
            Self::Birthday => "birthdays",
            Self::Todo => "todos",
            Self::Pomodoro => "scenes",
        }
    }
    fn native(self) -> bool {
        matches!(self, Self::Todo | Self::Pomodoro)
    }
    fn limit(self) -> usize {
        if self == Self::Mindmap {
            32 * 1024 * 1024
        } else {
            4 * 1024 * 1024
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    version: u32,
    id: String,
    kind: Kind,
    name: String,
    revision: u64,
    updated_at: u64,
}
fn kind(manifest: &Manifest) -> AppResult<Kind> {
    if !manifest
        .capabilities
        .iter()
        .any(|cap| cap == "storage.shared")
    {
        return Err("CAPABILITY_DENIED: 未声明本地共享库能力".into());
    }
    match manifest.id.as_str() {
        "local.souldesk.widgetjs.mindmap" => Ok(Kind::Mindmap),
        "local.souldesk.widgetjs.countdown" | "local.souldesk.widgetjs.countdown-list" => {
            Ok(Kind::Countdown)
        }
        "local.souldesk.widgetjs.birthday-list" => Ok(Kind::Birthday),
        "local.souldesk.widgetjs.todo" | "local.souldesk.widgetjs.calendar-large"
            if manifest
                .capabilities
                .iter()
                .any(|cap| cap == "todos.manage") =>
        {
            Ok(Kind::Todo)
        }
        "local.souldesk.widgetjs.pomodoro"
            if manifest
                .capabilities
                .iter()
                .any(|cap| cap == "focus.pomodoro") =>
        {
            Ok(Kind::Pomodoro)
        }
        _ => Err("CAPABILITY_DENIED: 此组件尚未开放共享库数据类型".into()),
    }
}
fn valid_id(id: &str) -> AppResult<()> {
    if uuid::Uuid::parse_str(id).is_err() {
        Err("INVALID_ARGUMENT: 共享库标识无效".into())
    } else {
        Ok(())
    }
}
fn name(input: &str) -> AppResult<String> {
    let value = input.trim();
    if value.is_empty() || value.chars().count() > 80 || value.chars().any(char::is_control) {
        return Err("INVALID_ARGUMENT: 库名称须为1～80字且不能含控制字符".into());
    }
    Ok(value.into())
}
fn key(id: &str) -> String {
    format!("shared-library:{id}")
}
fn data_key(id: &str) -> String {
    format!("shared-library-data:{id}")
}
fn binding_key(instance: &str) -> String {
    format!("shared-binding:{instance}")
}
fn view_key(instance: &str, id: &str) -> String {
    format!("shared-view:{instance}:{id}")
}
fn read(db: &Connection, key: &str) -> AppResult<Option<String>> {
    db.query_row(
        "SELECT value FROM app_settings WHERE key=?1",
        [key],
        |row| row.get(0),
    )
    .optional()
    .map_err(err)
}
fn write(db: &Connection, key: &str, value: &str) -> AppResult<()> {
    db.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value]).map_err(err)?;
    Ok(())
}
fn record(db: &Connection, id: &str) -> AppResult<Record> {
    valid_id(id)?;
    let value: Record =
        serde_json::from_str(&read(db, &key(id))?.ok_or("DATA_UNAVAILABLE: 共享库不存在")?)
            .map_err(|_| "DATA_UNAVAILABLE: 共享库元数据损坏，请保留原数据")?;
    if value.version != 1
        || value.id != id
        || value.revision == 0
        || value.revision > 9_007_199_254_740_991
        || name(&value.name).is_err()
    {
        return Err("DATA_UNAVAILABLE: 共享库版本或元数据无效".into());
    }
    Ok(value)
}
fn validate_payload(kind: Kind, value: &Value) -> AppResult<()> {
    let rows = value[kind.field()]
        .as_array()
        .ok_or("INVALID_ARGUMENT: 共享内容类型无效")?;
    if value["version"] != 1 || rows.len() > 512 || value.to_string().len() > kind.limit() {
        return Err("LIMIT: 共享内容版本、条目数或大小超限".into());
    }
    let mut ids = std::collections::HashSet::new();
    for row in rows {
        let id = row["id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or("INVALID_ARGUMENT: 共享条目标识无效")?;
        if !ids.insert(id) {
            return Err("INVALID_ARGUMENT: 共享条目标识重复".into());
        }
    }
    Ok(())
}
fn split(kind: Kind, document: &Value) -> AppResult<(Value, Value)> {
    if !document.is_object() {
        return Err("INVALID_ARGUMENT: 组件文档无效".into());
    }
    let mut payload = json!({"version":1});
    payload[kind.field()] = if document.as_object().unwrap().is_empty() {
        json!([])
    } else {
        if document["version"] != 1 {
            return Err("INVALID_ARGUMENT: 组件文档版本无效".into());
        }
        document[kind.field()].clone()
    };
    validate_payload(kind, &payload)?;
    let view = match kind {
        Kind::Mindmap => {
            json!({"preferences":document.get("preferences").cloned().unwrap_or(json!({"theme":"system","showGrid":true,"showCheckboxes":true,"sidebarOpen":true}))})
        }
        Kind::Countdown => {
            json!({"selectedId":document.get("selectedId").and_then(Value::as_str).unwrap_or(""),"sort":if document["sort"]=="desc"{"desc"}else{"asc"}})
        }
        Kind::Birthday | Kind::Todo | Kind::Pomodoro => json!({}),
    };
    Ok((payload, view))
}
fn payload(db: &Connection, library: &Record) -> AppResult<Value> {
    let value = serde_json::from_str(
        &read(db, &data_key(&library.id))?.ok_or("DATA_UNAVAILABLE: 共享库内容缺失")?,
    )
    .map_err(|_| "DATA_UNAVAILABLE: 共享库内容损坏")?;
    validate_payload(library.kind, &value)?;
    Ok(value)
}
fn compose(kind: Kind, mut data: Value, view: &Value) -> Value {
    match kind {
        Kind::Mindmap => data["preferences"] = view["preferences"].clone(),
        Kind::Countdown => {
            let selected = view["selectedId"].as_str().unwrap_or("");
            let items = data["events"].as_array().unwrap();
            data["selectedId"] = json!(if items.iter().any(|row| row["id"] == selected) {
                selected.to_owned()
            } else {
                items
                    .first()
                    .and_then(|row| row["id"].as_str())
                    .unwrap_or("")
                    .to_owned()
            });
            data["sort"] = json!(if view["sort"] == "desc" {
                "desc"
            } else {
                "asc"
            });
        }
        Kind::Birthday | Kind::Todo | Kind::Pomodoro => {}
    }
    data
}
fn bound(db: &Connection, instance: &str, expected: Kind) -> AppResult<Option<Record>> {
    read(db, &binding_key(instance))?
        .map(|id| {
            let value = record(db, &id)?;
            if value.kind != expected {
                return Err("CAPABILITY_DENIED: 共享库类型不匹配".into());
            }
            Ok(value)
        })
        .transpose()
}
fn consumers(db: &Connection, id: &str) -> AppResult<u64> {
    db.query_row(
        "SELECT count(*) FROM app_settings WHERE key LIKE 'shared-binding:%' AND value=?1",
        [id],
        |row| row.get(0),
    )
    .map_err(err)
}
fn native_kind(capability: &str) -> AppResult<Kind> {
    match capability {
        "todos.manage" => Ok(Kind::Todo),
        "focus.pomodoro" => Ok(Kind::Pomodoro),
        _ => Err("CAPABILITY_DENIED: 不是已开放的原生共享类型".into()),
    }
}
fn native_owner(id: &str) -> String {
    format!("shared/{id}")
}
fn native_revision_key(id: &str) -> String {
    format!("shared-library-native-revision:{id}")
}
fn native_revision(db: &Connection, id: &str) -> AppResult<u64> {
    read(db, &native_revision_key(id))?
        .ok_or("DATA_UNAVAILABLE: 原生共享内容版本缺失")?
        .parse()
        .map_err(|_| "DATA_UNAVAILABLE: 原生共享内容版本无效".into())
}
pub(crate) fn persist_native_revision(
    db: &Connection,
    owner: &str,
    revision: u64,
) -> AppResult<()> {
    if let Some(id) = owner.strip_prefix("shared/") {
        valid_id(id)?;
        write(db, &native_revision_key(id), &revision.to_string())?;
    }
    Ok(())
}
fn instance_manifest(db: &Connection, instance: &str) -> AppResult<Manifest> {
    let raw:String=db.query_row("SELECT p.manifest FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE i.id=?1",[instance],|row|row.get(0)).map_err(err)?;
    serde_json::from_str(&raw).map_err(err)
}
pub(crate) fn native_source(
    db: &Connection,
    instance: &str,
    capability: &str,
) -> AppResult<String> {
    let manifest = instance_manifest(db, instance)?;
    if !manifest.capabilities.iter().any(|cap| cap == capability) {
        return Err("CAPABILITY_DENIED: 未声明原生业务能力".into());
    }
    let Some(id) = read(db, &binding_key(instance))? else {
        return Ok(instance.into());
    };
    let expected = native_kind(capability)?;
    if kind(&manifest)? != expected || record(db, &id)?.kind != expected {
        return Err("CAPABILITY_DENIED: 原生共享库类型不匹配".into());
    }
    Ok(native_owner(&id))
}
pub(crate) fn native_active(db: &Connection, owner: &str, capability: &str) -> AppResult<bool> {
    if let Some(id) = owner.strip_prefix("shared/") {
        if read(db, &key(id))?.is_none() {
            return Ok(false);
        }
        let expected = native_kind(capability)?;
        if record(db, id)?.kind != expected {
            return Ok(false);
        }
        let mut stmt=db.prepare("SELECT i.id FROM widget_instances i JOIN app_settings b ON b.key=('shared-binding:'||i.id) WHERE b.value=?1").map_err(err)?;
        let ids = stmt
            .query_map([id], |row| row.get::<_, String>(0))
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        for instance in ids {
            let manifest = instance_manifest(db, &instance)?;
            if manifest.capabilities.iter().any(|cap| cap == capability)
                && kind(&manifest).is_ok_and(|k| k == expected)
            {
                return Ok(true);
            }
        }
        Ok(false)
    } else {
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM widget_instances WHERE id=?1)",
                [owner],
                |row| row.get(0),
            )
            .map_err(err)?;
        if !exists {
            return Ok(false);
        }
        let manifest = instance_manifest(db, owner)?;
        Ok(manifest.capabilities.iter().any(|cap| cap == capability)
            && read(db, &binding_key(owner))?.is_none())
    }
}
pub(crate) fn native_sources(db: &Connection, capability: &str) -> AppResult<Vec<String>> {
    let mut stmt=db.prepare("SELECT i.id,p.manifest FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id ORDER BY i.id").map_err(err)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(err)?;
    let mut owners = std::collections::BTreeSet::new();
    for row in rows {
        let (id, raw) = row.map_err(err)?;
        let manifest: Manifest = serde_json::from_str(&raw).map_err(err)?;
        if manifest.capabilities.iter().any(|cap| cap == capability) {
            owners.insert(native_source(db, &id, capability)?);
        }
    }
    Ok(owners.into_iter().collect())
}
pub(crate) fn native_cursor(session: &mut Session, owner: &str, value: &Value) {
    session.shared_cursor = owner
        .strip_prefix("shared/")
        .and_then(|id| value["revision"].as_u64().map(|rev| (id.to_owned(), rev)));
}
fn pause_native(db: &Connection, owner: &str, kind: Kind) -> AppResult<()> {
    match kind {
        Kind::Todo => crate::todo::pause_source(db, owner),
        Kind::Pomodoro => crate::pomodoro::pause_source(db, owner, now_ms()),
        _ => Ok(()),
    }
}
fn switch_native(db: &Connection, instance: &str, kind: Kind, target: &str) -> AppResult<()> {
    let previous = read(db, &binding_key(instance))?;
    let current = previous
        .as_deref()
        .map(native_owner)
        .unwrap_or_else(|| instance.into());
    if current != target
        && previous
            .as_deref()
            .map(|id| consumers(db, id))
            .transpose()?
            .unwrap_or(1)
            <= 1
    {
        pause_native(db, &current, kind)?;
    }
    Ok(())
}
fn metadata(db: &Connection, value: &Record) -> AppResult<Value> {
    let mut result = serde_json::to_value(value).map_err(err)?;
    result["consumers"] = json!(consumers(db, &value.id)?);
    if value.kind.native() {
        result["contentRevision"] = json!(native_revision(db, &value.id)?);
    }
    Ok(result)
}
fn private(raw: &str) -> AppResult<Value> {
    serde_json::from_str(raw).map_err(|_| "DATA_UNAVAILABLE: 私有文档损坏，未覆盖原数据".into())
}
pub(crate) fn storage_get(
    db: &Connection,
    session: &mut Session,
    manifest: &Manifest,
    raw: &str,
) -> AppResult<Option<Value>> {
    if !manifest
        .capabilities
        .iter()
        .any(|cap| cap == "storage.shared")
    {
        return Ok(None);
    }
    let kind = kind(manifest)?;
    if kind.native() {
        return Ok(None);
    }
    let Some(library) = bound(db, &session.instance, kind)? else {
        session.shared_cursor = Some((String::new(), 0));
        return Ok(None);
    };
    let data = payload(db, &library)?;
    let view = match read(db, &view_key(&session.instance, &library.id))? {
        Some(raw) => serde_json::from_str(&raw).map_err(err)?,
        None => split(kind, &private(raw)?)?.1,
    };
    let document = compose(kind, data, &view);
    session.shared_cursor = Some((library.id, library.revision));
    Ok(Some(document))
}
pub(crate) fn storage_set(
    db: &mut Connection,
    session: &mut Session,
    manifest: &Manifest,
    args: &Value,
) -> AppResult<bool> {
    if !manifest
        .capabilities
        .iter()
        .any(|cap| cap == "storage.shared")
    {
        return Ok(false);
    }
    let kind = kind(manifest)?;
    if kind.native() {
        return Ok(false);
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    let Some(mut library) = bound(&tx, &session.instance, kind)? else {
        if session
            .shared_cursor
            .as_ref()
            .is_some_and(|(id, _)| !id.is_empty())
        {
            return Err("SESSION_EXPIRED: 数据源已切换，请重新载入".into());
        }
        return Ok(false);
    };
    if session.shared_cursor.as_ref() != Some(&(library.id.clone(), library.revision)) {
        return Err(
            "REVISION_CONFLICT: 共享库已更新，保留当前草稿，请导出或重新载入后再编辑".into(),
        );
    }
    let (data, view) = split(kind, args)?;
    let previous = payload(&tx, &library)?;
    if data != previous {
        if library.revision >= 9_007_199_254_740_991 {
            return Err("LIMIT: 共享库版本达到上限".into());
        }
        library.revision += 1;
        library.updated_at = now_ms();
        write(&tx, &data_key(&library.id), &data.to_string())?;
        write(
            &tx,
            &key(&library.id),
            &serde_json::to_string(&library).map_err(err)?,
        )?;
    }
    write(
        &tx,
        &view_key(&session.instance, &library.id),
        &view.to_string(),
    )?;
    tx.commit().map_err(err)?;
    session.shared_cursor = Some((library.id, library.revision));
    Ok(true)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bind {
    id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Alter {
    id: String,
    expected_revision: u64,
    name: Option<String>,
}
pub(crate) fn command(
    db: &mut Connection,
    session: &Session,
    manifest: &Manifest,
    method: &str,
    args: Value,
    raw: &str,
) -> AppResult<(Value, bool)> {
    let kind = kind(manifest)?;
    if method == "library.status" || method == "library.list" {
        if !args.is_null() && args.as_object().is_none_or(|args| !args.is_empty()) {
            return Err("INVALID_ARGUMENT: 此共享库读取不接受参数".into());
        }
        if method == "library.status" {
            let library = bound(db, &session.instance, kind)?;
            let binding = library.as_ref().map(|v| metadata(db, v)).transpose()?;
            let revision = library.as_ref().and_then(|library| {
                session
                    .shared_cursor
                    .as_ref()
                    .filter(|(id, _)| *id == library.id)
                    .map(|(_, rev)| *rev)
            });
            return Ok((
                json!({"mode":if library.is_some(){"shared"}else{"instance"},"binding":binding,"readRevision":revision}),
                false,
            ));
        }
        let mut query = db
            .prepare(
                "SELECT value FROM app_settings WHERE key LIKE 'shared-library:%' ORDER BY key",
            )
            .map_err(err)?;
        let rows = query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(err)?;
        let mut values = Vec::new();
        for row in rows {
            let value: Record = serde_json::from_str(&row.map_err(err)?)
                .map_err(|_| "DATA_UNAVAILABLE: 共享库元数据损坏")?;
            if value.kind == kind {
                values.push(metadata(db, &record(db, &value.id)?)?);
            }
        }
        return Ok((json!(values), false));
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    let mut invalidate = false;
    let result = match method {
        "library.create" => {
            let args: Create =
                serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENT: 新建共享库参数无效")?;
            let name = name(&args.name)?;
            let count: u64 = tx
                .query_row(
                    "SELECT count(*) FROM app_settings WHERE key LIKE 'shared-library:%'",
                    [],
                    |row| row.get(0),
                )
                .map_err(err)?;
            if count >= 64 {
                return Err("LIMIT: 本地共享库最多64个".into());
            }
            let content = if kind.native() {
                Value::Null
            } else if let Some(library) = bound(&tx, &session.instance, kind)? {
                payload(&tx, &library)?
            } else {
                split(kind, &private(raw)?)?.0
            };
            let library = Record {
                version: 1,
                id: uuid::Uuid::new_v4().to_string(),
                kind,
                name,
                revision: 1,
                updated_at: now_ms(),
            };
            write(
                &tx,
                &key(&library.id),
                &serde_json::to_string(&library).map_err(err)?,
            )?;
            if kind.native() {
                let capability = if kind == Kind::Todo {
                    "todos.manage"
                } else {
                    "focus.pomodoro"
                };
                let source = native_source(&tx, &session.instance, capability)?;
                let target = native_owner(&library.id);
                if kind == Kind::Todo {
                    crate::todo::copy_source(&tx, &source, &target)?;
                } else {
                    crate::pomodoro::copy_source(&tx, &source, &target)?;
                }
                switch_native(&tx, &session.instance, kind, &target)?;
                write(&tx, &binding_key(&session.instance), &library.id)?;
                invalidate = true;
                let mut result = metadata(&tx, &library)?;
                result["reloadRequired"] = json!(true);
                result
            } else {
                write(&tx, &data_key(&library.id), &content.to_string())?;
                metadata(&tx, &library)?
            }
        }
        "library.bind" => {
            let args: Bind =
                serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENT: 切换共享库参数无效")?;
            if let Some(id) = args.id {
                let library = record(&tx, &id)?;
                if library.kind != kind {
                    return Err("CAPABILITY_DENIED: 不能连接其他类型的共享库".into());
                }
                if kind.native() {
                    switch_native(&tx, &session.instance, kind, &native_owner(&id))?;
                }
                write(&tx, &binding_key(&session.instance), &id)?;
            } else {
                if kind.native() {
                    switch_native(&tx, &session.instance, kind, &session.instance)?;
                }
                tx.execute(
                    "DELETE FROM app_settings WHERE key=?1",
                    [binding_key(&session.instance)],
                )
                .map_err(err)?;
            }
            invalidate = true;
            json!({"reloadRequired":true})
        }
        "library.rename" | "library.remove" => {
            let args: Alter =
                serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENT: 共享库修改参数无效")?;
            let mut library = record(&tx, &args.id)?;
            if library.kind != kind {
                return Err("CAPABILITY_DENIED: 共享库类型不匹配".into());
            }
            if library.revision != args.expected_revision {
                return Err("REVISION_CONFLICT: 库已更新，请刷新列表后操作".into());
            }
            if method == "library.remove" {
                if args.name.is_some() {
                    return Err("INVALID_ARGUMENT: 删除不接受名称".into());
                }
                if consumers(&tx, &args.id)? > 0 {
                    return Err("CONFLICT: 共享库仍有实例连接，请先断开".into());
                }
                if kind.native() {
                    let owner = native_owner(&args.id);
                    if kind == Kind::Todo {
                        crate::todo::remove_data(&tx, &owner)?;
                    } else {
                        crate::pomodoro::remove_data(&tx, &owner)?;
                    }
                    tx.execute(
                        "DELETE FROM app_settings WHERE key=?1",
                        [native_revision_key(&args.id)],
                    )
                    .map_err(err)?;
                }
                tx.execute("DELETE FROM app_settings WHERE key=?1", [key(&args.id)])
                    .map_err(err)?;
                tx.execute(
                    "DELETE FROM app_settings WHERE key=?1",
                    [data_key(&args.id)],
                )
                .map_err(err)?;
                Value::Null
            } else {
                library.name = name(args.name.as_deref().ok_or("INVALID_ARGUMENT: 缺少名称")?)?;
                if library.revision >= 9_007_199_254_740_991 {
                    return Err("LIMIT: 共享库版本达到上限".into());
                }
                library.revision += 1;
                library.updated_at = now_ms();
                write(
                    &tx,
                    &key(&args.id),
                    &serde_json::to_string(&library).map_err(err)?,
                )?;
                metadata(&tx, &library)?
            }
        }
        _ => return Err("CAPABILITY_DENIED: 未开放的共享库方法".into()),
    };
    tx.commit().map_err(err)?;
    Ok((result, invalidate))
}
pub(crate) fn remove_data(db: &Connection, instance: &str) -> AppResult<()> {
    if let Some(id) = read(db, &binding_key(instance))? {
        let library = record(db, &id)?;
        if library.kind.native() && consumers(db, &id)? <= 1 {
            pause_native(db, &native_owner(&id), library.kind)?;
        }
    }
    db.execute(
        "DELETE FROM app_settings WHERE key=?1",
        [binding_key(instance)],
    )
    .map_err(err)?;
    db.execute(
        "DELETE FROM app_settings WHERE key LIKE ?1",
        [format!("shared-view:{instance}:%")],
    )
    .map_err(err)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{store::Store, widgets};
    use serde_json::{json, Value};
    #[test]
    fn native_todo_and_calendar_share_business_data_but_keep_settings_and_instance_removal_independent(
    ) {
        let (s, sessions, a, b) = native_fixture("todo");
        let package = "local.souldesk.widgetjs.calendar-large";
        let staged = widgets::stage_bytes(
            &s,
            crate::presets::find(package)
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        widgets::activate(&s, &staged).unwrap();
        let calendar = widgets::create_instance(&s, package).unwrap();
        let a0 = open(&s, &sessions, &a);
        let c0 = open(&s, &sessions, &calendar);
        let library = call(
            &s,
            &sessions,
            &a,
            &a0,
            "library.create",
            json!({"name":"待办与日历"}),
        );
        let id = library["id"].as_str().unwrap();
        let a1 = open(&s, &sessions, &a);
        call(
            &s,
            &sessions,
            &calendar,
            &c0,
            "library.bind",
            json!({"id":id}),
        );
        let c1 = open(&s, &sessions, &calendar);
        let at = now_ms();
        let mut args = change_request(1, "save");
        args["todo"] = json!({"id":uuid::Uuid::new_v4().to_string(),"title":"从日历创建","dueDateTime":chrono::DateTime::<chrono::Utc>::from_timestamp_millis((at+86400000)as i64).unwrap().to_rfc3339(),"isReminderOn":false});
        business(&s, &sessions, &calendar, &c1, "todo.change", args, at).unwrap();
        assert_eq!(
            business(&s, &sessions, &a, &a1, "todo.get", Value::Null, at).unwrap()["todos"][0]
                ["title"],
            "从日历创建"
        );
        let before = call(&s, &sessions, &calendar, &c1, "settings.get", Value::Null);
        let mut settings = call(&s, &sessions, &a, &a1, "settings.get", Value::Null);
        settings["background"] = json!("#123456");
        call(&s, &sessions, &a, &a1, "settings.update", settings);
        assert_eq!(
            call(&s, &sessions, &calendar, &c1, "settings.get", Value::Null),
            before
        );
        widgets::remove_instance(&s, &sessions, &a).unwrap();
        assert!(native_active(&s.db.lock().unwrap(), &native_owner(id), "todos.manage").unwrap());
        assert_eq!(
            business(&s, &sessions, &calendar, &c1, "todo.get", Value::Null, at).unwrap()["todos"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        widgets::remove_instance(&s, &sessions, &calendar).unwrap();
        assert!(!native_active(&s.db.lock().unwrap(), &native_owner(id), "todos.manage").unwrap());
        let b0 = open(&s, &sessions, &b);
        assert_eq!(
            call(&s, &sessions, &b, &b0, "library.list", Value::Null)[0]["consumers"],
            0
        );
    }
    #[test]
    fn native_source_switch_refuses_other_detail_sessions_before_discarding_their_drafts() {
        let (s, sessions, a, _) = native_fixture("pomodoro");
        let token = open(&s, &sessions, &a);
        let detail =
            widgets::open_display_session(&s, &sessions, &format!("detail-{a}"), &a, None).unwrap();
        let detail_token = detail["token"].as_str().unwrap();
        let result = widgets::gateway(
            &s,
            &sessions,
            &format!("float-{a}"),
            &token,
            "library.create",
            json!({"name":"不能丢草稿"}),
        );
        assert!(
            result.is_err(),
            "有详情会话时必须先拒绝切源，不能先提交再广播丢弃详情草稿"
        );
        assert!(sessions.0.lock().unwrap().contains_key(detail_token));
        assert!(sessions.0.lock().unwrap().contains_key(&token));
        sessions.0.lock().unwrap().remove(detail_token);
        let library = call(
            &s,
            &sessions,
            &a,
            &token,
            "library.create",
            json!({"name":"已关闭详情"}),
        );
        assert_eq!(library["reloadRequired"], true);
        let token = open(&s, &sessions, &a);
        widgets::open_display_session(&s, &sessions, &format!("detail-{a}"), &a, None).unwrap();
        assert!(widgets::gateway(
            &s,
            &sessions,
            &format!("float-{a}"),
            &token,
            "library.bind",
            json!({"id":null})
        )
        .is_err());
        assert_eq!(
            call(&s, &sessions, &a, &token, "library.status", Value::Null)["mode"],
            "shared"
        );
    }
    fn business(
        store: &Store,
        sessions: &widgets::Sessions,
        id: &str,
        token: &str,
        method: &str,
        args: Value,
        at: u64,
    ) -> AppResult<Value> {
        if method.starts_with("todo.") {
            crate::todo::authorized(
                store,
                sessions,
                &format!("float-{id}"),
                token,
                method,
                args,
                at as i64,
            )
        } else {
            crate::pomodoro::authorized(
                store,
                sessions,
                &format!("float-{id}"),
                token,
                method,
                args,
                at,
            )
        }
    }
    fn change_request(revision: u64, operation: &str) -> Value {
        json!({"requestId":uuid::Uuid::new_v4().to_string(),"expectedRevision":revision,"operation":operation})
    }
    #[test]
    fn native_todo_business_is_shared_versioned_and_restores_private_data() {
        let (s, sessions, a, b) = native_fixture("todo");
        let at = now_ms();
        let a0 = open(&s, &sessions, &a);
        let b0 = open(&s, &sessions, &b);
        let item = json!({"id":uuid::Uuid::new_v4().to_string(),"title":"原私有事项","isReminderOn":false});
        let mut args = change_request(0, "save");
        args["todo"] = item.clone();
        business(&s, &sessions, &a, &a0, "todo.change", args, at).unwrap();
        let library = call(
            &s,
            &sessions,
            &a,
            &a0,
            "library.create",
            json!({"name":"共同待办"}),
        );
        let library_id = library["id"].as_str().unwrap();
        assert!(business(&s, &sessions, &a, &a0, "todo.get", Value::Null, at).is_err());
        let a1 = open(&s, &sessions, &a);
        call(
            &s,
            &sessions,
            &b,
            &b0,
            "library.bind",
            json!({"id":library_id}),
        );
        let b1 = open(&s, &sessions, &b);
        let first = business(&s, &sessions, &a, &a1, "todo.get", Value::Null, at).unwrap();
        let other = business(&s, &sessions, &b, &b1, "todo.get", Value::Null, at).unwrap();
        assert_eq!(first["todos"], other["todos"]);
        let mut args = change_request(first["revision"].as_u64().unwrap(), "save");
        args["todo"] = first["todos"][0].clone();
        args["todo"]["title"] = json!("共享修改");
        business(&s, &sessions, &a, &a1, "todo.change", args.clone(), at).unwrap();
        args["requestId"] = json!(uuid::Uuid::new_v4().to_string());
        assert!(business(&s, &sessions, &b, &b1, "todo.change", args, at)
            .unwrap_err()
            .starts_with("REVISION_CONFLICT:"));
        let updated = business(&s, &sessions, &b, &b1, "todo.get", Value::Null, at).unwrap();
        assert_eq!(updated["todos"][0]["title"], "共享修改");
        let status = call(&s, &sessions, &b, &b1, "library.status", Value::Null);
        assert_eq!(status["binding"]["contentRevision"], updated["revision"]);
        assert_eq!(status["readRevision"], updated["revision"]);
        assert_eq!(status["binding"]["revision"], 1);
        s.presentation(&a, "desktop", false, false).unwrap();
        s.presentation(&b, "desktop", false, false).unwrap();
        assert_eq!(
            native_sources(&s.db.lock().unwrap(), "todos.manage").unwrap(),
            vec![native_owner(library_id)]
        );
        call(&s, &sessions, &b, &b1, "library.bind", json!({"id":null}));
        assert!(business(&s, &sessions, &a, &a1, "todo.get", Value::Null, at).is_ok());
        s.presentation(&a, "desktop", true, false).unwrap();
        s.presentation(&b, "desktop", true, false).unwrap();
        let b2 = open(&s, &sessions, &b);
        assert!(
            business(&s, &sessions, &b, &b2, "todo.get", Value::Null, at).unwrap()["todos"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        call(&s, &sessions, &a, &a1, "library.bind", json!({"id":null}));
        let a2 = open(&s, &sessions, &a);
        assert_eq!(
            business(&s, &sessions, &a, &a2, "todo.get", Value::Null, at).unwrap()["todos"][0]
                ["title"],
            "原私有事项"
        );
        assert!(!native_active(
            &s.db.lock().unwrap(),
            &native_owner(library_id),
            "todos.manage"
        )
        .unwrap());
        call(
            &s,
            &sessions,
            &a,
            &a2,
            "library.remove",
            json!({"id":library_id,"expectedRevision":1}),
        );
        assert!(!native_active(
            &s.db.lock().unwrap(),
            &native_owner(library_id),
            "todos.manage"
        )
        .unwrap());
    }
    #[test]
    fn native_pomodoro_copy_stops_new_cycle_and_only_last_consumer_pauses_shared_source() {
        let (s, sessions, a, b) = native_fixture("pomodoro");
        let at = now_ms();
        let a0 = open(&s, &sessions, &a);
        let b0 = open(&s, &sessions, &b);
        let mut args = change_request(0, "initialize");
        args["templates"] = json!(["reading"]);
        let initial = business(&s, &sessions, &a, &a0, "pomodoro.change", args, at).unwrap();
        business(
            &s,
            &sessions,
            &a,
            &a0,
            "pomodoro.change",
            change_request(initial["revision"].as_u64().unwrap(), "start"),
            at,
        )
        .unwrap();
        let library = call(
            &s,
            &sessions,
            &a,
            &a0,
            "library.create",
            json!({"name":"共同专注"}),
        );
        let id = library["id"].as_str().unwrap();
        let a1 = open(&s, &sessions, &a);
        let copied = business(&s, &sessions, &a, &a1, "pomodoro.get", Value::Null, at).unwrap();
        assert_eq!(copied["phase"]["status"], "stop");
        assert_eq!(copied["scenes"], initial["scenes"]);
        assert!(copied["notice"].is_null());
        call(&s, &sessions, &b, &b0, "library.bind", json!({"id":id}));
        let b1 = open(&s, &sessions, &b);
        business(
            &s,
            &sessions,
            &a,
            &a1,
            "pomodoro.change",
            change_request(copied["revision"].as_u64().unwrap(), "start"),
            at,
        )
        .unwrap();
        call(&s, &sessions, &a, &a1, "library.bind", json!({"id":null}));
        let active = business(&s, &sessions, &b, &b1, "pomodoro.get", Value::Null, at).unwrap();
        assert_eq!(active["phase"]["status"], "running");
        let a2 = open(&s, &sessions, &a);
        let restored = business(
            &s,
            &sessions,
            &a,
            &a2,
            "pomodoro.get",
            Value::Null,
            at + 60000,
        )
        .unwrap();
        assert_eq!(restored["phase"]["status"], "pause");
        assert_eq!(
            native_sources(&s.db.lock().unwrap(), "focus.pomodoro")
                .unwrap()
                .len(),
            2
        );
        call(&s, &sessions, &b, &b1, "library.bind", json!({"id":null}));
        assert!(
            !native_active(&s.db.lock().unwrap(), &native_owner(id), "focus.pomodoro").unwrap()
        );
        call(&s, &sessions, &a, &a2, "library.bind", json!({"id":id}));
        let a3 = open(&s, &sessions, &a);
        assert_eq!(
            business(
                &s,
                &sessions,
                &a,
                &a3,
                "pomodoro.get",
                Value::Null,
                at + 120000
            )
            .unwrap()["phase"]["status"],
            "pause"
        );
    }
    #[test]
    fn native_copy_bind_failure_rolls_back_original_phase_and_new_library() {
        let (s, sessions, a, _) = native_fixture("pomodoro");
        let token = open(&s, &sessions, &a);
        let at = now_ms();
        let mut args = change_request(0, "initialize");
        args["templates"] = json!(["coding"]);
        let d = business(&s, &sessions, &a, &token, "pomodoro.change", args, at).unwrap();
        business(
            &s,
            &sessions,
            &a,
            &token,
            "pomodoro.change",
            change_request(d["revision"].as_u64().unwrap(), "start"),
            at,
        )
        .unwrap();
        let before: Vec<(String, String)> = {
            let db = s.db.lock().unwrap();
            let mut stmt = db
                .prepare("SELECT key,value FROM app_settings ORDER BY key")
                .unwrap();
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            db.execute_batch("CREATE TRIGGER refuse_native_binding BEFORE INSERT ON app_settings WHEN new.key LIKE 'shared-binding:%' BEGIN SELECT RAISE(FAIL,'binding denied'); END;").unwrap();
            rows
        };
        assert!(widgets::gateway(
            &s,
            &sessions,
            &format!("float-{a}"),
            &token,
            "library.create",
            json!({"name":"不应半完成"})
        )
        .is_err());
        assert!(sessions.0.lock().unwrap().contains_key(&token));
        let db = s.db.lock().unwrap();
        let mut stmt = db
            .prepare("SELECT key,value FROM app_settings ORDER BY key")
            .unwrap();
        let after = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(before, after);
    }
    #[test]
    fn native_metadata_and_create_do_not_load_private_generic_storage() {
        let (s, sessions, a, _) = native_fixture("todo");
        let token = open(&s, &sessions, &a);
        s.db.lock()
            .unwrap()
            .execute(
                "UPDATE widget_instances SET storage=zeroblob(33554432) WHERE id=?1",
                [&a],
            )
            .unwrap();
        assert!(
            call(&s, &sessions, &a, &token, "library.status", Value::Null)["binding"].is_null()
        );
        assert_eq!(
            call(
                &s,
                &sessions,
                &a,
                &token,
                "library.create",
                json!({"name":"原生内容"})
            )["reloadRequired"],
            true
        );
    }
    fn native_fixture(name: &str) -> (Store, widgets::Sessions, String, String) {
        let (store, sessions, a, b) = fixture(name);
        {
            let db = store.db.lock().unwrap();
            let package = format!("local.souldesk.widgetjs.{name}");
            let raw: String = db
                .query_row(
                    "SELECT manifest FROM widget_packages WHERE id=?1",
                    [&package],
                    |row| row.get(0),
                )
                .unwrap();
            let mut manifest: Value = serde_json::from_str(&raw).unwrap();
            if !manifest["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!("storage.shared"))
            {
                manifest["capabilities"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!("storage.shared"));
            }
            db.execute(
                "UPDATE widget_packages SET manifest=?1 WHERE id=?2",
                params![manifest.to_string(), package],
            )
            .unwrap();
        }
        (store, sessions, a, b)
    }
    #[test]
    fn native_todo_library_create_is_atomic_and_storage_stays_private() {
        let (store, sessions, a, b) = native_fixture("todo");
        let t = open(&store, &sessions, &a);
        let created = call(
            &store,
            &sessions,
            &a,
            &t,
            "library.create",
            json!({"name":"共同事项"}),
        );
        assert_eq!(
            created["reloadRequired"], true,
            "原生复制和连接必须在一次事务内完成"
        );
        assert!(!sessions.0.lock().unwrap().contains_key(&t));
        let t = open(&store, &sessions, &a);
        let status = call(&store, &sessions, &a, &t, "library.status", Value::Null);
        assert_eq!(status["mode"], "shared");
        assert_eq!(status["binding"]["kind"], "todo");
        assert!(status["binding"]["contentRevision"].is_u64());
        call(
            &store,
            &sessions,
            &a,
            &t,
            "storage.set",
            json!({"verification":{"private":true}}),
        );
        assert_eq!(
            call(&store, &sessions, &a, &t, "storage.get", Value::Null),
            json!({"verification":{"private":true}})
        );
        let other = open(&store, &sessions, &b);
        assert_eq!(
            call(&store, &sessions, &b, &other, "library.status", Value::Null)["mode"],
            "instance"
        );
    }
    #[test]
    fn native_pomodoro_library_has_separate_kind_and_metadata_revision() {
        let (store, sessions, a, _) = native_fixture("pomodoro");
        let t = open(&store, &sessions, &a);
        let created = call(
            &store,
            &sessions,
            &a,
            &t,
            "library.create",
            json!({"name":"共同专注"}),
        );
        assert_eq!(created["reloadRequired"], true);
        assert_eq!(created["kind"], "pomodoro");
        assert_eq!(created["revision"], 1);
        assert!(created["contentRevision"].is_u64());
    }
    fn fixture(name: &str) -> (Store, widgets::Sessions, String, String) {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-shared-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let id = format!("local.souldesk.widgetjs.{name}");
        let preset = crate::presets::find(&id).unwrap().unwrap();
        let staged = widgets::stage_bytes(&store, preset.archive().unwrap()).unwrap();
        widgets::activate(&store, &staged).unwrap();
        let a = widgets::create_instance(&store, &id).unwrap();
        let b = widgets::create_instance(&store, &id).unwrap();
        (store, widgets::Sessions::default(), a, b)
    }
    fn open(s: &Store, sessions: &widgets::Sessions, id: &str) -> String {
        widgets::open_display_session(s, sessions, &format!("float-{id}"), id, None).unwrap()
            ["token"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn call(
        s: &Store,
        sessions: &widgets::Sessions,
        id: &str,
        t: &str,
        method: &str,
        args: Value,
    ) -> Value {
        widgets::gateway(s, sessions, &format!("float-{id}"), t, method, args).unwrap()
    }
    fn birthday(name: &str) -> Value {
        json!({"version":1,"birthdays":[{"id":"fixture","name":name,"dateType":0,"year":2000,"month":1,"dayOfMonth":1,"introduction":"","sortOrder":0,"createdAt":0,"updatedAt":0}]})
    }

    #[test]
    fn ordinary_mindmap_package_upgrade_preserves_nonempty_private_shared_and_view_data() {
        use std::io::{Cursor, Read, Write};
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-shared-upgrade-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let package = "local.souldesk.widgetjs.mindmap";
        let bytes = crate::presets::find(package)
            .unwrap()
            .unwrap()
            .archive()
            .unwrap();
        let mut input = zip::ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
        let mut old = zip::ZipWriter::new(Cursor::new(Vec::new()));
        // 仅构造0.2.0清单的升级夹具，不把当前资源当作旧程序实机证据。
        for index in 0..input.len() {
            let mut file = input.by_index(index).unwrap();
            let name = file.name().to_owned();
            let mut body = Vec::new();
            file.read_to_end(&mut body).unwrap();
            if name == "manifest.json" {
                let mut manifest: Value = serde_json::from_slice(&body).unwrap();
                manifest["version"] = json!("0.2.0");
                body = serde_json::to_vec(&manifest).unwrap();
            }
            old.start_file(
                name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
            old.write_all(&body).unwrap();
        }
        let baseline = widgets::stage_bytes(&store, old.finish().unwrap().into_inner()).unwrap();
        widgets::activate(&store, &baseline).unwrap();
        let a = widgets::create_instance(&store, package).unwrap();
        let b = widgets::create_instance(&store, package).unwrap();
        let sessions = widgets::Sessions::default();
        let ta = open(&store, &sessions, &a);
        let mut tb = open(&store, &sessions, &b);
        let document = |topic: &str, theme: &str| json!({"version":1,"documents":[{"id":uuid::Uuid::new_v4().to_string(),"topic":topic,"emoji":"📖","group":"升级前资料","mindmap":{"id":"root","name":topic,"checked":true,"children":[{"id":"child","name":"不能丢失的子节点"}]},"createdAt":1000,"updatedAt":2000}],"preferences":{"theme":theme,"showGrid":true,"showCheckboxes":true,"sidebarOpen":false}});
        let private_a = document("私有甲", "dark");
        let private_b = document("私有乙", "light");
        call(&store, &sessions, &a, &ta, "storage.set", private_a.clone());
        call(&store, &sessions, &b, &tb, "storage.set", private_b);
        let library = call(
            &store,
            &sessions,
            &a,
            &ta,
            "library.create",
            json!({"name":"升级前共享库"}),
        );
        let library_id = library["id"].as_str().unwrap();
        call(
            &store,
            &sessions,
            &b,
            &tb,
            "library.bind",
            json!({"id":library_id}),
        );
        tb = open(&store, &sessions, &b);
        let mut shared = call(&store, &sessions, &b, &tb, "storage.get", Value::Null);
        shared["documents"][0]["topic"] = json!("共享资料");
        shared["documents"][0]["mindmap"]["name"] = json!("共享资料");
        let shared = call(&store, &sessions, &b, &tb, "storage.set", shared);
        assert_eq!(shared["preferences"]["theme"], "light");
        fn shared_rows(store: &Store) -> Vec<(String, String)> {
            let db = store.db.lock().unwrap();
            let mut stmt = db
                .prepare(
                    "SELECT key,value FROM app_settings WHERE key LIKE 'shared-%' ORDER BY key",
                )
                .unwrap();
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows
        }
        let before_shared = shared_rows(&store);
        let before_instances = store.snapshot().unwrap()["instances"].clone();
        let next = widgets::stage_bytes(&store, bytes).unwrap();
        assert_ne!(baseline.digest, next.digest);
        widgets::activate_sessions(&store, &sessions, &next).unwrap();
        assert!(sessions.0.lock().unwrap().is_empty());
        assert_eq!(store.snapshot().unwrap()["instances"], before_instances);
        assert_eq!(shared_rows(&store), before_shared);
        assert!(widgets::gateway(
            &store,
            &sessions,
            &format!("float-{a}"),
            &ta,
            "storage.set",
            json!({"version":1,"documents":[]})
        )
        .unwrap_err()
        .starts_with("SESSION_EXPIRED:"));
        let new_a = open(&store, &sessions, &a);
        let new_b = open(&store, &sessions, &b);
        assert_eq!(
            call(&store, &sessions, &a, &new_a, "storage.get", Value::Null),
            private_a
        );
        assert_eq!(
            call(&store, &sessions, &b, &new_b, "storage.get", Value::Null),
            shared
        );
        assert_eq!(
            call(&store, &sessions, &b, &new_b, "library.status", Value::Null)["binding"]["id"],
            library_id
        );
        assert_eq!(
            store
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM widget_upgrade_snapshots WHERE package_id=?1",
                    [package],
                    |r| r.get::<_, u32>(0)
                )
                .unwrap(),
            2
        );
        let root = store.root.clone();
        drop(store);
        let reopened = Store::open(root).unwrap();
        let new_sessions = widgets::Sessions::default();
        let token = open(&reopened, &new_sessions, &b);
        assert_eq!(
            call(
                &reopened,
                &new_sessions,
                &b,
                &token,
                "storage.get",
                Value::Null
            ),
            shared
        );
    }

    #[test]
    fn shared_capability_kind_preview_and_connected_deletion_are_enforced() {
        use std::sync::{Arc, Mutex};
        let (s, sessions, a, _) = fixture("birthday-list");
        let token = open(&s, &sessions, &a);
        call(&s, &sessions, &a, &token, "storage.set", birthday("私有"));
        let library = call(
            &s,
            &sessions,
            &a,
            &token,
            "library.create",
            json!({"name":"生日"}),
        );
        let id = library["id"].as_str().unwrap();
        assert!(
            !call(&s, &sessions, &a, &token, "library.list", Value::Null)
                .to_string()
                .contains("私有")
        );
        assert!(widgets::gateway(
            &s,
            &sessions,
            "other-window",
            &token,
            "library.list",
            Value::Null
        )
        .is_err());
        let staged = widgets::stage_bytes(
            &s,
            crate::presets::find("local.souldesk.widgetjs.countdown")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        widgets::activate(&s, &staged).unwrap();
        let other = widgets::create_instance(&s, "local.souldesk.widgetjs.countdown").unwrap();
        let other_token = open(&s, &sessions, &other);
        assert!(widgets::gateway(
            &s,
            &sessions,
            &format!("float-{other}"),
            &other_token,
            "library.bind",
            json!({"id":id})
        )
        .unwrap_err()
        .starts_with("CAPABILITY_DENIED:"));
        let preview = Arc::new(Mutex::new(widgets::Preview {
            settings: widgets::initial_settings(&staged.manifest),
            storage: json!({}),
            package: staged,
            ready_since: None,
            failure: None,
        }));
        let trial =
            widgets::open_preview(&sessions, "trial-library", "trial-library", preview).unwrap();
        assert!(widgets::gateway(
            &s,
            &sessions,
            "trial-library",
            trial["token"].as_str().unwrap(),
            "library.list",
            Value::Null
        )
        .is_err());
        call(&s, &sessions, &a, &token, "library.bind", json!({"id":id}));
        let token = open(&s, &sessions, &a);
        assert!(widgets::gateway(
            &s,
            &sessions,
            &format!("float-{a}"),
            &token,
            "library.remove",
            json!({"id":id,"expectedRevision":1})
        )
        .unwrap_err()
        .starts_with("CONFLICT:"));
        let (no_cap, no_sessions, no_a, _) = fixture("note");
        let no_token = open(&no_cap, &no_sessions, &no_a);
        assert!(widgets::gateway(
            &no_cap,
            &no_sessions,
            &format!("float-{no_a}"),
            &no_token,
            "library.create",
            json!({"name":"不应创建"})
        )
        .is_err());
    }

    #[test]
    fn shared_write_failure_rolls_back_content_metadata_and_cursor() {
        let (s, sessions, a, _) = fixture("birthday-list");
        let token = open(&s, &sessions, &a);
        call(&s, &sessions, &a, &token, "storage.set", birthday("原值"));
        let library = call(
            &s,
            &sessions,
            &a,
            &token,
            "library.create",
            json!({"name":"回滚"}),
        );
        let id = library["id"].as_str().unwrap();
        call(&s, &sessions, &a, &token, "library.bind", json!({"id":id}));
        let token = open(&s, &sessions, &a);
        call(&s, &sessions, &a, &token, "storage.get", Value::Null);
        s.db.lock().unwrap().execute_batch("CREATE TRIGGER shared_fixture_failure BEFORE INSERT ON app_settings WHEN NEW.key LIKE 'shared-view:%' BEGIN SELECT RAISE(FAIL,'fixture write failure'); END;").unwrap();
        assert!(widgets::gateway(
            &s,
            &sessions,
            &format!("float-{a}"),
            &token,
            "storage.set",
            birthday("不得部分写入")
        )
        .is_err());
        {
            let db = s.db.lock().unwrap();
            let meta = record(&db, id).unwrap();
            assert_eq!(meta.revision, 1);
            assert_eq!(payload(&db, &meta).unwrap(), birthday("原值"));
            db.execute_batch("DROP TRIGGER shared_fixture_failure;")
                .unwrap();
        }
        call(
            &s,
            &sessions,
            &a,
            &token,
            "storage.set",
            birthday("重试成功"),
        );
        assert_eq!(record(&s.db.lock().unwrap(), id).unwrap().revision, 2);
    }

    #[test]
    fn shared_mindmap_preferences_are_private_and_invalid_library_changes_preserve_data() {
        let (s, sessions, a, b) = fixture("mindmap");
        let mut ta = open(&s, &sessions, &a);
        let mut tb = open(&s, &sessions, &b);
        let dark = json!({"version":1,"documents":[],"preferences":{"theme":"dark","showGrid":true,"showCheckboxes":true,"sidebarOpen":true}});
        let mut light = dark.clone();
        light["preferences"]["theme"] = json!("light");
        call(&s, &sessions, &a, &ta, "storage.set", dark);
        call(&s, &sessions, &b, &tb, "storage.set", light);
        let library = call(
            &s,
            &sessions,
            &a,
            &ta,
            "library.create",
            json!({"name":"导图库"}),
        );
        let id = library["id"].as_str().unwrap();
        for (instance, token) in [(&a, &ta), (&b, &tb)] {
            call(
                &s,
                &sessions,
                instance,
                token,
                "library.bind",
                json!({"id":id}),
            );
        }
        ta = open(&s, &sessions, &a);
        tb = open(&s, &sessions, &b);
        assert_eq!(
            call(&s, &sessions, &a, &ta, "storage.get", Value::Null)["preferences"]["theme"],
            "dark"
        );
        assert_eq!(
            call(&s, &sessions, &b, &tb, "storage.get", Value::Null)["preferences"]["theme"],
            "light"
        );
        assert!(widgets::gateway(
            &s,
            &sessions,
            &format!("float-{a}"),
            &ta,
            "storage.set",
            json!({"version":2,"documents":[]})
        )
        .is_err());
        assert_eq!(record(&s.db.lock().unwrap(), id).unwrap().revision, 1);
    }

    #[test]
    fn shared_rename_delete_require_matching_revision_and_size_limits_do_not_overwrite() {
        let (s, sessions, a, _) = fixture("birthday-list");
        let token = open(&s, &sessions, &a);
        call(
            &s,
            &sessions,
            &a,
            &token,
            "storage.set",
            birthday("保留私有"),
        );
        let library = call(
            &s,
            &sessions,
            &a,
            &token,
            "library.create",
            json!({"name":"库"}),
        );
        let id = library["id"].as_str().unwrap();
        assert!(widgets::gateway(
            &s,
            &sessions,
            &format!("float-{a}"),
            &token,
            "library.rename",
            json!({"id":id,"expectedRevision":1,"name":"\n"})
        )
        .is_err());
        call(
            &s,
            &sessions,
            &a,
            &token,
            "library.rename",
            json!({"id":id,"expectedRevision":1,"name":"改名"}),
        );
        assert!(widgets::gateway(
            &s,
            &sessions,
            &format!("float-{a}"),
            &token,
            "library.remove",
            json!({"id":id,"expectedRevision":1})
        )
        .is_err());
        assert!(validate_payload(
            Kind::Birthday,
            &json!({"version":1,"birthdays":[{"id":"huge","text":"x".repeat(4*1024*1024)}]})
        )
        .is_err());
        call(
            &s,
            &sessions,
            &a,
            &token,
            "library.remove",
            json!({"id":id,"expectedRevision":2}),
        );
        assert!(record(&s.db.lock().unwrap(), id).is_err());
        assert_eq!(
            call(&s, &sessions, &a, &token, "storage.get", Value::Null),
            birthday("保留私有")
        );
    }

    #[test]
    fn shared_metadata_does_not_load_or_decode_private_document_contents() {
        let (s, sessions, a, _) = fixture("mindmap");
        let token = open(&s, &sessions, &a);
        s.db.lock()
            .unwrap()
            .execute(
                "UPDATE widget_instances SET storage=zeroblob(33554432) WHERE id=?1",
                [&a],
            )
            .unwrap();
        assert_eq!(
            call(&s, &sessions, &a, &token, "library.status", Value::Null)["mode"],
            "instance"
        );
        assert_eq!(
            call(&s, &sessions, &a, &token, "library.list", Value::Null),
            json!([])
        );
        assert!(widgets::gateway(
            &s,
            &sessions,
            &format!("float-{a}"),
            &token,
            "storage.get",
            Value::Null
        )
        .is_err());
    }

    #[test]
    fn shared_versions_conflict_without_overwriting_private_documents_or_other_sessions() {
        let (s, sessions, a, b) = fixture("birthday-list");
        let mut ta = open(&s, &sessions, &a);
        let mut tb = open(&s, &sessions, &b);
        call(&s, &sessions, &a, &ta, "storage.set", birthday("甲私有"));
        call(&s, &sessions, &b, &tb, "storage.set", birthday("乙私有"));
        let created = call(
            &s,
            &sessions,
            &a,
            &ta,
            "library.create",
            json!({"name":"生日共享"}),
        );
        let id = created["id"].as_str().unwrap();
        call(&s, &sessions, &a, &ta, "library.bind", json!({"id":id}));
        assert!(widgets::gateway(
            &s,
            &sessions,
            &format!("float-{a}"),
            &ta,
            "storage.set",
            birthday("旧会话")
        )
        .is_err());
        ta = open(&s, &sessions, &a);
        call(&s, &sessions, &b, &tb, "library.bind", json!({"id":id}));
        tb = open(&s, &sessions, &b);
        assert_eq!(
            call(&s, &sessions, &a, &ta, "storage.get", Value::Null),
            birthday("甲私有")
        );
        assert_eq!(
            call(&s, &sessions, &b, &tb, "storage.get", Value::Null),
            birthday("甲私有")
        );
        call(&s, &sessions, &a, &ta, "storage.set", birthday("共享新值"));
        let error = widgets::gateway(
            &s,
            &sessions,
            &format!("float-{b}"),
            &tb,
            "storage.set",
            birthday("乙旧草稿"),
        )
        .unwrap_err();
        assert!(error.starts_with("REVISION_CONFLICT:"));
        call(&s, &sessions, &b, &tb, "library.status", Value::Null);
        assert!(widgets::gateway(
            &s,
            &sessions,
            &format!("float-{b}"),
            &tb,
            "storage.set",
            birthday("不能因状态读取而覆盖")
        )
        .is_err());
        call(&s, &sessions, &b, &tb, "library.bind", json!({"id":null}));
        tb = open(&s, &sessions, &b);
        assert_eq!(
            call(&s, &sessions, &b, &tb, "storage.get", Value::Null),
            birthday("乙私有")
        );
        widgets::remove_instance(&s, &sessions, &a).unwrap();
        assert_eq!(
            call(&s, &sessions, &b, &tb, "library.list", Value::Null)[0]["consumers"],
            0
        );
        assert!(record(&s.db.lock().unwrap(), id).is_ok());
    }

    #[test]
    fn shared_countdown_content_keeps_each_view_selection_and_sort() {
        let (s, sessions, a, b) = fixture("countdown");
        let mut ta = open(&s, &sessions, &a);
        let mut tb = open(&s, &sessions, &b);
        let data = json!({"version":1,"events":[{"id":"one"},{"id":"two"}],"selectedId":"one","sort":"asc"});
        call(&s, &sessions, &a, &ta, "storage.set", data.clone());
        call(
            &s,
            &sessions,
            &b,
            &tb,
            "storage.set",
            json!({"version":1,"events":[],"selectedId":"","sort":"desc"}),
        );
        let library = call(
            &s,
            &sessions,
            &a,
            &ta,
            "library.create",
            json!({"name":"日期库"}),
        );
        let id = library["id"].as_str().unwrap();
        for (instance, token) in [(&a, &ta), (&b, &tb)] {
            call(
                &s,
                &sessions,
                instance,
                token,
                "library.bind",
                json!({"id":id}),
            );
        }
        ta = open(&s, &sessions, &a);
        tb = open(&s, &sessions, &b);
        let mut va = call(&s, &sessions, &a, &ta, "storage.get", Value::Null);
        let vb = call(&s, &sessions, &b, &tb, "storage.get", Value::Null);
        assert_eq!(vb["sort"], "desc");
        va["selectedId"] = json!("two");
        call(&s, &sessions, &a, &ta, "storage.set", va);
        let vb = call(&s, &sessions, &b, &tb, "storage.get", Value::Null);
        assert_eq!(vb["selectedId"], "one");
        assert_eq!(vb["sort"], "desc");
        assert_eq!(record(&s.db.lock().unwrap(), id).unwrap().revision, 1);
    }
}
