//! 三款AI生成工具的固定作业网关；不开放任意HTTP或系统命令。
use crate::{
    ai_credentials::{self, Profile},
    ai_protocol::{self, Task},
    ai_transport, business,
    network::NetworkConfig,
    store::{err, now_ms, AppResult, Store},
    widgets, Core,
};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
use tauri::Manager;
use tokio_util::sync::CancellationToken;
#[derive(Default)]
pub(crate) struct Service {
    jobs: Mutex<HashMap<String, Job>>,
    tools: Mutex<crate::clipboard_tools::Book>,
    stopped: AtomicBool,
}
pub(crate) struct Work {
    pub instance: String,
    scope: String,
    pub request: String,
    pub cancel: CancellationToken,
    window: String,
    token: String,
    revision: u64,
    profile: Profile,
    body: Value,
    network: NetworkConfig,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    request_id: String,
    input: Value,
    state: String,
    text: String,
    error: Option<String>,
    result_available: bool,
    updated_at: u64,
}
struct Job {
    snapshot: Snapshot,
    cancel: CancellationToken,
    token: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum RecordedState {
    Reserved,
    Completed,
    Cancelled,
    Failed,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: String,
    fingerprint: String,
    state: RecordedState,
    at: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    version: u32,
    entries: Vec<Receipt>,
}
fn ledger_key(instance: &str) -> String {
    format!("ai.requests/{instance}")
}
fn load_ledger(db: &Connection, instance: &str) -> AppResult<Ledger> {
    let raw: Option<String> = db
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [ledger_key(instance)],
            |row| row.get(0),
        )
        .optional()
        .map_err(err)?;
    let Some(raw) = raw else {
        return Ok(Ledger {
            version: 1,
            entries: Vec::new(),
        });
    };
    if raw.len() > 32768 {
        return Err("DATA_UNAVAILABLE: AI请求记录超限，未覆盖".into());
    }
    let ledger: Ledger =
        serde_json::from_str(&raw).map_err(|_| "DATA_UNAVAILABLE: AI请求记录损坏，未重新发送")?;
    let mut ids = std::collections::HashSet::new();
    if ledger.version != 1
        || ledger.entries.len() > 64
        || ledger.entries.iter().any(|entry| {
            uuid::Uuid::parse_str(&entry.id).is_err()
                || entry.fingerprint.len() != 64
                || !entry
                    .fingerprint
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
                || !ids.insert(entry.id.as_str())
        })
    {
        return Err("DATA_UNAVAILABLE: AI请求记录无效，未重新发送".into());
    }
    Ok(ledger)
}
fn save_ledger(db: &Connection, instance: &str, ledger: &Ledger, insert: bool) -> AppResult<()> {
    let raw = serde_json::to_string(ledger).map_err(err)?;
    if raw.len() > 32768 {
        return Err("LIMIT: AI请求记录超限".into());
    }
    if insert {
        db.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![ledger_key(instance),raw]).map_err(err)?;
    } else {
        db.execute(
            "UPDATE app_settings SET value=?2 WHERE key=?1",
            rusqlite::params![ledger_key(instance), raw],
        )
        .map_err(err)?;
    }
    Ok(())
}
fn recorded_state(store: &Store, instance: &str, id: &str, state: RecordedState) -> AppResult<()> {
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    let mut ledger = load_ledger(&tx, instance)?;
    if let Some(entry) = ledger.entries.iter_mut().find(|entry| entry.id == id) {
        entry.state = state;
        entry.at = now_ms();
        save_ledger(&tx, instance, &ledger, false)?;
    }
    tx.commit().map_err(err)
}
fn fixed_task(store: &Store, instance: &str) -> AppResult<Task> {
    let package: String = store
        .db
        .lock()
        .map_err(err)?
        .query_row(
            "SELECT package_id FROM widget_instances WHERE id=?1",
            [instance],
            |row| row.get(0),
        )
        .map_err(err)?;
    match package.as_str() {
        "local.souldesk.widgetjs.ai-mindmap" => Ok(Task::Mindmap),
        "local.souldesk.widgetjs.ai-mermaid" => Ok(Task::Mermaid),
        "local.souldesk.widgetjs.ai-translate" => Ok(Task::Translate),
        crate::clipboard_host::SEARCH => Ok(Task::Translate),
        _ => Err("CAPABILITY_DENIED: 仅固定AI生成组件可调用".into()),
    }
}
fn request_id(value: &str) -> AppResult<String> {
    uuid::Uuid::parse_str(value)
        .map(|id| id.to_string())
        .map_err(|_| "INVALID_ARGUMENT: AI请求标识须为UUID".into())
}
fn replay(entry: &Receipt) -> Value {
    json!({"requestId":entry.id,"state":match entry.state{RecordedState::Reserved=>"interrupted",RecordedState::Completed=>"completed",RecordedState::Cancelled=>"cancelled",RecordedState::Failed=>"failed"},"text":"","resultAvailable":false,"error":"此请求已记录，未重复发送；结果不在当前进程缓存，请查看已保存历史或显式发起新请求","updatedAt":entry.at})
}
impl std::fmt::Debug for Work {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AIWork")
            .field("instance", &self.instance)
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}
impl Service {
    // 调用方持有原生动作锁和有效剪切板会话；这里只创建有限的内存授权。
    pub(crate) fn prepare_tool(
        &self,
        store: &Store,
        instance: &str,
        kind: crate::clipboard_tools::Kind,
        content: &str,
    ) -> AppResult<crate::clipboard_tools::Tool> {
        let package: String = store
            .db
            .lock()
            .map_err(err)?
            .query_row(
                "SELECT package_id FROM widget_instances WHERE id=?1",
                [instance],
                |r| r.get(0),
            )
            .map_err(err)?;
        if package != crate::clipboard_host::SEARCH {
            return Err("CAPABILITY_DENIED: 仅AI剪切板可创建固定工具".into());
        }
        if self.stopped.load(Ordering::Acquire) {
            return Err("SESSION_EXPIRED: AI后台已退出".into());
        }
        let config = crate::clipboard_history::status(store, instance)?;
        let tool = self.tools.lock().map_err(err)?.begin(
            instance,
            kind,
            content,
            config["configRevision"]
                .as_u64()
                .ok_or("DATA_UNAVAILABLE: 剪切板配置无效")?,
        )?;
        self.cancel_current(
            store,
            &kind.scope(instance),
            None,
            "同类工具已重新打开，旧任务已停止",
        )?;
        Ok(tool)
    }
    pub(crate) fn tool(
        &self,
        store: &Store,
        label: &str,
        instance: &str,
    ) -> AppResult<crate::clipboard_tools::Tool> {
        let tool = self.tools.lock().map_err(err)?.get(label, instance)?;
        if self.stopped.load(Ordering::Acquire)
            || store.preference("hidden_all").as_deref() == Some("true")
            || crate::clipboard_history::status(store, instance)?["configRevision"]
                != tool.config_revision
        {
            return Err("PRESENTATION_EXPIRED: 工具窗口配置已变化或已隐藏".into());
        }
        Ok(tool)
    }
    pub(crate) fn current_tool(&self, store: &Store, label: &str, instance: &str) -> bool {
        self.tool(store, label, instance).is_ok()
    }
    pub(crate) fn close_tool(&self, store: &Store, label: &str) -> AppResult<()> {
        let _native = store.native_actions.lock().map_err(err)?;
        let tool = self
            .tools
            .lock()
            .map_err(err)?
            .entries
            .values()
            .find(|p| p.label == label)
            .cloned();
        self.tools.lock().map_err(err)?.close(label);
        if let Some(p) = tool {
            self.cancel_current(
                store,
                &p.kind.scope(&p.instance),
                None,
                "工具窗口已关闭，已保留部分结果",
            )?;
        }
        Ok(())
    }
    pub(crate) fn reconcile_tools(&self, store: &Store) -> AppResult<Vec<String>> {
        let entries = self
            .tools
            .lock()
            .map_err(err)?
            .entries
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for p in entries {
            if !self.current_tool(store, &p.label, &p.instance) {
                self.close_tool(store, &p.label)?;
                self.tools.lock().map_err(err)?.closing.push(p.label);
            }
        }
        Ok(std::mem::take(&mut self.tools.lock().map_err(err)?.closing))
    }
    fn task_scope(
        &self,
        store: &Store,
        instance: &str,
        window: &str,
        method: &str,
    ) -> AppResult<(Task, String)> {
        let task = fixed_task(store, instance)?;
        if let Some(kind) = crate::clipboard_tools::role(window, instance) {
            let tool = self.tool(store, window, instance)?;
            return Ok((tool.kind.task().unwrap_or(task), kind.scope(instance)));
        }
        let package: String = store
            .db
            .lock()
            .map_err(err)?
            .query_row(
                "SELECT package_id FROM widget_instances WHERE id=?1",
                [instance],
                |r| r.get(0),
            )
            .map_err(err)?;
        if package == crate::clipboard_host::SEARCH
            && !matches!(method, "ai.status" | "ai.configure" | "ai.clear")
        {
            return Err("CAPABILITY_DENIED: 请从固定导图或翻译窗口操作".into());
        }
        Ok((task, instance.into()))
    }
    pub fn access(
        &self,
        store: &Store,
        sessions: &widgets::Sessions,
        window: &str,
        token: &str,
        method: &str,
        args: Value,
    ) -> AppResult<(Value, Option<Work>)> {
        crate::keyboard_host::overlay_method(window, method)?;
        crate::reminder_host::surface_method(window, method)?;
        crate::clipboard_surface::surface_method(window, method)?;
        let _native = store.native_actions.lock().map_err(err)?;
        if self.stopped.load(Ordering::Acquire) {
            return Err("SESSION_EXPIRED: AI生成后台已退出".into());
        }
        widgets::with_capability(store, sessions, window, token, "ai.generate", |session| {
            let (task, scope) = self.task_scope(store, &session.instance, window, method)?;
            match method {
                "ai.toolInfo" => {
                    if !args.is_null() {
                        return Err("INVALID_ARGUMENT: 工具信息不接受参数".into());
                    }
                    let p = self.tool(store, window, &session.instance)?;
                    Ok((
                        json!({"kind":p.kind.name(),"content":p.content,"presentationId":p.id}),
                        None,
                    ))
                }
                "ai.document.get" | "ai.document.set" => {
                    let p = self.tool(store, window, &session.instance)?;
                    let mut db = store.db.lock().map_err(err)?;
                    let result = if method == "ai.document.get" {
                        if !args.is_null() {
                            return Err("INVALID_ARGUMENT: 文档读取不接受参数".into());
                        }
                        crate::clipboard_tools::document_get(&db, &session.instance, p.kind)?
                    } else {
                        #[derive(Deserialize)]
                        #[serde(deny_unknown_fields)]
                        struct Document {
                            document: Value,
                        }
                        let value: Document = serde_json::from_value(args)
                            .map_err(|_| "INVALID_ARGUMENT: 工具保存只接受document")?;
                        let tx = db.transaction().map_err(err)?;
                        let result = crate::clipboard_tools::document_set(
                            &tx,
                            &session.instance,
                            p.kind,
                            value.document,
                        )?;
                        tx.commit().map_err(err)?;
                        result
                    };
                    Ok((result, None))
                }
                "ai.status" => {
                    if !args.is_null() && args.as_object().is_none_or(|args| !args.is_empty()) {
                        return Err("INVALID_ARGUMENT: AI状态不接受参数".into());
                    }
                    Ok((self.status(store, &session.instance, &scope)?, None))
                }
                "ai.configure" | "ai.clear" => {
                    let before = ai_credentials::status(store, &session.instance)?;
                    let after = if method == "ai.configure" {
                        ai_credentials::configure(store, &session.instance, args)?
                    } else {
                        ai_credentials::clear(store, &session.instance, args)?
                    };
                    if before["revision"] != after["revision"] {
                        self.cancel_instance(
                            store,
                            &session.instance,
                            "AI配置已变更，旧任务已取消",
                        )?;
                    }
                    Ok((self.status(store, &session.instance, &scope)?, None))
                }
                "ai.begin" => {
                    #[derive(Deserialize)]
                    #[serde(rename_all = "camelCase", deny_unknown_fields)]
                    struct Begin {
                        request_id: String,
                        config_revision: u64,
                        input: Value,
                    }
                    let input: Begin = serde_json::from_value(args)
                        .map_err(|_| "INVALID_ARGUMENT: AI生成参数无效")?;
                    let id = request_id(&input.request_id)?;
                    let (revision, profile) = ai_credentials::profile(store, &session.instance)?;
                    if revision != input.config_revision {
                        return Err("REVISION_CONFLICT: AI连接配置已更新，请重新读取".into());
                    }
                    let profile = profile
                        .filter(|profile| profile.enabled)
                        .ok_or("CONFIG_REQUIRED: 请先保存自己的模型连接并显式启用")?;
                    let body =
                        ai_protocol::request_body(task, &profile.model, input.input.clone())?;
                    let fingerprint = format!("{:x}", Sha256::digest(format!("{revision}:{body}")));
                    // 网络配置先读取；不得持有SQLite锁再次调用Store.preference。
                    let network: Value = serde_json::from_str(
                        &store
                            .preference("network")
                            .unwrap_or("{\"mode\":\"system\"}".into()),
                    )
                    .map_err(err)?;
                    let network = business::resolve_network(&network)?;
                    let mut db = store.db.lock().map_err(err)?;
                    let tx = db.transaction().map_err(err)?;
                    let mut ledger = load_ledger(&tx, &scope)?;
                    if let Some(entry) = ledger.entries.iter().find(|entry| entry.id == id) {
                        if entry.fingerprint != fingerprint {
                            return Err(
                                "REQUEST_CONFLICT: 同一AI请求标识不能用于不同内容或配置".into()
                            );
                        }
                        let jobs = self.jobs.lock().map_err(err)?;
                        let value = jobs
                            .get(&scope)
                            .filter(|job| job.snapshot.request_id == id)
                            .map(|job| serde_json::to_value(&job.snapshot).map_err(err))
                            .transpose()?
                            .unwrap_or_else(|| replay(entry));
                        return Ok((value, None));
                    }
                    {
                        let jobs = self.jobs.lock().map_err(err)?;
                        if !jobs.contains_key(&scope) && jobs.len() >= 128 {
                            return Err("LIMIT: AI缓存最多128个实例，请先关闭不再使用的实例".into());
                        }
                        if jobs
                            .iter()
                            .filter(|(owner, job)| {
                                *owner != &scope && job.snapshot.state == "running"
                            })
                            .count()
                            >= 4
                        {
                            return Err("LIMIT: AI同时最多运行4个任务".into());
                        }
                    }
                    // 先提交请求槽；数据库失败时不能停止旧任务或发出新付费请求。
                    for entry in &mut ledger.entries {
                        if matches!(entry.state, RecordedState::Reserved) {
                            entry.state = RecordedState::Cancelled;
                        }
                    }
                    ledger.entries.push(Receipt {
                        id: id.clone(),
                        fingerprint,
                        state: RecordedState::Reserved,
                        at: now_ms(),
                    });
                    if ledger.entries.len() > 64 {
                        ledger.entries.remove(0);
                    }
                    save_ledger(&tx, &scope, &ledger, true)?;
                    tx.commit().map_err(err)?;
                    drop(db);
                    let cancel = CancellationToken::new();
                    let snapshot = Snapshot {
                        request_id: id.clone(),
                        input: input.input,
                        state: "running".into(),
                        text: String::new(),
                        error: None,
                        result_available: false,
                        updated_at: now_ms(),
                    };
                    let mut jobs = self.jobs.lock().map_err(err)?;
                    if self.stopped.load(Ordering::Acquire) {
                        return Err("SESSION_EXPIRED: AI生成后台已退出".into());
                    }
                    if let Some(old) = jobs.get(&scope) {
                        old.cancel.cancel();
                    }
                    jobs.insert(
                        scope.clone(),
                        Job {
                            snapshot: snapshot.clone(),
                            cancel: cancel.clone(),
                            token: token.into(),
                        },
                    );
                    Ok((
                        serde_json::to_value(snapshot).map_err(err)?,
                        Some(Work {
                            instance: session.instance.clone(),
                            scope,
                            request: id,
                            cancel,
                            window: window.into(),
                            token: token.into(),
                            revision,
                            profile,
                            body,
                            network,
                        }),
                    ))
                }
                "ai.poll" | "ai.cancel" => {
                    #[derive(Deserialize)]
                    #[serde(rename_all = "camelCase", deny_unknown_fields)]
                    struct Id {
                        request_id: String,
                    }
                    let args: Id = serde_json::from_value(args)
                        .map_err(|_| "INVALID_ARGUMENT: AI任务读取/取消参数无效")?;
                    let id = request_id(&args.request_id)?;
                    let ledger = load_ledger(&*store.db.lock().map_err(err)?, &scope)?;
                    let entry = ledger
                        .entries
                        .iter()
                        .find(|entry| entry.id == id)
                        .ok_or("SESSION_EXPIRED: 当前实例没有此AI请求")?;
                    if method == "ai.cancel" {
                        self.cancel_current(
                            store,
                            &scope,
                            Some(&id),
                            "用户已取消，已保留部分结果",
                        )?;
                    }
                    let jobs = self.jobs.lock().map_err(err)?;
                    let value = jobs
                        .get(&scope)
                        .filter(|job| job.snapshot.request_id == id)
                        .map(|job| serde_json::to_value(&job.snapshot).map_err(err))
                        .transpose()?
                        .unwrap_or_else(|| replay(entry));
                    Ok((value, None))
                }
                _ => Err("CAPABILITY_DENIED: 未开放的AI方法".into()),
            }
        })
    }
    fn status(&self, store: &Store, instance: &str, scope: &str) -> AppResult<Value> {
        let mut status = ai_credentials::status(store, instance)?;
        let current = self
            .jobs
            .lock()
            .map_err(err)?
            .get(scope)
            .map(|job| serde_json::to_value(&job.snapshot).map_err(err))
            .transpose()?;
        status["job"] = if let Some(current) = current {
            current
        } else {
            match load_ledger(&*store.db.lock().map_err(err)?, scope) {
                Ok(ledger) => ledger.entries.last().map(replay).unwrap_or(Value::Null),
                Err(error) => {
                    json!({"requestId":Value::Null,"state":"failed","text":"","resultAvailable":false,"error":error,"updatedAt":now_ms()})
                }
            }
        };
        Ok(status)
    }
    fn cancel_current(
        &self,
        store: &Store,
        instance: &str,
        id: Option<&str>,
        reason: &str,
    ) -> AppResult<()> {
        let request = {
            let mut jobs = self.jobs.lock().map_err(err)?;
            let Some(job) = jobs.get_mut(instance).filter(|job| {
                id.is_none_or(|id| job.snapshot.request_id == id) && job.snapshot.state == "running"
            }) else {
                return Ok(());
            };
            job.cancel.cancel();
            job.snapshot.state = "cancelled".into();
            job.snapshot.error = Some(reason.into());
            job.snapshot.updated_at = now_ms();
            job.snapshot.request_id.clone()
        };
        if let Err(error) = recorded_state(store, instance, &request, RecordedState::Cancelled) {
            if let Some(job) = self
                .jobs
                .lock()
                .map_err(err)?
                .get_mut(instance)
                .filter(|job| job.snapshot.request_id == request)
            {
                job.snapshot.error = Some(format!("任务已取消，但停止状态未保存：{error}"));
            }
        }
        Ok(())
    }
    pub fn cancel_instance(&self, store: &Store, instance: &str, reason: &str) -> AppResult<()> {
        for key in [
            instance.to_owned(),
            crate::clipboard_tools::Kind::Mindmap.scope(instance),
            crate::clipboard_tools::Kind::Translate.scope(instance),
        ] {
            self.cancel_current(store, &key, None, reason)?;
        }
        Ok(())
    }
    pub fn cancel_session(&self, store: &Store, token: &str) -> AppResult<()> {
        let owners = self
            .jobs
            .lock()
            .map_err(err)?
            .iter()
            .filter(|(_, job)| job.token == token)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for instance in owners {
            self.cancel_current(store, &instance, None, "组件会话已关闭，已保留部分结果")?;
        }
        Ok(())
    }
    pub fn forget(&self, instance: &str) -> AppResult<()> {
        self.tools.lock().map_err(err)?.forget(instance);
        self.jobs.lock().map_err(err)?.retain(|key, job| {
            if key == instance || key.starts_with(&format!("{instance}/")) {
                job.cancel.cancel();
                false
            } else {
                true
            }
        });
        Ok(())
    }
    fn with_live<T>(
        &self,
        store: &Store,
        sessions: &widgets::Sessions,
        work: &Work,
        action: impl FnOnce() -> AppResult<T>,
    ) -> AppResult<T> {
        if self.stopped.load(Ordering::Acquire) || work.cancel.is_cancelled() {
            return Err("SESSION_EXPIRED: AI任务已取消或退出".into());
        }
        widgets::with_capability(
            store,
            sessions,
            &work.window,
            &work.token,
            "ai.generate",
            |session| {
                if session.instance != work.instance {
                    return Err("SESSION_EXPIRED: AI来源已变化".into());
                }
                let (_, scope) = self
                    .task_scope(store, &work.instance, &work.window, "ai.begin")
                    .map_err(|error| format!("SESSION_EXPIRED: {error}"))?;
                if scope != work.scope {
                    return Err("SESSION_EXPIRED: AI任务范围已变化".into());
                }
                let (revision, profile) = ai_credentials::profile(store, &work.instance)?;
                if revision != work.revision || !profile.is_some_and(|profile| profile.enabled) {
                    return Err("SESSION_EXPIRED: AI连接配置已变化".into());
                }
                {
                    let jobs = self.jobs.lock().map_err(err)?;
                    if jobs.get(&work.scope).is_none_or(|job| {
                        job.snapshot.request_id != work.request || job.snapshot.state != "running"
                    }) {
                        return Err("SESSION_EXPIRED: AI任务已被替换或关闭".into());
                    }
                }
                // 会话锁跨越结果更新，避免关闭视图穿过鉴权与提交的间隙。
                action()
            },
        )
    }
    fn live_locked(
        &self,
        store: &Store,
        sessions: &widgets::Sessions,
        work: &Work,
    ) -> AppResult<()> {
        self.with_live(store, sessions, work, || Ok(()))
    }
    pub fn live(&self, store: &Store, sessions: &widgets::Sessions, work: &Work) -> AppResult<()> {
        let _native = store.native_actions.lock().map_err(err)?;
        let result = self.live_locked(store, sessions, work);
        if let Err(error) = &result {
            self.cancel_current(store, &work.scope, Some(&work.request), error)?;
        }
        result
    }
    pub fn push(
        &self,
        store: &Store,
        sessions: &widgets::Sessions,
        work: &Work,
        text: &str,
    ) -> AppResult<()> {
        let _native = store.native_actions.lock().map_err(err)?;
        let result = self.with_live(store, sessions, work, || {
            if text.len() > 512 * 1024 {
                return Err("LIMIT: AI结果超过512KiB".into());
            }
            let mut jobs = self.jobs.lock().map_err(err)?;
            let job = jobs
                .get_mut(&work.scope)
                .filter(|job| {
                    job.snapshot.request_id == work.request
                        && job.snapshot.state == "running"
                        && !work.cancel.is_cancelled()
                        && !self.stopped.load(Ordering::Acquire)
                })
                .ok_or("SESSION_EXPIRED: AI任务已移除或取消")?;
            job.snapshot.text = text.into();
            job.snapshot.result_available = !text.is_empty();
            job.snapshot.updated_at = now_ms();
            Ok(())
        });
        if let Err(error) = &result {
            if error.starts_with("SESSION_EXPIRED:") {
                self.cancel_current(store, &work.scope, Some(&work.request), error)?;
            }
        }
        result
    }
    pub fn finish(
        &self,
        store: &Store,
        sessions: &widgets::Sessions,
        work: &Work,
        result: AppResult<String>,
    ) -> AppResult<()> {
        let _native = store.native_actions.lock().map_err(err)?;
        let result = self.with_live(store, sessions, work, || {
            let (state, text, error) = match result {
                Ok(text) if !text.trim().is_empty() && text.len() <= 512 * 1024 => {
                    (RecordedState::Completed, Some(text), None)
                }
                Ok(_) => (
                    RecordedState::Failed,
                    None,
                    Some("DATA_UNAVAILABLE: AI没有返回有效完整文本".into()),
                ),
                Err(error) => (
                    if error == "CANCELLED" {
                        RecordedState::Cancelled
                    } else {
                        RecordedState::Failed
                    },
                    None,
                    Some(error),
                ),
            };
            let mut jobs = self.jobs.lock().map_err(err)?;
            let job = jobs
                .get_mut(&work.scope)
                .filter(|job| {
                    job.snapshot.request_id == work.request
                        && job.snapshot.state == "running"
                        && !work.cancel.is_cancelled()
                        && !self.stopped.load(Ordering::Acquire)
                })
                .ok_or("SESSION_EXPIRED: AI任务已移除或取消")?;
            let saved = recorded_state(store, &work.scope, &work.request, state.clone());
            if let Some(text) = text {
                job.snapshot.text = text;
                job.snapshot.result_available = true;
            }
            job.snapshot.state = match state {
                RecordedState::Completed => "completed",
                RecordedState::Cancelled => "cancelled",
                _ => "failed",
            }
            .into();
            job.snapshot.error = error.clone();
            job.snapshot.updated_at = now_ms();
            if let Err(error) = saved {
                job.snapshot.state = "failed".into();
                job.snapshot.error = Some(format!("AI结果状态未保存：{error}"));
                return Err(error);
            }
            if let Some(error) = error {
                return Err(error);
            }
            Ok(())
        });
        if let Err(error) = &result {
            if error.starts_with("SESSION_EXPIRED:") {
                self.cancel_current(store, &work.scope, Some(&work.request), error)?;
            }
        }
        result
    }
    pub async fn execute(
        &self,
        store: &Store,
        sessions: &widgets::Sessions,
        work: Work,
    ) -> AppResult<()> {
        self.live(store, sessions, &work)?;
        let url = ai_protocol::endpoint(&work.profile.base_url)?;
        let operation = ai_transport::generate(
            &work.network,
            url,
            &work.profile.key,
            work.body.clone(),
            &work.cancel,
            |text| self.push(store, sessions, &work, text),
        );
        tokio::pin!(operation);
        let mut heartbeat = tokio::time::interval(Duration::from_millis(250));
        let result = loop {
            tokio::select! {biased;_=work.cancel.cancelled()=>break Err("CANCELLED".into()),result=&mut operation=>break result,_=heartbeat.tick()=>{if let Err(error)=self.live(store,sessions,&work){break Err(error);}}}
        };
        self.finish(store, sessions, &work, result)
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(mut jobs) = self.jobs.lock() {
            for job in jobs.values_mut() {
                job.cancel.cancel();
                if job.snapshot.state == "running" {
                    job.snapshot.state = "cancelled".into();
                    job.snapshot.error = Some("AI生成后台已退出".into());
                }
            }
        }
    }
}
pub(crate) fn remove_data(db: &Connection, instance: &str) -> AppResult<()> {
    ai_credentials::remove_data(db, instance)?;
    crate::clipboard_tools::remove_data(db, instance)?;
    for kind in [
        crate::clipboard_tools::Kind::Mindmap,
        crate::clipboard_tools::Kind::Translate,
    ] {
        db.execute(
            "DELETE FROM app_settings WHERE key=?1",
            [ledger_key(&kind.scope(instance))],
        )
        .map_err(err)?;
    }
    db.execute(
        "DELETE FROM app_settings WHERE key=?1",
        [ledger_key(instance)],
    )
    .map_err(err)?;
    Ok(())
}
pub(crate) async fn gateway(
    app: &tauri::AppHandle,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    let handle = app.clone();
    let (window, token, method) = (window.to_owned(), token.to_owned(), method.to_owned());
    let (value, work) = tauri::async_runtime::spawn_blocking(move || {
        let core = handle.state::<Core>();
        core.ai
            .access(&core.store, &core.sessions, &window, &token, &method, args)
    })
    .await
    .map_err(err)??;
    if let Some(work) = work {
        let handle = app.clone();
        handle
            .state::<Core>()
            .active_tasks
            .fetch_add(1, Ordering::AcqRel);
        tauri::async_runtime::spawn(async move {
            struct Release(tauri::AppHandle);
            impl Drop for Release {
                fn drop(&mut self) {
                    self.0
                        .state::<Core>()
                        .active_tasks
                        .fetch_sub(1, Ordering::AcqRel);
                }
            }
            let _release = Release(handle.clone());
            let core = handle.state::<Core>();
            let _ = core.ai.execute(&core.store, &core.sessions, work).await;
        });
    }
    Ok(value)
}
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::ai_credentials;
    use serde_json::json;
    use uuid::Uuid;
    const PACKAGE: &str = "local.souldesk.widgetjs.ai-translate";
    fn fixture() -> (Store, widgets::Sessions, Service) {
        let store =
            Store::open(std::env::temp_dir().join(format!("souldesk-ai-host-{}", Uuid::new_v4())))
                .unwrap();
        store
            .save_preference("network", "{\"mode\":\"direct\"}")
            .unwrap();
        (store, widgets::Sessions::default(), Service::default())
    }
    fn add(
        store: &Store,
        sessions: &widgets::Sessions,
        package: &str,
        capability: &str,
    ) -> (String, String, String) {
        let manifest = json!({"schemaVersion":1,"id":package,"name":"AI固定作业检查","version":"0.1.0","hostApiMajor":1,"entry":"dist/index.html","defaultSize":{"width":600,"height":450},"minSize":{"width":300,"height":200},"capabilities":[capability],"settingsSchema":{"type":"object","additionalProperties":false,"properties":{}}});
        store.db.lock().unwrap().execute("INSERT OR IGNORE INTO widget_packages(id,manifest,directory,digest) VALUES(?1,?2,'ai-test','ai-test')",rusqlite::params![package,manifest.to_string()]).unwrap();
        let instance = widgets::create_instance(store, package).unwrap();
        let window = format!("float-{instance}");
        let session =
            widgets::open_display_session(store, sessions, &window, &instance, None).unwrap();
        (instance, window, session["token"].as_str().unwrap().into())
    }
    fn configure(
        service: &Service,
        store: &Store,
        sessions: &widgets::Sessions,
        window: &str,
        token: &str,
        revision: u64,
        enabled: bool,
    ) -> Value {
        service.access(store,sessions,window,token,"ai.configure",json!({"requestId":Uuid::new_v4().to_string(),"expectedRevision":revision,"apiBaseUrl":"https://api.example.com/v1","apiKey":"fixture-key","model":"fixture-model","enabled":enabled})).unwrap().0
    }
    fn input(id: &str, revision: u64) -> Value {
        json!({"requestId":id,"configRevision":revision,"input":{"content":"你好","firstLang":"zh","secondLang":"en"}})
    }
    fn begin(
        service: &Service,
        store: &Store,
        sessions: &widgets::Sessions,
        window: &str,
        token: &str,
        id: &str,
    ) -> (Value, Option<Work>) {
        service
            .access(store, sessions, window, token, "ai.begin", input(id, 1))
            .unwrap()
    }
    fn search_tool_source(s: &Store, sessions: &widgets::Sessions) -> (String, String, String) {
        let row = add(s, sessions, crate::clipboard_host::SEARCH, "ai.generate");
        let db = s.db.lock().unwrap();
        let raw: String = db
            .query_row(
                "SELECT manifest FROM widget_packages WHERE id=?1",
                [crate::clipboard_host::SEARCH],
                |r| r.get(0),
            )
            .unwrap();
        let mut m: Value = serde_json::from_str(&raw).unwrap();
        m["capabilities"] = json!([
            "ai.generate",
            "clipboard.manage",
            "documents.user",
            "storage.instance",
            "clipboard.write"
        ]);
        db.execute(
            "UPDATE widget_packages SET manifest=?2 WHERE id=?1",
            rusqlite::params![crate::clipboard_host::SEARCH, m.to_string()],
        )
        .unwrap();
        drop(db);
        row
    }
    fn tool_session(
        service: &Service,
        s: &Store,
        sessions: &widgets::Sessions,
        id: &str,
        kind: crate::clipboard_tools::Kind,
    ) -> (crate::clipboard_tools::Tool, String) {
        let p = service
            .prepare_tool(
                s,
                id,
                kind,
                if kind == crate::clipboard_tools::Kind::Settings {
                    ""
                } else {
                    "预填文本"
                },
            )
            .unwrap();
        let value = widgets::open_display_session(s, sessions, &p.label, id, None).unwrap();
        (p, value["token"].as_str().unwrap().into())
    }
    #[test]
    fn ai_host_clipboard_tools_keep_parallel_jobs_documents_and_sessions_separate() {
        use crate::clipboard_tools::Kind;
        let (s, sessions, service) = fixture();
        let row = search_tool_source(&s, &sessions);
        configure(&service, &s, &sessions, &row.1, &row.2, 0, true);
        assert!(service
            .access(
                &s,
                &sessions,
                &row.1,
                &row.2,
                "ai.begin",
                input(&Uuid::new_v4().to_string(), 1)
            )
            .is_err());
        let (map, mt) = tool_session(&service, &s, &sessions, &row.0, Kind::Mindmap);
        let (trans, tt) = tool_session(&service, &s, &sessions, &row.0, Kind::Translate);
        assert!(sessions.0.lock().unwrap().contains_key(&mt));
        assert!(sessions.0.lock().unwrap().contains_key(&row.2));
        let info = service
            .access(&s, &sessions, &map.label, &mt, "ai.toolInfo", Value::Null)
            .unwrap()
            .0;
        assert_eq!(info["content"], "预填文本");
        assert!(!info.to_string().contains("fixture-key"));
        assert!(service.jobs.lock().unwrap().is_empty());
        let request = Uuid::new_v4().to_string();
        let(_,mw)=service.access(&s,&sessions,&map.label,&mt,"ai.begin",json!({"requestId":request,"configRevision":1,"input":{"content":"导图输入","extra":""}})).unwrap();
        let mw = mw.unwrap();
        let (_, tw) = begin(
            &service,
            &s,
            &sessions,
            &trans.label,
            &tt,
            &Uuid::new_v4().to_string(),
        );
        let tw = tw.unwrap();
        assert!(!mw.cancel.is_cancelled());
        assert_ne!(mw.scope, tw.scope);
        service.push(&s, &sessions, &mw, "# 导图").unwrap();
        service.push(&s, &sessions, &tw, "翻译结果").unwrap();
        for (p, t, text) in [(&map, &mt, "图文档"), (&trans, &tt, "译文档")] {
            service
                .access(
                    &s,
                    &sessions,
                    &p.label,
                    t,
                    "ai.document.set",
                    json!({"document":{"content":text}}),
                )
                .unwrap();
            assert_eq!(
                service
                    .access(&s, &sessions, &p.label, t, "ai.document.get", Value::Null)
                    .unwrap()
                    .0["content"],
                text
            );
        }
        assert!(service
            .access(&s, &sessions, &row.1, &row.2, "ai.status", Value::Null)
            .unwrap()
            .0["job"]
            .is_null());
        service.cancel_session(&s, &mt).unwrap();
        assert!(mw.cancel.is_cancelled());
        assert!(!tw.cancel.is_cancelled());
        service.close_tool(&s, &trans.label).unwrap();
        assert!(tw.cancel.is_cancelled());
    }
    #[test]
    fn ai_host_clipboard_tool_replacement_rejects_old_jobs_and_documents_without_recreating_work() {
        use crate::clipboard_tools::Kind;
        let (s, sessions, service) = fixture();
        let row = search_tool_source(&s, &sessions);
        configure(&service, &s, &sessions, &row.1, &row.2, 0, true);
        let (p, t) = tool_session(&service, &s, &sessions, &row.0, Kind::Translate);
        let id = Uuid::new_v4().to_string();
        let (_, w) = begin(&service, &s, &sessions, &p.label, &t, &id);
        let w = w.unwrap();
        service.push(&s, &sessions, &w, "保留部分").unwrap();
        let newer = service
            .prepare_tool(&s, &row.0, Kind::Translate, "新文本")
            .unwrap();
        assert!(w.cancel.is_cancelled());
        assert!(service
            .access(
                &s,
                &sessions,
                &p.label,
                &t,
                "ai.document.set",
                json!({"document":{"content":"旧窗覆盖"}})
            )
            .is_err());
        assert!(service
            .finish(&s, &sessions, &w, Ok("迟到完成".into()))
            .is_err());
        let value =
            widgets::open_display_session(&s, &sessions, &newer.label, &row.0, None).unwrap();
        let token = value["token"].as_str().unwrap();
        let result = service
            .access(
                &s,
                &sessions,
                &newer.label,
                token,
                "ai.begin",
                input(&id, 1),
            )
            .unwrap();
        assert!(result.1.is_none());
        assert_eq!(result.0["state"], "cancelled");
        let (config, ct) = tool_session(&service, &s, &sessions, &row.0, Kind::Settings);
        assert!(service
            .access(
                &s,
                &sessions,
                &config.label,
                &ct,
                "ai.begin",
                input(&Uuid::new_v4().to_string(), 1)
            )
            .is_err());
        assert!(service
            .access(
                &s,
                &sessions,
                &config.label,
                &ct,
                "ai.document.get",
                Value::Null
            )
            .is_err());
    }
    #[test]
    fn ai_host_clipboard_config_change_cancels_old_tool_and_late_document_save() {
        use crate::clipboard_tools::Kind;
        let (s, sessions, service) = fixture();
        let row = search_tool_source(&s, &sessions);
        configure(&service, &s, &sessions, &row.1, &row.2, 0, true);
        let (p, t) = tool_session(&service, &s, &sessions, &row.0, Kind::Translate);
        let (_, w) = begin(
            &service,
            &s,
            &sessions,
            &p.label,
            &t,
            &Uuid::new_v4().to_string(),
        );
        let w = w.unwrap();
        crate::clipboard_history::mutate(&s,&row.0,json!({"requestId":Uuid::new_v4().to_string(),"command":{"type":"configure","expectedRevision":0,"captureEnabled":false,"shortcutEnabled":false,"pasteEnabled":false,"shortcut":"Super+Alt+S","historyExpiredHours":24}})).unwrap();
        assert!(service.live(&s, &sessions, &w).is_err());
        assert!(w.cancel.is_cancelled());
        assert!(service
            .access(
                &s,
                &sessions,
                &p.label,
                &t,
                "ai.document.set",
                json!({"document":{}})
            )
            .is_err());
        assert!(service.reconcile_tools(&s).unwrap().contains(&p.label));
    }
    #[test]
    fn ai_host_clipboard_delete_removes_tool_documents_and_scoped_receipts_only() {
        use crate::clipboard_tools::Kind;
        let (s, sessions, service) = fixture();
        let row = search_tool_source(&s, &sessions);
        configure(&service, &s, &sessions, &row.1, &row.2, 0, true);
        let (p, t) = tool_session(&service, &s, &sessions, &row.0, Kind::Translate);
        let (_, w) = begin(
            &service,
            &s,
            &sessions,
            &p.label,
            &t,
            &Uuid::new_v4().to_string(),
        );
        let w = w.unwrap();
        service
            .access(
                &s,
                &sessions,
                &p.label,
                &t,
                "ai.document.set",
                json!({"document":{"content":"待删除"}}),
            )
            .unwrap();
        let other = Uuid::new_v4().to_string();
        s.save_preference(&ledger_key(&Kind::Translate.scope(&other)), "keep-other")
            .unwrap();
        widgets::remove_instance(&s, &sessions, &row.0).unwrap();
        service.forget(&row.0).unwrap();
        assert!(w.cancel.is_cancelled());
        assert!(s.preference(&ledger_key(&w.scope)).is_none());
        assert!(service
            .finish(&s, &sessions, &w, Ok("不能复活".into()))
            .is_err());
        assert_eq!(
            s.preference(&ledger_key(&Kind::Translate.scope(&other)))
                .as_deref(),
            Some("keep-other")
        );
        assert_eq!(
            crate::clipboard_tools::document_get(&s.db.lock().unwrap(), &row.0, Kind::Translate)
                .unwrap(),
            json!({})
        );
    }
    #[test]
    fn ai_host_requires_fixed_package_capability_current_window_and_explicit_enabled_configuration()
    {
        let (s, sessions, service) = fixture();
        let (instance, window, token) = add(&s, &sessions, PACKAGE, "ai.generate");
        let status = service
            .access(&s, &sessions, &window, &token, "ai.status", Value::Null)
            .unwrap()
            .0;
        assert_eq!(status["enabled"], false);
        assert!(service
            .access(
                &s,
                &sessions,
                &window,
                &token,
                "ai.begin",
                input(&Uuid::new_v4().to_string(), 0)
            )
            .is_err());
        let (_, foreign, foreign_token) =
            add(&s, &sessions, "local.test.foreign-ai", "ai.generate");
        assert!(service
            .access(
                &s,
                &sessions,
                &foreign,
                &foreign_token,
                "ai.status",
                Value::Null
            )
            .unwrap_err()
            .contains("固定"));
        let (_, no_cap, no_cap_token) = add(
            &s,
            &sessions,
            "local.souldesk.widgetjs.ai-mindmap",
            "storage.instance",
        );
        assert!(service
            .access(
                &s,
                &sessions,
                &no_cap,
                &no_cap_token,
                "ai.status",
                Value::Null
            )
            .unwrap_err()
            .contains("未声明"));
        assert!(service
            .access(
                &s,
                &sessions,
                "other-window",
                &token,
                "ai.status",
                Value::Null
            )
            .is_err());
        configure(&service, &s, &sessions, &window, &token, 0, true);
        let (value, work) = begin(
            &service,
            &s,
            &sessions,
            &window,
            &token,
            &Uuid::new_v4().to_string(),
        );
        assert_eq!(value["state"], "running");
        assert_eq!(work.unwrap().instance, instance);
    }
    #[test]
    fn ai_host_uuid_receipt_survives_restart_and_retries_never_create_second_work() {
        let (s, sessions, service) = fixture();
        let (_, window, token) = add(&s, &sessions, PACKAGE, "ai.generate");
        configure(&service, &s, &sessions, &window, &token, 0, true);
        let id = Uuid::new_v4().to_string();
        let (first, work) = begin(&service, &s, &sessions, &window, &token, &id);
        assert!(work.is_some());
        let (retry, work) = begin(&service, &s, &sessions, &window, &token, &id);
        assert_eq!(first, retry);
        assert!(work.is_none());
        let restarted = Service::default();
        let (retry, work) = begin(&restarted, &s, &sessions, &window, &token, &id);
        assert!(work.is_none());
        assert_eq!(retry["state"], "interrupted");
        assert_eq!(retry["resultAvailable"], false);
        let mut conflict = input(&id, 1);
        conflict["input"]["content"] = json!("另一个内容");
        assert!(service
            .access(&s, &sessions, &window, &token, "ai.begin", conflict)
            .unwrap_err()
            .contains("REQUEST_CONFLICT"));
    }
    #[test]
    fn ai_host_reservation_failure_cannot_cancel_old_job_or_create_billable_work() {
        let (s, sessions, service) = fixture();
        let (instance, window, token) = add(&s, &sessions, PACKAGE, "ai.generate");
        configure(&service, &s, &sessions, &window, &token, 0, true);
        let old_id = Uuid::new_v4().to_string();
        let (_, old) = begin(&service, &s, &sessions, &window, &token, &old_id);
        let old = old.unwrap();
        s.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_ai_request BEFORE UPDATE ON app_settings WHEN new.key LIKE 'ai.requests/%' BEGIN SELECT RAISE(FAIL,'request denied'); END;").unwrap();
        assert!(service
            .access(
                &s,
                &sessions,
                &window,
                &token,
                "ai.begin",
                input(&Uuid::new_v4().to_string(), 1)
            )
            .is_err());
        assert!(!old.cancel.is_cancelled());
        service.push(&s, &sessions, &old, "仍然有效").unwrap();
        let polled = service
            .access(
                &s,
                &sessions,
                &window,
                &token,
                "ai.poll",
                json!({"requestId":old_id}),
            )
            .unwrap()
            .0;
        assert_eq!(polled["text"], "仍然有效");
        assert_eq!(
            ai_credentials::status(&s, &instance).unwrap()["revision"],
            1
        );
    }
    #[test]
    fn ai_host_replacement_and_cancel_preserve_partial_text_without_affecting_other_instance() {
        let (s, sessions, service) = fixture();
        let (_, a, at) = add(&s, &sessions, PACKAGE, "ai.generate");
        let (_, b, bt) = add(&s, &sessions, PACKAGE, "ai.generate");
        for (window, token) in [(&a, &at), (&b, &bt)] {
            configure(&service, &s, &sessions, window, token, 0, true);
        }
        let old_id = Uuid::new_v4().to_string();
        let (_, old) = begin(&service, &s, &sessions, &a, &at, &old_id);
        let old = old.unwrap();
        let (_, other) = begin(
            &service,
            &s,
            &sessions,
            &b,
            &bt,
            &Uuid::new_v4().to_string(),
        );
        let other = other.unwrap();
        let new_id = Uuid::new_v4().to_string();
        let (_, new) = begin(&service, &s, &sessions, &a, &at, &new_id);
        let new = new.unwrap();
        assert!(old.cancel.is_cancelled());
        assert!(!other.cancel.is_cancelled());
        assert!(service.push(&s, &sessions, &old, "迟到旧文本").is_err());
        service.push(&s, &sessions, &new, "部分结果").unwrap();
        let cancelled = service
            .access(
                &s,
                &sessions,
                &a,
                &at,
                "ai.cancel",
                json!({"requestId":new_id}),
            )
            .unwrap()
            .0;
        assert_eq!(cancelled["state"], "cancelled");
        assert_eq!(cancelled["text"], "部分结果");
        assert!(new.cancel.is_cancelled());
        assert!(!other.cancel.is_cancelled());
        assert!(service
            .access(
                &s,
                &sessions,
                &b,
                &bt,
                "ai.poll",
                json!({"requestId":old_id})
            )
            .is_err());
    }
    #[test]
    fn ai_host_session_expiration_and_config_change_reject_late_success() {
        let (s, sessions, service) = fixture();
        let (_, window, token) = add(&s, &sessions, PACKAGE, "ai.generate");
        configure(&service, &s, &sessions, &window, &token, 0, true);
        let id = Uuid::new_v4().to_string();
        let (_, work) = begin(&service, &s, &sessions, &window, &token, &id);
        let work = work.unwrap();
        service.push(&s, &sessions, &work, "真实部分").unwrap();
        configure(&service, &s, &sessions, &window, &token, 1, false);
        assert!(work.cancel.is_cancelled());
        assert!(service
            .finish(&s, &sessions, &work, Ok("旧成功".into()))
            .is_err());
        configure(&service, &s, &sessions, &window, &token, 2, true);
        let (_, work) = service
            .access(
                &s,
                &sessions,
                &window,
                &token,
                "ai.begin",
                input(&Uuid::new_v4().to_string(), 3),
            )
            .unwrap();
        let work = work.unwrap();
        sessions.0.lock().unwrap().remove(&token);
        assert!(service.push(&s, &sessions, &work, "不应出现").is_err());
        assert!(work.cancel.is_cancelled());
    }
    #[test]
    fn ai_host_delete_cleans_only_own_credentials_receipts_and_late_finish_cannot_recreate_them() {
        let (s, sessions, service) = fixture();
        let (a, aw, at) = add(&s, &sessions, PACKAGE, "ai.generate");
        let (b, bw, bt) = add(&s, &sessions, PACKAGE, "ai.generate");
        for (window, token) in [(&aw, &at), (&bw, &bt)] {
            configure(&service, &s, &sessions, window, token, 0, true);
        }
        let (_, old) = begin(
            &service,
            &s,
            &sessions,
            &aw,
            &at,
            &Uuid::new_v4().to_string(),
        );
        let old = old.unwrap();
        let (_, other) = begin(
            &service,
            &s,
            &sessions,
            &bw,
            &bt,
            &Uuid::new_v4().to_string(),
        );
        let other = other.unwrap();
        widgets::remove_instance(&s, &sessions, &a).unwrap();
        assert!(service
            .finish(&s, &sessions, &old, Ok("不应复活".into()))
            .is_err());
        assert_eq!(ai_credentials::status(&s, &a).unwrap()["configured"], false);
        let count: i64 =
            s.db.lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM app_settings WHERE key=?1",
                    [format!("ai.requests/{a}")],
                    |row| row.get(0),
                )
                .unwrap();
        assert_eq!(count, 0);
        service
            .push(&s, &sessions, &other, "另一个实例继续")
            .unwrap();
        assert_eq!(ai_credentials::status(&s, &b).unwrap()["configured"], true);
    }
    #[test]
    fn ai_host_limits_active_work_and_stopping_rejects_every_new_native_operation() {
        let (s, sessions, service) = fixture();
        let mut works = Vec::new();
        let mut last = None;
        for _ in 0..5 {
            let (_, window, token) = add(&s, &sessions, PACKAGE, "ai.generate");
            configure(&service, &s, &sessions, &window, &token, 0, true);
            if works.len() < 4 {
                let (_, work) = begin(
                    &service,
                    &s,
                    &sessions,
                    &window,
                    &token,
                    &Uuid::new_v4().to_string(),
                );
                works.push(work.unwrap());
            } else {
                assert!(service
                    .access(
                        &s,
                        &sessions,
                        &window,
                        &token,
                        "ai.begin",
                        input(&Uuid::new_v4().to_string(), 1)
                    )
                    .unwrap_err()
                    .contains("LIMIT"));
                last = Some((window, token));
            }
        }
        service.stop();
        assert!(works.iter().all(|work| work.cancel.is_cancelled()));
        let (window, token) = last.unwrap();
        assert!(service
            .access(&s, &sessions, &window, &token, "ai.status", Value::Null)
            .unwrap_err()
            .contains("退出"));
    }
    #[test]
    fn ai_host_completed_receipt_failure_keeps_real_text_but_never_reports_completed_or_resends() {
        let (s, sessions, service) = fixture();
        let (_, window, token) = add(&s, &sessions, PACKAGE, "ai.generate");
        configure(&service, &s, &sessions, &window, &token, 0, true);
        let id = Uuid::new_v4().to_string();
        let (_, work) = begin(&service, &s, &sessions, &window, &token, &id);
        let work = work.unwrap();
        service.push(&s, &sessions, &work, "真实部分").unwrap();
        s.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_ai_completion BEFORE UPDATE ON app_settings WHEN new.key LIKE 'ai.requests/%' BEGIN SELECT RAISE(FAIL,'completion denied'); END;").unwrap();
        assert!(service
            .finish(&s, &sessions, &work, Ok("真实完整文本".into()))
            .is_err());
        let polled = service
            .access(
                &s,
                &sessions,
                &window,
                &token,
                "ai.poll",
                json!({"requestId":id}),
            )
            .unwrap()
            .0;
        assert_eq!(polled["state"], "failed");
        assert_eq!(polled["text"], "真实完整文本");
        assert!(polled["error"].as_str().unwrap().contains("未保存"));
        let (_, work) = begin(&Service::default(), &s, &sessions, &window, &token, &id);
        assert!(work.is_none());
    }
    #[test]
    fn ai_host_live_session_lock_spans_result_commit_so_close_cannot_interleave() {
        let (s, sessions, service) = fixture();
        let (_, window, token) = add(&s, &sessions, PACKAGE, "ai.generate");
        configure(&service, &s, &sessions, &window, &token, 0, true);
        let (_, work) = begin(
            &service,
            &s,
            &sessions,
            &window,
            &token,
            &Uuid::new_v4().to_string(),
        );
        let work = work.unwrap();
        service
            .with_live(&s, &sessions, &work, || {
                assert!(
                    sessions.0.try_lock().is_err(),
                    "完成/部分结果提交期间必须保留当前会话锁"
                );
                Ok(())
            })
            .unwrap();
    }
    #[test]
    fn ai_host_snapshot_retains_exact_validated_input_for_recreated_view_without_credentials() {
        let (s, sessions, service) = fixture();
        let (_, window, token) = add(&s, &sessions, PACKAGE, "ai.generate");
        configure(&service, &s, &sessions, &window, &token, 0, true);
        let id = Uuid::new_v4().to_string();
        let (value, work) = begin(&service, &s, &sessions, &window, &token, &id);
        assert_eq!(
            value["input"],
            json!({"content":"你好","firstLang":"zh","secondLang":"en"})
        );
        assert!(!value.to_string().contains("fixture-key"));
        service
            .push(&s, &sessions, &work.unwrap(), "真实部分")
            .unwrap();
        let current = service
            .access(&s, &sessions, &window, &token, "ai.status", Value::Null)
            .unwrap()
            .0;
        assert_eq!(current["job"]["input"], value["input"]);
    }
    #[test]
    fn ai_host_restart_status_exposes_interrupted_receipt_without_restarting_work() {
        let (s, sessions, service) = fixture();
        let (_, window, token) = add(&s, &sessions, PACKAGE, "ai.generate");
        configure(&service, &s, &sessions, &window, &token, 0, true);
        let id = Uuid::new_v4().to_string();
        let (_, work) = begin(&service, &s, &sessions, &window, &token, &id);
        service
            .push(&s, &sessions, &work.unwrap(), "真实部分")
            .unwrap();
        let restarted = Service::default();
        let (status, work) = restarted
            .access(&s, &sessions, &window, &token, "ai.status", Value::Null)
            .unwrap();
        assert!(work.is_none());
        assert_eq!(status["job"]["state"], "interrupted");
        assert_eq!(status["job"]["requestId"], id);
        assert_eq!(status["job"]["resultAvailable"], false);
        assert!(!status.to_string().contains("fixture-key"));
    }
    #[test]
    fn ai_host_hidden_preview_cannot_write_key_reserve_request_or_enable_network() {
        use std::{
            io::{Cursor, Write},
            sync::Arc,
        };
        let (s, sessions, service) = fixture();
        let (instance, _, _) = add(&s, &sessions, PACKAGE, "ai.generate");
        let raw: String =
            s.db.lock()
                .unwrap()
                .query_row(
                    "SELECT manifest FROM widget_packages WHERE id=?1",
                    [PACKAGE],
                    |row| row.get(0),
                )
                .unwrap();
        let mut manifest: Value = serde_json::from_str(&raw).unwrap();
        manifest["version"] = json!("0.1.1");
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("manifest.json", options).unwrap();
        archive.write_all(manifest.to_string().as_bytes()).unwrap();
        archive.start_file("dist/index.html", options).unwrap();
        archive
            .write_all(b"<!doctype html><title>AI preview</title>")
            .unwrap();
        let package = widgets::stage_bytes(&s, archive.finish().unwrap().into_inner()).unwrap();
        let window = format!("trial-{}", Uuid::new_v4());
        let preview = Arc::new(Mutex::new(widgets::Preview {
            package,
            settings: json!({}),
            storage: json!({}),
            ready_since: None,
            failure: None,
        }));
        let session = widgets::open_preview(&sessions, &window, &window, preview).unwrap();
        for (method, args) in [
            ("ai.status", Value::Null),
            ("ai.begin", input(&Uuid::new_v4().to_string(), 0)),
            (
                "ai.configure",
                json!({"requestId":Uuid::new_v4().to_string(),"expectedRevision":0,"apiBaseUrl":"https://api.example.com/v1","apiKey":"fixture-key","model":"fixture-model","enabled":true}),
            ),
        ] {
            assert!(service
                .access(
                    &s,
                    &sessions,
                    &window,
                    session["token"].as_str().unwrap(),
                    method,
                    args
                )
                .unwrap_err()
                .contains("试加载"));
        }
        assert_eq!(
            ai_credentials::status(&s, &instance).unwrap()["revision"],
            0
        );
        assert!(service.jobs.lock().unwrap().is_empty());
    }
    async fn peer() -> (
        String,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<String>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (release, finish) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0; 2048];
                let n = stream.read(&mut buffer).await.unwrap();
                assert!(n > 0 && bytes.len() < 128 * 1024);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes
                    .windows(4)
                    .position(|part| part == b"\r\n\r\n")
                    .map(|end| end + 4)
                {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let len = header
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse::<usize>()
                        .unwrap();
                    if bytes.len() >= end + len {
                        break;
                    }
                }
            }
            let first =
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"第一段\"}}]}\n\n";
            let second="data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"结束\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
            let header=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",first.len()+second.len());
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(first.as_bytes()).await.unwrap();
            let _ = finish.await;
            let _ = stream.write_all(second.as_bytes()).await;
            String::from_utf8(bytes).unwrap()
        });
        (format!("http://{address}/v1"), release, server)
    }
    fn connection(
        service: &Service,
        store: &Store,
        sessions: &widgets::Sessions,
        window: &str,
        token: &str,
        url: &str,
    ) {
        service.access(store,sessions,window,token,"ai.configure",json!({"requestId":Uuid::new_v4().to_string(),"expectedRevision":0,"apiBaseUrl":url,"apiKey":"fixture-key","model":"fixture-model","enabled":true})).unwrap();
    }
    async fn wait_partial(
        service: &Service,
        store: &Store,
        sessions: &widgets::Sessions,
        window: &str,
        token: &str,
        id: &str,
    ) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let value = service
                    .access(
                        store,
                        sessions,
                        window,
                        token,
                        "ai.poll",
                        json!({"requestId":id}),
                    )
                    .unwrap()
                    .0;
                if value["text"] == "第一段" {
                    assert_eq!(value["state"], "running");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn ai_host_real_loopback_exercises_authorized_begin_partial_poll_and_persisted_completion(
    ) {
        let (s, sessions, service) = fixture();
        let (_, window, token) = add(&s, &sessions, PACKAGE, "ai.generate");
        let (url, release, server) = peer().await;
        connection(&service, &s, &sessions, &window, &token, &url);
        let id = Uuid::new_v4().to_string();
        let (_, work) = begin(&service, &s, &sessions, &window, &token, &id);
        let observer = async {
            wait_partial(&service, &s, &sessions, &window, &token, &id).await;
            release.send(()).unwrap();
        };
        let (result, _) = tokio::join!(service.execute(&s, &sessions, work.unwrap()), observer);
        result.unwrap();
        let done = service
            .access(
                &s,
                &sessions,
                &window,
                &token,
                "ai.poll",
                json!({"requestId":id}),
            )
            .unwrap()
            .0;
        assert_eq!(done["state"], "completed");
        assert_eq!(done["text"], "第一段结束");
        assert_eq!(done["resultAvailable"], true);
        let request = server.await.unwrap();
        assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
        let (replay, work) = begin(&Service::default(), &s, &sessions, &window, &token, &id);
        assert!(work.is_none());
        assert_eq!(replay["state"], "completed");
        assert_eq!(replay["resultAvailable"], false);
    }
    #[tokio::test]
    async fn ai_host_heartbeat_cancels_expired_session_even_when_server_sends_no_further_chunks() {
        let (s, sessions, service) = fixture();
        let (_, window, token) = add(&s, &sessions, PACKAGE, "ai.generate");
        let (url, release, server) = peer().await;
        connection(&service, &s, &sessions, &window, &token, &url);
        let id = Uuid::new_v4().to_string();
        let (_, work) = begin(&service, &s, &sessions, &window, &token, &id);
        let observer = async {
            wait_partial(&service, &s, &sessions, &window, &token, &id).await;
            sessions.0.lock().unwrap().remove(&token);
        };
        let (result, _) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(service.execute(&s, &sessions, work.unwrap()), observer)
        })
        .await
        .unwrap();
        assert!(result.unwrap_err().starts_with("SESSION_EXPIRED:"));
        let _ = release.send(());
        server.await.unwrap();
        let status = service.jobs.lock().unwrap();
        let job = status.values().next().unwrap();
        assert_eq!(job.snapshot.state, "cancelled");
        assert_eq!(job.snapshot.text, "第一段");
    }
}
