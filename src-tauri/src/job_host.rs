//! 固定AI直聘任务协调；后台任务归核心所有，不依赖网页计时器或组件会话存活。
use crate::{
    ai_credentials::{self, Profile},
    ai_protocol, ai_transport, business, job_protocol,
    job_state::{self, Phase},
    job_surfaces::{self, Page, Surface},
    job_webviews::{self, Owner},
    network::NetworkConfig,
    store::{err, now_ms, AppResult, Store},
    widgets,
};
use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

type Pending<'a, T> = Pin<Box<dyn Future<Output = AppResult<T>> + Send + 'a>>;
type Update<'a> = Box<dyn FnMut(&str) -> AppResult<()> + Send + 'a>;
pub(crate) trait Browser: Send + Sync {
    type Capture: Send;
    fn page<'a>(
        &'a self,
        owner: Owner,
        keyword: String,
        page: u32,
        cancel: CancellationToken,
    ) -> Pending<'a, Self::Capture>;
    /// 原生驱动在持有导航账本锁期间提交，防止鉴权与落库之间穿过新导航。
    fn commit<T>(
        &self,
        capture: Self::Capture,
        apply: impl FnOnce(Value) -> AppResult<T>,
    ) -> AppResult<T>;
}
pub(crate) trait Model: Send + Sync {
    fn stream<'a>(
        &'a self,
        network: &'a NetworkConfig,
        profile: &'a Profile,
        body: Value,
        cancel: &'a CancellationToken,
        update: Update<'a>,
    ) -> Pending<'a, String>;
}
pub(crate) struct HttpModel;
impl Model for HttpModel {
    fn stream<'a>(
        &'a self,
        network: &'a NetworkConfig,
        profile: &'a Profile,
        body: Value,
        cancel: &'a CancellationToken,
        update: Update<'a>,
    ) -> Pending<'a, String> {
        Box::pin(async move {
            ai_transport::generate(
                network,
                ai_protocol::endpoint(&profile.base_url)?,
                &profile.key,
                body,
                cancel,
                update,
            )
            .await
        })
    }
}
#[cfg(windows)]
impl Browser for job_webviews::native::Native {
    type Capture = (job_webviews::Ticket, Value);
    fn commit<T>(
        &self,
        (ticket, value): Self::Capture,
        apply: impl FnOnce(Value) -> AppResult<T>,
    ) -> AppResult<T> {
        job_webviews::commit(&self.book, &ticket, || apply(value))
    }
    fn page<'a>(
        &'a self,
        owner: Owner,
        keyword: String,
        page: u32,
        cancel: CancellationToken,
    ) -> Pending<'a, Self::Capture> {
        Box::pin(async move {
            let login =
                self.book
                    .lock()
                    .map_err(err)?
                    .ticket(&owner, job_webviews::Script::Login, None)?;
            if !job_protocol::login_state(self.evaluate(login, &cancel).await?)? {
                return Err("LOGIN_REQUIRED: 请先在BOSS页面手动登录".into());
            }
            let request = self.navigate(&owner, job_protocol::search_url(&keyword, page)?)?;
            let url = self.wait(&owner, &request, &cancel).await?;
            if !job_webviews::search_identity(&url, &keyword, page) {
                let ticket = self.book.lock().map_err(err)?.ticket(
                    &owner,
                    job_webviews::Script::Login,
                    None,
                )?;
                if !job_protocol::login_state(self.evaluate(ticket, &cancel).await?)? {
                    return Err("LOGIN_REQUIRED: BOSS登录已失效，请手动登录后重新开始".into());
                }
                return Err(
                    "PRESENTATION_EXPIRED: BOSS页面跳转后不再属于当前搜索，未继续评分".into(),
                );
            }
            pause(&cancel, Duration::from_secs(2)).await?;
            for attempt in 0..3 {
                let ticket = self.book.lock().map_err(err)?.ticket(
                    &owner,
                    job_webviews::Script::Capture,
                    Some((&keyword, page)),
                )?;
                let value = self.evaluate(ticket.clone(), &cancel).await?;
                if value["error"] != "not_ready" || attempt == 2 {
                    return Ok((ticket, value));
                }
                pause(&cancel, Duration::from_secs(1)).await?;
            }
            unreachable!("固定三次内部载入检查都有返回")
        })
    }
}
async fn pause(cancel: &CancellationToken, duration: Duration) -> AppResult<()> {
    tokio::select! {biased;_=cancel.cancelled()=>Err("CANCELLED".into()),_=tokio::time::sleep(duration)=>Ok(())}
}
pub(crate) struct Source {
    pub digest: String,
    pub control: job_state::Control,
    pub visible: bool,
    pub hidden: bool,
}
pub(crate) fn source(store: &Store, instance: &str) -> AppResult<Source> {
    let db = store.db.lock().map_err(err)?;
    let row:Option<(String,String,String,i64)>=db.query_row("SELECT p.id,p.manifest,p.digest,i.visible FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE i.id=?1",[instance],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|"STORAGE_ERROR: AI直聘实例读取失败")?;
    let (package, raw, digest, visible) = row.ok_or("SESSION_EXPIRED: AI直聘实例已移除")?;
    let manifest: widgets::Manifest =
        serde_json::from_str(&raw).map_err(|_| "DATA_UNAVAILABLE: AI直聘清单无效")?;
    if package != job_protocol::PACKAGE
        || manifest.id != job_protocol::PACKAGE
        || !manifest
            .capabilities
            .iter()
            .any(|c| c == job_protocol::CAPABILITY)
    {
        return Err("CAPABILITY_DENIED: 仅声明AI直聘能力的固定组件可访问".into());
    }
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("DATA_UNAVAILABLE: AI直聘包版本标识无效".into());
    }
    let hidden: Option<String> = db
        .query_row(
            "SELECT value FROM app_settings WHERE key='hidden_all'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "STORAGE_ERROR: 组件显隐状态读取失败")?;
    if ![0, 1].contains(&visible)
        || hidden
            .as_deref()
            .is_some_and(|v| v != "true" && v != "false")
    {
        return Err("DATA_UNAVAILABLE: AI直聘显隐状态无效".into());
    }
    Ok(Source {
        digest,
        control: job_state::load(&db, instance)?,
        visible: visible == 1,
        hidden: hidden.as_deref() == Some("true"),
    })
}
struct Entry {
    id: String,
    window: String,
    digest: String,
    revision: u64,
    credential_revision: u64,
    cancel: CancellationToken,
    active: bool,
    received: usize,
    error: Option<String>,
    browser_owned: bool,
    ending: Option<Phase>,
}
#[derive(Default)]
pub(crate) struct Service {
    pub knowledge: crate::job_knowledge_host::Service,
    entries: Mutex<HashMap<String, Entry>>,
    pub views: job_webviews::Shared,
    pub surfaces: Mutex<job_surfaces::Registry>,
    pub render_lock: Mutex<()>,
    renders: Mutex<HashMap<String, u64>>,
    stopped: AtomicBool,
}
/// 不实现Serialize/Debug，避免模型密钥进入前端状态或诊断。
pub(crate) struct Work {
    instance: String,
    id: String,
    revision: u64,
    credential_revision: u64,
    digest: String,
    owner: Option<Owner>,
    profile: Profile,
    network: NetworkConfig,
    cancel: CancellationToken,
    pace: Duration,
}
impl Drop for Work {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl Service {
    pub(crate) fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub(crate) fn current_window(&self, window: &str, instance: &str) -> bool {
        !self.stopped()
            && self
                .surfaces
                .lock()
                .is_ok_and(|r| r.shell(window, instance).is_some())
    }
    pub(crate) fn bind(
        &self,
        store: &Store,
        window: &str,
        instance: &str,
        token: &str,
    ) -> AppResult<()> {
        let src = source(store, instance)?;
        let mut r = self.surfaces.lock().map_err(err)?;
        let s = r
            .shell(window, instance)
            .ok_or("PRESENTATION_EXPIRED: AI直聘本机壳不属于当前实例")?;
        if s.owner.digest != src.digest {
            return Err("VERSION_INCOMPATIBLE: AI直聘包版本已改变".into());
        }
        r.bind(window, instance, token)
    }
    pub(crate) fn closed_memory(&self, s: &Surface, retired: bool, force: bool) -> AppResult<()> {
        self.knowledge.closed_memory(s, force);
        if force || !s.background {
            if let Some(e) = self
                .entries
                .lock()
                .map_err(err)?
                .get_mut(&s.owner.instance)
                .filter(|e| e.window == s.shell() || s.search() && e.browser_owned)
            {
                e.cancel.cancel();
                e.active = false;
                e.ending.get_or_insert(Phase::Stopped);
                e.error = Some("任务窗口已关闭或失效，旧请求不自动重发".into());
            }
        }
        if s.search() && retired {
            self.views.lock().map_err(err)?.fail(&s.owner);
        }
        Ok(())
    }
    /// 可在实例删除/升级事务中调用：只撤销内存许可，不取得SQLite或原生窗口锁。
    pub(crate) fn revoke(&self, instance: &str) -> AppResult<()> {
        self.knowledge.revoke(instance);
        if let Some(e) = self.entries.lock().map_err(err)?.get_mut(instance) {
            e.cancel.cancel();
            e.active = false;
            e.ending.get_or_insert(Phase::Stopped);
            e.error = Some("实例或组件版本已撤销，旧任务已停止".into());
        }
        self.surfaces.lock().map_err(err)?.revoke(instance);
        let mut views = self.views.lock().map_err(err)?;
        if let Some(owner) = views.owner(instance) {
            views.fail(&owner);
        }
        Ok(())
    }
    pub(crate) fn shell_failed(
        &self,
        store: &Store,
        window: &str,
        instance: &str,
    ) -> AppResult<()> {
        let _native = store.native_actions.lock().map_err(err)?;
        let src = source(store, instance)?;
        let surface = {
            let mut r = self.surfaces.lock().map_err(err)?;
            let current = r
                .shell(window, instance)
                .ok_or("PRESENTATION_EXPIRED: 旧本机壳已撤销")?;
            if current.owner.digest != src.digest {
                return Err("VERSION_INCOMPATIBLE: 本机壳版本已改变".into());
            }
            r.fail(window)?
        };
        self.closed_memory(&surface, false, true)?;
        self.reconcile_locked(store, instance)
    }
    pub(crate) fn hide_all(&self, store: &Store) -> AppResult<()> {
        let _native = store.native_actions.lock().map_err(err)?;
        let mut ids = self.surfaces.lock().map_err(err)?.instances();
        ids.extend(self.entries.lock().map_err(err)?.keys().cloned());
        ids.sort();
        ids.dedup();
        for i in ids {
            self.surfaces.lock().map_err(err)?.hide(&i)?;
            if source(store, &i).is_ok_and(|s| !s.control.config.background_enabled) {
                self.knowledge.cancel_instance_locked(store, &i)?;
                self.cancel_locked(
                    store,
                    &i,
                    None,
                    Phase::Stopped,
                    "组件已隐藏，未开启后台继续",
                )?;
            }
        }
        Ok(())
    }
    pub(crate) fn hide_instance(&self, store: &Store, instance: &str) -> AppResult<()> {
        if !self.entries.lock().map_err(err)?.contains_key(instance)
            && self.surfaces.lock().map_err(err)?.list(instance).is_empty()
        {
            return Ok(());
        }
        let _native = store.native_actions.lock().map_err(err)?;
        self.surfaces.lock().map_err(err)?.hide(instance)?;
        if source(store, instance).is_ok_and(|s| !s.control.config.background_enabled) {
            self.knowledge.cancel_instance_locked(store, instance)?;
            self.cancel_locked(
                store,
                instance,
                None,
                Phase::Stopped,
                "组件已隐藏，未开启后台继续",
            )?;
        }
        Ok(())
    }
    pub(crate) fn stop_memory(&self) {
        self.knowledge.stop_memory();
        self.stopped.store(true, Ordering::Release);
        if let Ok(mut entries) = self.entries.lock() {
            for e in entries.values_mut() {
                e.cancel.cancel();
                e.active = false;
                e.ending.get_or_insert(Phase::Interrupted);
            }
        }
    }
    fn snapshot(&self, store: &Store, instance: &str) -> AppResult<Value> {
        let src = source(store, instance)?;
        let mut value = job_state::snapshot(&src.control);
        value["model"] = ai_credentials::status(store, instance)?;
        value["browser"] = self.views.lock().map_err(err)?.status(instance);
        value["presentation"] = self.surfaces.lock().map_err(err)?.snapshot(instance);
        value["execution"]=self.entries.lock().map_err(err)?.get(instance).map(|e|json!({"requestId":e.id,"active":e.active&&!e.cancel.is_cancelled(),"received":e.received,"error":e.error})).unwrap_or(Value::Null);
        Ok(value)
    }
    fn cancel_locked(
        &self,
        store: &Store,
        instance: &str,
        id: Option<&str>,
        mut phase: Phase,
        reason: &str,
    ) -> AppResult<()> {
        if let Some(e) = self
            .entries
            .lock()
            .map_err(err)?
            .get_mut(instance)
            .filter(|e| id.is_none_or(|id| e.id == id))
        {
            e.cancel.cancel();
            e.active = false;
            e.error = Some(reason.to_owned());
            phase = e.ending.get_or_insert(phase).clone();
        }
        let mut db = store.db.lock().map_err(err)?;
        let c = job_state::load(&db, instance)?;
        if let Some(r) = c
            .run
            .filter(|r| r.phase.active() && id.is_none_or(|id| r.id == id))
        {
            job_state::stop(&mut db, instance, &r.id, phase, now_ms())?;
        }
        Ok(())
    }
    pub(crate) fn cancel(&self, store: &Store, instance: &str, reason: &str) -> AppResult<()> {
        let _native = store.native_actions.lock().map_err(err)?;
        self.cancel_locked(store, instance, None, Phase::Stopped, reason)
    }
    pub(crate) fn closed_window(&self, store: &Store, window: &str) -> AppResult<()> {
        let _native = store.native_actions.lock().map_err(err)?;
        let ids = self
            .entries
            .lock()
            .map_err(err)?
            .iter()
            .filter(|(_, e)| e.window == window && e.active)
            .map(|(i, _)| i.clone())
            .collect::<Vec<_>>();
        for i in ids {
            match source(store, &i) {
                Ok(src) if src.control.config.background_enabled => (),
                _ => self.cancel_locked(
                    store,
                    &i,
                    None,
                    Phase::Stopped,
                    "任务所属窗口已关闭，未启用后台继续",
                )?,
            }
        }
        Ok(())
    }
    /// 不存在内存执行者时，持久化的运行状态只恢复为中断，不重新调度。
    pub(crate) fn reconcile_locked(&self, store: &Store, instance: &str) -> AppResult<()> {
        let src = source(store, instance)?;
        if let Some(r) = src.control.run.filter(|r| r.phase.active()) {
            let current = self
                .entries
                .lock()
                .map_err(err)?
                .get(instance)
                .is_some_and(|e| {
                    e.active
                        && !e.cancel.is_cancelled()
                        && e.id == r.id
                        && e.digest == src.digest
                        && e.revision == src.control.revision
                        && e.credential_revision == r.credential_revision
                });
            if !current {
                self.cancel_locked(
                    store,
                    instance,
                    Some(&r.id),
                    Phase::Interrupted,
                    "上次任务已中断，请手动重新开始；未自动补发模型请求",
                )?;
            } else if ai_credentials::status(store, instance)?["revision"].as_u64()
                != Some(r.credential_revision)
            {
                self.cancel_locked(
                    store,
                    instance,
                    Some(&r.id),
                    Phase::Stopped,
                    "模型配置已改变，旧任务已停止",
                )?;
            }
        }
        Ok(())
    }
    pub(crate) fn access(
        &self,
        store: &Store,
        sessions: &widgets::Sessions,
        window: &str,
        token: &str,
        method: &str,
        args: Value,
    ) -> AppResult<(Value, Option<Work>)> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("SESSION_EXPIRED: AI直聘宿主正在退出".into());
        }
        if serde_json::to_vec(&args).map_err(err)?.len() > 64 * 1024 {
            return Err("CAPACITY_EXCEEDED: AI直聘请求超过容量".into());
        }
        let _native = store.native_actions.lock().map_err(err)?;
        widgets::with_capability(
            store,
            sessions,
            window,
            token,
            job_protocol::CAPABILITY,
            |session| {
                let instance = &session.instance;
                let src = source(store, instance)?;
                if job_surfaces::role(window).is_some() {
                    let surface = self
                        .surfaces
                        .lock()
                        .map_err(err)?
                        .authorized(window, token)?;
                    if surface.owner.digest != src.digest {
                        return Err("VERSION_INCOMPATIBLE: AI直聘本机壳版本已改变".into());
                    }
                }
                if (src.hidden || !src.visible)
                    && !matches!(
                        method,
                        "jobs.status"
                            | "jobs.query"
                            | "jobs.stop"
                            | "jobs.model.clear"
                            | "jobs.surface.info"
                            | "jobs.surface.ready"
                            | "jobs.surface.layout"
                            | "jobs.surface.overlay"
                            | "jobs.surface.close"
                    )
                {
                    return Err("PRESENTATION_EXPIRED: 当前组件已隐藏，未执行新操作".into());
                }
                match method {
                    "jobs.help" => {
                        #[derive(Deserialize)]
                        #[serde(deny_unknown_fields)]
                        struct Input {
                            topic: String,
                        }
                        let input: Input = serde_json::from_value(args)
                            .map_err(|_| "INVALID_ARGUMENT: 帮助只接受固定topic")?;
                        job_protocol::help_url(&input.topic)?;
                        let mut value = self.snapshot(store, instance)?;
                        value["helpTopic"] = json!(input.topic);
                        Ok((value, None))
                    }
                    "jobs.open" => {
                        let page: Page = serde_json::from_value(args).map_err(|_| {
                            "INVALID_ARGUMENT: 仅接受搜索、分析、知识或固定职位详情页"
                        })?;
                        if let Page::Detail { job_id } = &page {
                            if job_state::row(&*store.db.lock().map_err(err)?, instance, job_id)?
                                .is_none()
                            {
                                return Err("NOT_FOUND: 职位已不存在".into());
                            }
                        }
                        let surface = self.surfaces.lock().map_err(err)?.open(
                            instance,
                            &src.digest,
                            page,
                            src.control.config.background_enabled,
                        )?;
                        let mut value = self.snapshot(store, instance)?;
                        value["opened"] = surface.snapshot();
                        Ok((value, None))
                    }
                    "jobs.surface.info"
                    | "jobs.surface.ready"
                    | "jobs.surface.overlay"
                    | "jobs.surface.layout"
                    | "jobs.surface.close" => {
                        let surface = self
                            .surfaces
                            .lock()
                            .map_err(err)?
                            .authorized(window, token)?;
                        match method {
                            "jobs.surface.info" => {
                                empty(&args)?;
                                return Ok((
                                    json!({"instanceId":instance,"surface":surface.snapshot()}),
                                    None,
                                ));
                            }
                            "jobs.surface.ready" => {
                                empty(&args)?;
                                self.surfaces.lock().map_err(err)?.ready(window, token)?;
                            }
                            "jobs.surface.overlay" => {
                                #[derive(Deserialize)]
                                #[serde(deny_unknown_fields)]
                                struct Input {
                                    active: bool,
                                }
                                let input: Input = serde_json::from_value(args)
                                    .map_err(|_| "INVALID_ARGUMENT: 覆盖层仅接受active")?;
                                self.surfaces.lock().map_err(err)?.overlay(
                                    window,
                                    token,
                                    input.active,
                                )?;
                            }
                            "jobs.surface.layout" => {
                                #[derive(Deserialize)]
                                #[serde(deny_unknown_fields)]
                                struct Input {
                                    x: f64,
                                    y: f64,
                                    width: f64,
                                    height: f64,
                                }
                                let r: Input = serde_json::from_value(args)
                                    .map_err(|_| "INVALID_ARGUMENT: BOSS布局只接受固定矩形")?;
                                self.surfaces.lock().map_err(err)?.layout(
                                    window,
                                    token,
                                    (r.x, r.y, r.width, r.height),
                                )?;
                            }
                            "jobs.surface.close" => {
                                empty(&args)?;
                                if let Some((s, retired)) = self
                                    .surfaces
                                    .lock()
                                    .map_err(err)?
                                    .close(&surface.parent(), false)?
                                {
                                    self.closed_memory(&s, retired, false)?;
                                }
                            }
                            _ => unreachable!(),
                        }
                        let mut value = self.snapshot(store, instance)?;
                        value["surface"] = self
                            .surfaces
                            .lock()
                            .map_err(err)?
                            .shell(window, instance)
                            .map(|s| s.snapshot())
                            .unwrap_or(Value::Null);
                        Ok((value, None))
                    }
                    "jobs.status" => {
                        empty(&args)?;
                        self.reconcile_locked(store, instance)?;
                        Ok((self.snapshot(store, instance)?, None))
                    }
                    "jobs.configure" => {
                        #[derive(Deserialize)]
                        #[serde(rename_all = "camelCase", deny_unknown_fields)]
                        struct Input {
                            request_id: String,
                            expected_revision: u64,
                            config: job_state::Config,
                        }
                        let v: Input = serde_json::from_value(args)
                            .map_err(|_| "INVALID_ARGUMENT: 求职设置参数无效")?;
                        let (c, replayed) = job_state::configure(
                            &mut *store.db.lock().map_err(err)?,
                            instance,
                            &v.request_id,
                            v.expected_revision,
                            v.config,
                            now_ms(),
                        )?;
                        self.surfaces
                            .lock()
                            .map_err(err)?
                            .configure(instance, c.config.background_enabled)?;
                        if let Some(e) = self
                            .entries
                            .lock()
                            .map_err(err)?
                            .get_mut(instance)
                            .filter(|e| e.revision != c.revision)
                        {
                            e.cancel.cancel();
                            e.active = false;
                            e.error = Some("求职条件已改变，旧任务已停止".into());
                        }
                        let mut v = self.snapshot(store, instance)?;
                        v["replayed"] = json!(replayed);
                        Ok((v, None))
                    }
                    "jobs.model.configure" | "jobs.model.clear" => {
                        let status = if method == "jobs.model.configure" {
                            ai_credentials::configure(store, instance, args)?
                        } else {
                            ai_credentials::clear(store, instance, args)?
                        };
                        self.knowledge.cancel_revision_locked(
                            store,
                            instance,
                            status["revision"]
                                .as_u64()
                                .ok_or("DATA_UNAVAILABLE: 模型版本无效")?,
                        )?;
                        let c = source(store, instance)?.control;
                        if let Some(r) = c.run.filter(|r| {
                            r.phase.active()
                                && Some(r.credential_revision) != status["revision"].as_u64()
                        }) {
                            self.cancel_locked(
                                store,
                                instance,
                                Some(&r.id),
                                Phase::Stopped,
                                "模型配置已改变，旧任务已停止",
                            )?;
                        }
                        Ok((self.snapshot(store, instance)?, None))
                    }
                    "jobs.start" | "jobs.rerate" => {
                        #[derive(Deserialize)]
                        #[serde(rename_all = "camelCase", deny_unknown_fields)]
                        struct Input {
                            request_id: String,
                            config_revision: u64,
                            generation: u64,
                            model_revision: u64,
                            job_id: Option<String>,
                        }
                        let input: Input = serde_json::from_value(args)
                            .map_err(|_| "INVALID_ARGUMENT: 搜索或重评请求字段无效")?;
                        if (method == "jobs.rerate") != input.job_id.is_some() {
                            return Err("INVALID_ARGUMENT: 只有单项重评接受固定职位标识".into());
                        }
                        let src = source(store, instance)?;
                        let (revision, profile) = ai_credentials::profile(store, instance)?;
                        if revision != input.model_revision {
                            return Err("REVISION_CONFLICT: 模型配置已有新版本".into());
                        }
                        let profile = profile
                            .filter(|p| p.enabled)
                            .ok_or("CONFIG_REQUIRED: 请先保存自己的模型连接并显式启用")?;
                        ai_protocol::endpoint(&profile.base_url)?;
                        let owner = self
                            .views
                            .lock()
                            .map_err(err)?
                            .owner(instance)
                            .filter(|o| o.digest == src.digest);
                        if input.job_id.is_none() && owner.is_none() {
                            return Err(
                                "PRESENTATION_EXPIRED: 请先打开AI直聘并完成BOSS手动登录".into()
                            );
                        }
                        let network = business::resolve_network(
                            &serde_json::from_str::<Value>(
                                &store
                                    .preference("network")
                                    .unwrap_or("{\"mode\":\"system\"}".into()),
                            )
                            .map_err(|_| "DATA_UNAVAILABLE: 宿主网络配置无效")?,
                        )?;
                        let (r, fresh) = {
                            let mut db = store.db.lock().map_err(err)?;
                            if let Some(job) = input.job_id {
                                job_state::rerate(
                                    &mut db,
                                    instance,
                                    &input.request_id,
                                    input.config_revision,
                                    input.generation,
                                    revision,
                                    &job,
                                    now_ms(),
                                )?
                            } else {
                                job_state::start(
                                    &mut db,
                                    instance,
                                    &input.request_id,
                                    input.config_revision,
                                    input.generation,
                                    revision,
                                    now_ms(),
                                )?
                            }
                        };
                        let work = if fresh {
                            let cancel = CancellationToken::new();
                            self.entries.lock().map_err(err)?.insert(
                                instance.clone(),
                                Entry {
                                    id: r.id.clone(),
                                    window: window.into(),
                                    digest: src.digest.clone(),
                                    revision: src.control.revision,
                                    credential_revision: revision,
                                    cancel: cancel.clone(),
                                    active: true,
                                    received: 0,
                                    error: None,
                                    browser_owned: r.target.is_none(),
                                    ending: None,
                                },
                            );
                            Some(Work {
                                instance: instance.clone(),
                                id: r.id,
                                revision: src.control.revision,
                                credential_revision: revision,
                                digest: src.digest,
                                owner,
                                profile,
                                network,
                                cancel,
                                pace: Duration::from_secs(5),
                            })
                        } else {
                            None
                        };
                        let mut value = self.snapshot(store, instance)?;
                        value["replayed"] = json!(!fresh);
                        Ok((value, work))
                    }
                    "jobs.stop" => {
                        #[derive(Deserialize)]
                        #[serde(rename_all = "camelCase", deny_unknown_fields)]
                        struct Input {
                            request_id: String,
                        }
                        let input: Input = serde_json::from_value(args)
                            .map_err(|_| "INVALID_ARGUMENT: 停止只接受当前requestId")?;
                        let c = source(store, instance)?.control;
                        if c.run.as_ref().is_none_or(|r| r.id != input.request_id) {
                            return Err("PRESENTATION_EXPIRED: 搜索任务已被替换".into());
                        }
                        self.cancel_locked(
                            store,
                            instance,
                            Some(&input.request_id),
                            Phase::Stopped,
                            "已手动停止；已发出的请求不自动补发",
                        )?;
                        Ok((self.snapshot(store, instance)?, None))
                    }
                    "jobs.clear" => {
                        #[derive(Deserialize)]
                        #[serde(rename_all = "camelCase", deny_unknown_fields)]
                        struct Input {
                            expected_generation: u64,
                        }
                        let input: Input = serde_json::from_value(args)
                            .map_err(|_| "INVALID_ARGUMENT: 清空只接受expectedGeneration")?;
                        job_state::clear_jobs(
                            &mut *store.db.lock().map_err(err)?,
                            instance,
                            input.expected_generation,
                        )?;
                        Ok((self.snapshot(store, instance)?, None))
                    }
                    "jobs.query" => {
                        let query: job_state::Query = serde_json::from_value(args)
                            .map_err(|_| "INVALID_ARGUMENT: 职位筛选参数无效")?;
                        let revision = ai_credentials::status(store, instance)?["revision"]
                            .as_u64()
                            .ok_or("DATA_UNAVAILABLE: 模型配置版本无效")?;
                        Ok((
                            job_state::query(
                                &*store.db.lock().map_err(err)?,
                                instance,
                                revision,
                                query,
                            )?,
                            None,
                        ))
                    }
                    _ => Err(
                        "CAPABILITY_DENIED: 未知AI直聘固定方法，不接受网址、脚本或任意HTTP请求"
                            .into(),
                    ),
                }
            },
        )
    }
    fn live_locked(&self, store: &Store, w: &Work) -> AppResult<job_state::Run> {
        if self.stopped.load(Ordering::Acquire) || w.cancel.is_cancelled() {
            return Err("CANCELLED".into());
        }
        if !self
            .entries
            .lock()
            .map_err(err)?
            .get(&w.instance)
            .is_some_and(|e| e.id == w.id && e.active && !e.cancel.is_cancelled())
        {
            return Err("SESSION_EXPIRED: 旧AI直聘任务已撤销".into());
        }
        let src = source(store, &w.instance)?;
        if src.digest != w.digest || src.control.revision != w.revision {
            return Err("VERSION_INCOMPATIBLE: AI直聘组件或求职条件已改变".into());
        }
        if !src.control.config.background_enabled && (!src.visible || src.hidden) {
            return Err("CANCELLED".into());
        }
        let (revision, profile) = ai_credentials::profile(store, &w.instance)?;
        if revision != w.credential_revision || !profile.is_some_and(|p| p.enabled) {
            return Err("REVISION_CONFLICT: 模型配置已改变或停用".into());
        }
        src.control
            .run
            .filter(|r| r.id == w.id && r.phase.active())
            .ok_or_else(|| "SESSION_EXPIRED: AI直聘任务已结束".into())
    }
    fn current(&self, store: &Store, w: &Work) -> AppResult<job_state::Run> {
        let _native = store.native_actions.lock().map_err(err)?;
        self.live_locked(store, w)
    }
    fn hold_entry(&self, w: &Work) -> AppResult<std::sync::MutexGuard<'_, HashMap<String, Entry>>> {
        let entries = self.entries.lock().map_err(err)?;
        if !entries
            .get(&w.instance)
            .is_some_and(|e| e.id == w.id && e.active && !e.cancel.is_cancelled())
        {
            return Err("CANCELLED".into());
        }
        Ok(entries)
    }
    fn push(&self, store: &Store, w: &Work, text: &str) -> AppResult<()> {
        if text.len() > 512 * 1024 {
            return Err("CAPACITY_EXCEEDED: 职位评分输出超过512KiB".into());
        }
        let _native = store.native_actions.lock().map_err(err)?;
        self.live_locked(store, w)?;
        if let Some(e) = self
            .entries
            .lock()
            .map_err(err)?
            .get_mut(&w.instance)
            .filter(|e| e.id == w.id && e.active && !e.cancel.is_cancelled())
        {
            e.received = text.len();
        } else {
            return Err("CANCELLED".into());
        }
        Ok(())
    }
    async fn run(
        &self,
        store: &Store,
        w: &Work,
        browser: &impl Browser,
        model: &impl Model,
    ) -> AppResult<()> {
        loop {
            let r = self.current(store, w)?;
            match r.phase {
                Phase::WaitingPage => {
                    if r.page > 1 {
                        pause(&w.cancel, w.pace).await?;
                        self.current(store, w)?;
                    }
                    let owner = w
                        .owner
                        .clone()
                        .ok_or("PRESENTATION_EXPIRED: BOSS原生视图不存在")?;
                    let keyword = source(store, &w.instance)?
                        .control
                        .config
                        .search
                        .search_keyword;
                    let value = browser
                        .page(owner, keyword, r.page, w.cancel.clone())
                        .await?;
                    let _native = store.native_actions.lock().map_err(err)?;
                    self.live_locked(store, w)?;
                    let _entry = self.hold_entry(w)?;
                    let r = browser.commit(value, |value| {
                        job_state::capture(
                            &mut *store.db.lock().map_err(err)?,
                            &w.instance,
                            &w.id,
                            w.credential_revision,
                            value,
                            now_ms(),
                        )
                    })?;
                    if r.phase == Phase::Completed {
                        return Ok(());
                    }
                }
                Phase::ReadyToRate => {
                    let prepared = {
                        let _native = store.native_actions.lock().map_err(err)?;
                        self.live_locked(store, w)?;
                        let _entry = self.hold_entry(w)?;
                        job_state::reserve(
                            &mut *store.db.lock().map_err(err)?,
                            &w.instance,
                            &w.id,
                            w.credential_revision,
                            &w.profile.model,
                            now_ms(),
                        )?
                    };
                    self.current(store, w)?;
                    let output = model
                        .stream(
                            &w.network,
                            &w.profile,
                            prepared.body,
                            &w.cancel,
                            Box::new(|text| self.push(store, w, text)),
                        )
                        .await?;
                    let _native = store.native_actions.lock().map_err(err)?;
                    self.live_locked(store, w)?;
                    let _entry = self.hold_entry(w)?;
                    let r = job_state::complete(
                        &mut *store.db.lock().map_err(err)?,
                        &w.instance,
                        &w.id,
                        w.credential_revision,
                        &prepared.token,
                        &output,
                        now_ms(),
                    )?;
                    if r.phase == Phase::Completed {
                        return Ok(());
                    }
                }
                _ => {
                    return Err(
                        "DATA_UNAVAILABLE: 任务停在已预留或无效阶段，未自动补发模型请求".into(),
                    )
                }
            }
        }
    }
    fn end(&self, store: &Store, w: &Work, error: Option<&str>) -> AppResult<()> {
        let _native = store.native_actions.lock().map_err(err)?;
        let mut requested = None;
        if let Some(e) = self
            .entries
            .lock()
            .map_err(err)?
            .get_mut(&w.instance)
            .filter(|e| e.id == w.id)
        {
            e.active = false;
            requested = e.ending.clone();
            e.cancel.cancel();
            if e.error.is_none() {
                e.error = error.map(str::to_owned);
            }
        }
        if source(store, &w.instance).is_ok() {
            let mut db = store.db.lock().map_err(err)?;
            let c = job_state::load(&db, &w.instance)?;
            if c.run
                .as_ref()
                .is_some_and(|r| r.id == w.id && r.phase.active())
            {
                job_state::stop(
                    &mut db,
                    &w.instance,
                    &w.id,
                    requested.unwrap_or(if error == Some("CANCELLED") {
                        Phase::Stopped
                    } else {
                        Phase::Failed
                    }),
                    now_ms(),
                )?;
            }
        }
        Ok(())
    }
    pub(crate) async fn execute(
        &self,
        store: &Store,
        work: Work,
        browser: &impl Browser,
        model: &impl Model,
    ) -> AppResult<()> {
        struct Guard<'a> {
            service: &'a Service,
            store: &'a Store,
            work: &'a Work,
            done: bool,
        }
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                if !self.done {
                    let _native = self.store.native_actions.lock();
                    self.work.cancel.cancel();
                    let _ = self.service.cancel_locked(
                        self.store,
                        &self.work.instance,
                        Some(&self.work.id),
                        Phase::Interrupted,
                        "AI直聘执行已中断，请手动重新开始；未自动补发请求",
                    );
                }
            }
        }
        let mut guard = Guard {
            service: self,
            store,
            work: &work,
            done: false,
        };
        let result = tokio::select! {biased;_=work.cancel.cancelled()=>Err("CANCELLED".into()),r=self.run(store,&work,browser,model)=>r};
        self.end(store, &work, result.as_ref().err().map(String::as_str))?;
        guard.done = true;
        result
    }
    pub(crate) fn shutdown(&self, store: &Store) -> AppResult<()> {
        self.stop_memory();
        let _native = store.native_actions.lock().map_err(err)?;
        let knowledge_error = self.knowledge.shutdown_locked(store).err();
        let mut ids = self
            .entries
            .lock()
            .map_err(err)?
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        ids.extend(self.surfaces.lock().map_err(err)?.instances());
        ids.sort();
        ids.dedup();
        let mut failure = knowledge_error;
        for i in ids {
            if let Err(e) = self.cancel_locked(
                store,
                &i,
                None,
                Phase::Interrupted,
                "宿主正在退出，未自动补发未确认请求",
            ) {
                failure.get_or_insert(e);
            }
            self.surfaces.lock().map_err(err)?.revoke(&i);
        }
        if let Some(e) = failure {
            return Err(e);
        }
        Ok(())
    }
    pub(crate) fn kick(&self, app: &tauri::AppHandle, instance: &str) {
        let Ok(mut pending) = self.renders.lock() else {
            return;
        };
        if let Some(stamp) = pending.get_mut(instance) {
            *stamp = stamp.saturating_add(1);
            return;
        }
        pending.insert(instance.into(), 0);
        drop(pending);
        let handle = app.clone();
        let instance = instance.to_owned();
        tauri::async_runtime::spawn_blocking(move || loop {
            use tauri::Manager;
            let core = handle.state::<crate::Core>();
            let service = &core.jobs;
            let Some(stamp) = service
                .renders
                .lock()
                .ok()
                .and_then(|r| r.get(&instance).copied())
            else {
                break;
            };
            let result = service.render(&handle, &instance);
            if let Err(error) = result {
                if let Ok(mut r) = service.surfaces.lock() {
                    r.errors.insert(instance.clone(), error);
                }
            }
            let Ok(mut pending) = service.renders.lock() else {
                break;
            };
            if pending.get(&instance) == Some(&stamp) {
                pending.remove(&instance);
                break;
            }
        });
    }
    #[cfg(windows)]
    fn render(&self, app: &tauri::AppHandle, instance: &str) -> AppResult<()> {
        crate::job_surfaces::native::render(app, instance)
    }
    #[cfg(not(windows))]
    fn render(&self, _: &tauri::AppHandle, _: &str) -> AppResult<()> {
        Err("UNSUPPORTED: AI直聘原生页面当前仅支持Windows".into())
    }
    pub(crate) fn cleanup(&self, app: &tauri::AppHandle) -> AppResult<()> {
        let ids = self.surfaces.lock().map_err(err)?.instances();
        for id in ids {
            self.render(app, &id)?;
        }
        Ok(())
    }
    pub(crate) fn cleanup_instance(&self, app: &tauri::AppHandle, instance: &str) -> AppResult<()> {
        if self
            .surfaces
            .lock()
            .map_err(err)?
            .instances()
            .iter()
            .any(|id| id == instance)
        {
            self.render(app, instance)?;
        }
        Ok(())
    }
    pub(crate) fn tick(&self, app: &tauri::AppHandle) {
        use tauri::Manager;
        let core = app.state::<crate::Core>();
        if self.stopped() {
            return;
        }
        let mut ids = self
            .surfaces
            .lock()
            .map(|r| r.instances())
            .unwrap_or_default();
        ids.extend(
            self.entries
                .lock()
                .map(|e| e.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default(),
        );
        if let Ok(db) = core.store.db.lock() {
            if let Ok(mut stmt) = db.prepare("SELECT id FROM widget_instances WHERE package_id=?1")
            {
                if let Ok(rows) = stmt.query_map([job_protocol::PACKAGE], |r| r.get::<_, String>(0))
                {
                    ids.extend(rows.filter_map(Result::ok));
                }
            }
        }
        ids.sort();
        ids.dedup();
        for id in ids {
            let result = (|| -> AppResult<()> {
                let _native = core.store.native_actions.lock().map_err(err)?;
                match source(&core.store, &id) {
                    Ok(src) => {
                        if (!src.visible || src.hidden) && !src.control.config.background_enabled {
                            self.knowledge.cancel_instance_locked(&core.store, &id)?;
                            self.cancel_locked(
                                &core.store,
                                &id,
                                None,
                                Phase::Stopped,
                                "组件已隐藏，未开启后台继续",
                            )?;
                        }
                        self.reconcile_locked(&core.store, &id)?;
                        self.knowledge.recover_instance_locked(&core.store, &id)
                    }
                    Err(_) => self.revoke(&id),
                }
            })();
            if let Err(error) = result {
                if let Ok(mut r) = self.surfaces.lock() {
                    r.errors.insert(id.clone(), error);
                }
            }
            if self.surfaces.lock().is_ok_and(|r| {
                !r.list(&id).is_empty() || r.retired.iter().any(|s| s.owner.instance == id)
            }) {
                self.kick(app, &id);
            }
        }
    }
}
pub(crate) async fn gateway(
    app: &tauri::AppHandle,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    if method.starts_with("jobs.knowledge.") {
        return crate::job_knowledge_host::gateway(app, window, token, method, args).await;
    }
    use tauri::Manager;
    let core = app.state::<crate::Core>();
    if core.quitting.load(Ordering::Acquire) {
        return Err("SESSION_EXPIRED: 宿主正在退出".into());
    }
    let instance = {
        let sessions = core.sessions.0.lock().map_err(err)?;
        sessions
            .get(token)
            .filter(|s| s.window == window && s.preview.is_none())
            .map(|s| s.instance.clone())
            .ok_or("SESSION_EXPIRED: AI直聘会话无效")?
    };
    let (value, work) =
        core.jobs
            .access(&core.store, &core.sessions, window, token, method, args)?;
    if method == "jobs.help" {
        let url = job_protocol::help_url(
            value["helpTopic"]
                .as_str()
                .ok_or("DATA_UNAVAILABLE: 帮助主题缺失")?,
        )?;
        crate::content::open_external(&url)?;
        return Ok(value);
    }
    if let Some(work) = work {
        core.active_tasks.fetch_add(1, Ordering::AcqRel);
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            struct Count(tauri::AppHandle);
            impl Drop for Count {
                fn drop(&mut self) {
                    use tauri::Manager;
                    self.0
                        .state::<crate::Core>()
                        .active_tasks
                        .fetch_sub(1, Ordering::AcqRel);
                }
            }
            let _count = Count(handle.clone());
            let core = handle.state::<crate::Core>();
            #[cfg(windows)]
            {
                let browser = job_webviews::native::Native {
                    app: handle.clone(),
                    book: core.jobs.views.clone(),
                };
                if let Err(error) = core
                    .jobs
                    .execute(&core.store, work, &browser, &HttpModel)
                    .await
                {
                    if error != "CANCELLED" {
                        core.report(error);
                    }
                }
            }
            #[cfg(not(windows))]
            {
                drop(work);
                core.report("UNSUPPORTED: AI直聘当前仅支持Windows".into());
            }
        });
    }
    if method != "jobs.status" && method != "jobs.query" {
        core.jobs.kick(app, &instance);
    }
    Ok(value)
}
fn empty(args: &Value) -> AppResult<()> {
    if args.is_null() || args.as_object().is_some_and(|o| o.is_empty()) {
        Ok(())
    } else {
        Err("INVALID_ARGUMENT: 此方法不接受参数".into())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, Arc};
    struct Fixture {
        store: Arc<Store>,
        sessions: Arc<widgets::Sessions>,
        service: Arc<Service>,
        instance: String,
        window: String,
        token: String,
    }
    fn uid() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn fixture(background: bool) -> Fixture {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("artifacts")
            .join(format!(
                "desktop-integration-widgetjs-ai-job-core-{}",
                uid()
            ));
        let store = Arc::new(Store::open(root).unwrap());
        store
            .save_preference("network", "{\"mode\":\"direct\"}")
            .unwrap();
        let manifest = json!({"schemaVersion":1,"id":job_protocol::PACKAGE,"name":"AI直聘离线检查","version":"0.1.0","hostApiMajor":1,"entry":"dist/index.html","defaultSize":{"width":600,"height":450},"minSize":{"width":300,"height":200},"capabilities":[job_protocol::CAPABILITY],"settingsSchema":{"type":"object","additionalProperties":false,"properties":{}}});
        store.db.lock().unwrap().execute("INSERT INTO widget_packages(id,manifest,directory,digest) VALUES(?1,?2,'job-fixture',?3)",rusqlite::params![job_protocol::PACKAGE,manifest.to_string(),"a".repeat(64)]).unwrap();
        let instance = widgets::create_instance(&store, job_protocol::PACKAGE).unwrap();
        let window = format!("float-{instance}");
        let sessions = Arc::new(widgets::Sessions::default());
        let session =
            widgets::open_display_session(&store, &sessions, &window, &instance, None).unwrap();
        let token = session["token"].as_str().unwrap().to_owned();
        let service = Arc::new(Service::default());
        let f = Fixture {
            store,
            sessions,
            service,
            instance,
            window,
            token,
        };
        f.call("jobs.configure",json!({"requestId":uid(),"expectedRevision":0,"config":{"search":{"skills":"Vue","selfIntroduction":"有开发经验","desiredSalary":"15-25K","location":"杭州","searchKeyword":"开发","disadvantages":"","advantages":["双休"]},"backgroundEnabled":background}})).unwrap();
        f.model_config(0, "https://api.example.com/v1").unwrap();
        f.service
            .views
            .lock()
            .unwrap()
            .attach(
                Owner {
                    instance: f.instance.clone(),
                    presentation: uid(),
                    digest: "a".repeat(64),
                },
                uid(),
            )
            .unwrap();
        f
    }
    impl Fixture {
        fn call(&self, method: &str, args: Value) -> AppResult<(Value, Option<Work>)> {
            self.service.access(
                &self.store,
                &self.sessions,
                &self.window,
                &self.token,
                method,
                args,
            )
        }
        fn model_config(&self, revision: u64, url: &str) -> AppResult<Value> {
            self.call("jobs.model.configure",json!({"requestId":uid(),"expectedRevision":revision,"apiBaseUrl":url,"apiKey":"fixture-only-no-network","model":"fixture-model","enabled":true})).map(|v|v.0)
        }
        fn start(&self) -> Work {
            let c = source(&self.store, &self.instance).unwrap().control;
            let mut w=self.call("jobs.start",json!({"requestId":uid(),"configRevision":c.revision,"generation":c.generation,"modelRevision":1})).unwrap().1.unwrap();
            w.pace = Duration::ZERO;
            w
        }
        fn run(&self) -> job_state::Run {
            job_state::load(&self.store.db.lock().unwrap(), &self.instance)
                .unwrap()
                .run
                .unwrap()
        }
    }
    #[derive(Default)]
    struct FakeBrowser {
        calls: Mutex<Vec<u32>>,
        pages: usize,
        partial: bool,
    }
    impl Browser for FakeBrowser {
        type Capture = Value;
        fn commit<T>(
            &self,
            capture: Value,
            apply: impl FnOnce(Value) -> AppResult<T>,
        ) -> AppResult<T> {
            apply(capture)
        }
        fn page<'a>(
            &'a self,
            _: Owner,
            _: String,
            page: u32,
            cancel: CancellationToken,
        ) -> Pending<'a, Value> {
            Box::pin(async move {
                if cancel.is_cancelled() {
                    return Err("CANCELLED".into());
                }
                self.calls.lock().unwrap().push(page);
                let total = if self.pages > 1 { 31 } else { 1 };
                let count = if page == 1 && self.pages > 1 && !self.partial {
                    30
                } else {
                    1
                };
                let jobs=(0..count).map(|n|json!({"encryptJobId":if count==1{format!("job{page}")}else{format!("job{page}-{n}")},"jobName":"开发工程师","skills":["Vue"],"brandName":"离线样本"})).collect::<Vec<_>>();
                Ok(
                    json!({"origin":"https://www.zhipin.com","path":"/web/geek/job","loggedIn":true,"page":page,"pageSize":30,"total":total,"jobs":jobs}),
                )
            })
        }
    }
    #[derive(Default)]
    struct FakeModel {
        calls: AtomicUsize,
        invalid: bool,
        hook: Option<Box<dyn Fn() -> AppResult<()> + Send + Sync>>,
    }
    impl Model for FakeModel {
        fn stream<'a>(
            &'a self,
            _: &'a NetworkConfig,
            profile: &'a Profile,
            body: Value,
            cancel: &'a CancellationToken,
            mut update: Update<'a>,
        ) -> Pending<'a, String> {
            Box::pin(async move {
                if cancel.is_cancelled() {
                    return Err("CANCELLED".into());
                }
                self.calls.fetch_add(1, Ordering::AcqRel);
                assert_eq!(profile.key, "fixture-only-no-network");
                assert!(!body.to_string().contains(&profile.key));
                update("评分片段")?;
                if let Some(hook) = &self.hook {
                    hook()?;
                }
                let jobs: Vec<Value> =
                    serde_json::from_str(body["messages"][2]["content"].as_str().unwrap()).unwrap();
                let results=jobs.into_iter().map(|j|json!({"encryptJobId":if self.invalid{json!("foreign")}else{j["encryptJobId"].clone()},"salaryMatch":true,"degreeMatch":true,"comment":"离线样本满足部分要求","score":4})).collect::<Vec<_>>();
                let output = json!({"jobs":results}).to_string();
                update(&output)?;
                Ok(output)
            })
        }
    }
    #[tokio::test]
    async fn fixed_search_scores_each_page_then_completes_without_a_web_timer() {
        let f = fixture(false);
        let w = f.start();
        let browser = FakeBrowser {
            pages: 2,
            ..Default::default()
        };
        let model = FakeModel::default();
        f.service
            .execute(&f.store, w, &browser, &model)
            .await
            .unwrap();
        assert_eq!(*browser.calls.lock().unwrap(), [1, 2]);
        assert_eq!(model.calls.load(Ordering::Acquire), 2);
        let r = f.run();
        assert_eq!(r.phase, Phase::Completed);
        assert_eq!((r.seen, r.rated), (31, 31));
        let value = f
            .call(
                "jobs.query",
                json!({"searchKeyword":"开发","score":4,"page":1,"pageSize":20}),
            )
            .unwrap()
            .0;
        assert_eq!(value["total"], 31);
        assert!(!value.to_string().contains("fixture-only-no-network"));
        let c = source(&f.store, &f.instance).unwrap().control;
        let replay=f.call("jobs.start",json!({"requestId":r.id,"configRevision":c.revision,"generation":c.generation-1,"modelRevision":1})).unwrap();
        assert!(replay.1.is_none());
        assert_eq!(replay.0["replayed"], true);
        assert_eq!(model.calls.load(Ordering::Acquire), 2);
    }
    #[tokio::test]
    async fn invalid_model_batch_never_writes_a_score_or_automatically_retries() {
        let f = fixture(false);
        let w = f.start();
        let browser = FakeBrowser::default();
        let model = FakeModel {
            invalid: true,
            ..Default::default()
        };
        assert!(f
            .service
            .execute(&f.store, w, &browser, &model)
            .await
            .is_err());
        assert_eq!(model.calls.load(Ordering::Acquire), 1);
        let r = f.run();
        assert_eq!(r.phase, Phase::Failed);
        assert!(r.uncertain_request());
        assert!(
            job_state::row(&f.store.db.lock().unwrap(), &f.instance, "job1")
                .unwrap()
                .unwrap()
                .scored
                .is_none()
        );
    }
    #[tokio::test]
    async fn reservation_failure_does_not_issue_billable_work() {
        let f = fixture(false);
        let w = f.start();
        f.store.db.lock().unwrap().execute_batch("CREATE TEMP TRIGGER reject_reservation BEFORE UPDATE ON app_settings WHEN NEW.key LIKE 'job.control/%' AND json_extract(NEW.value,'$.run.phase')='waiting_model' BEGIN SELECT RAISE(ABORT,'故障注入'); END;").unwrap();
        let browser = FakeBrowser::default();
        let model = FakeModel::default();
        assert!(f
            .service
            .execute(&f.store, w, &browser, &model)
            .await
            .is_err());
        assert_eq!(model.calls.load(Ordering::Acquire), 0);
        assert_eq!(f.run().phase, Phase::Failed);
        assert!(!f.run().uncertain_request());
    }
    #[tokio::test]
    async fn closing_an_unapproved_background_window_cancels_the_current_model_and_result() {
        let f = fixture(false);
        let w = f.start();
        let service = f.service.clone();
        let store = f.store.clone();
        let window = f.window.clone();
        let model = FakeModel {
            hook: Some(Box::new(move || service.closed_window(&store, &window))),
            ..Default::default()
        };
        assert!(f
            .service
            .execute(&f.store, w, &FakeBrowser::default(), &model)
            .await
            .is_err());
        assert_eq!(f.run().phase, Phase::Stopped);
        assert!(f.run().uncertain_request());
        assert_eq!(model.calls.load(Ordering::Acquire), 1);
        assert!(
            job_state::row(&f.store.db.lock().unwrap(), &f.instance, "job1")
                .unwrap()
                .unwrap()
                .scored
                .is_none()
        );
    }
    #[tokio::test]
    async fn explicitly_enabled_background_continues_after_the_issuer_view_closes() {
        let f = fixture(true);
        let w = f.start();
        let service = f.service.clone();
        let store = f.store.clone();
        let window = f.window.clone();
        let model = FakeModel {
            hook: Some(Box::new(move || service.closed_window(&store, &window))),
            ..Default::default()
        };
        f.service
            .execute(&f.store, w, &FakeBrowser::default(), &model)
            .await
            .unwrap();
        assert_eq!(f.run().phase, Phase::Completed);
        assert_eq!(f.run().rated, 1);
    }
    #[test]
    fn rejected_configuration_does_not_cancel_the_running_task_but_a_saved_model_change_does() {
        let f = fixture(false);
        let w = f.start();
        assert!(f.model_config(1, "http://evil.example/v1").is_err());
        assert!(!w.cancel.is_cancelled());
        assert!(f
            .call(
                "jobs.configure",
                json!({"requestId":uid(),"expectedRevision":1,"config":{"script":"不接受"}})
            )
            .is_err());
        assert!(!w.cancel.is_cancelled());
        assert_eq!(f.run().phase, Phase::WaitingPage);
        f.model_config(1, "https://api2.example.com/v1").unwrap();
        assert!(w.cancel.is_cancelled());
        assert_eq!(f.run().phase, Phase::Stopped);
    }
    #[tokio::test]
    async fn package_replacement_rejects_inflight_model_updates_and_scores() {
        let f = fixture(false);
        let w = f.start();
        let store = f.store.clone();
        let model = FakeModel {
            hook: Some(Box::new(move || {
                store
                    .db
                    .lock()
                    .map_err(err)?
                    .execute(
                        "UPDATE widget_packages SET digest=?1 WHERE id=?2",
                        rusqlite::params!["b".repeat(64), job_protocol::PACKAGE],
                    )
                    .map_err(err)?;
                Ok(())
            })),
            ..Default::default()
        };
        assert!(f
            .service
            .execute(&f.store, w, &FakeBrowser::default(), &model)
            .await
            .is_err());
        assert_eq!(model.calls.load(Ordering::Acquire), 1);
        assert_eq!(f.run().phase, Phase::Failed);
        assert!(
            job_state::row(&f.store.db.lock().unwrap(), &f.instance, "job1")
                .unwrap()
                .unwrap()
                .scored
                .is_none()
        );
    }
    #[tokio::test]
    async fn explicit_rerating_needs_no_boss_capture_and_only_scores_the_selected_job() {
        let f = fixture(false);
        let w = f.start();
        let browser = FakeBrowser::default();
        f.service
            .execute(&f.store, w, &browser, &FakeModel::default())
            .await
            .unwrap();
        let owner = f.service.views.lock().unwrap().owner(&f.instance).unwrap();
        f.service.views.lock().unwrap().detach(&owner);
        let c = source(&f.store, &f.instance).unwrap().control;
        let w=f.call("jobs.rerate",json!({"requestId":uid(),"configRevision":c.revision,"generation":c.generation,"modelRevision":1,"jobId":"job1"})).unwrap().1.unwrap();
        let model = FakeModel::default();
        f.service
            .execute(&f.store, w, &browser, &model)
            .await
            .unwrap();
        assert_eq!(*browser.calls.lock().unwrap(), [1]);
        assert_eq!(model.calls.load(Ordering::Acquire), 1);
        assert_eq!(f.run().rated, 1);
    }
    #[test]
    fn status_recovers_orphaned_work_as_interrupted_and_never_returns_a_new_work_item() {
        let f = fixture(false);
        let request = uid();
        job_state::start(
            &mut f.store.db.lock().unwrap(),
            &f.instance,
            &request,
            1,
            0,
            1,
            1,
        )
        .unwrap();
        let (v, w) = f.call("jobs.status", Value::Null).unwrap();
        assert!(w.is_none());
        assert_eq!(v["run"]["phase"], "interrupted");
        assert_eq!(f.run().id, request);
        assert!(!v.to_string().contains("fixture-only-no-network"));
    }
    #[test]
    fn cross_window_or_arbitrary_script_requests_cannot_inject_code_or_read_a_key() {
        let f = fixture(false);
        for topic in ["tutorial", "deepseek", "qwen", "spark", "tencent"] {
            assert!(f
                .call("jobs.help", json!({"topic":topic}))
                .unwrap()
                .1
                .is_none());
        }
        for args in [
            json!({"topic":"https://evil.example"}),
            json!({"topic":"tutorial","url":"https://evil.example"}),
        ] {
            assert!(f.call("jobs.help", args).is_err());
        }
        assert!(f
            .service
            .access(
                &f.store,
                &f.sessions,
                "other-window",
                &f.token,
                "jobs.model.configure",
                json!({})
            )
            .is_err());
        for method in ["jobs.eval", "jobs.fetch", "jobs.capture", "jobs.complete"] {
            assert!(f
                .call(
                    method,
                    json!({"script":"不接受","url":"https://evil.example/"})
                )
                .is_err());
        }
        assert!(f.call("jobs.start",json!({"requestId":uid(),"configRevision":1,"generation":0,"modelRevision":1,"script":"不接受"})).is_err());
        assert!(source(&f.store, &f.instance).unwrap().control.run.is_none());
    }
    #[test]
    fn shutdown_cancels_immediately_and_prevents_further_methods() {
        let f = fixture(false);
        let w = f.start();
        f.service.shutdown(&f.store).unwrap();
        assert!(w.cancel.is_cancelled());
        assert_eq!(f.run().phase, Phase::Interrupted);
        assert!(f.call("jobs.status", Value::Null).is_err());
    }
    struct BlockingModel {
        started: Arc<tokio::sync::Notify>,
    }
    impl Model for BlockingModel {
        fn stream<'a>(
            &'a self,
            _: &'a NetworkConfig,
            _: &'a Profile,
            _: Value,
            _: &'a CancellationToken,
            mut update: Update<'a>,
        ) -> Pending<'a, String> {
            Box::pin(async move {
                update("等待离线结果")?;
                self.started.notify_one();
                std::future::pending().await
            })
        }
    }
    #[tokio::test]
    async fn aborting_a_worker_future_persists_interruption_and_preserves_the_uncertain_request() {
        let f = fixture(false);
        let w = f.start();
        let started = Arc::new(tokio::sync::Notify::new());
        let signal = started.clone();
        let service = f.service.clone();
        let store = f.store.clone();
        let task = tokio::spawn(async move {
            service
                .execute(
                    &store,
                    w,
                    &FakeBrowser::default(),
                    &BlockingModel { started: signal },
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), started.notified())
            .await
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(f.run().phase, Phase::Interrupted);
        assert!(f.run().uncertain_request());
    }
    #[tokio::test]
    async fn an_incomplete_page_is_not_scored_or_skipped_to_the_next_page() {
        let f = fixture(false);
        let w = f.start();
        let browser = FakeBrowser {
            pages: 2,
            partial: true,
            ..Default::default()
        };
        let model = FakeModel::default();
        assert!(f
            .service
            .execute(&f.store, w, &browser, &model)
            .await
            .is_err());
        assert_eq!(*browser.calls.lock().unwrap(), [1]);
        assert_eq!(model.calls.load(Ordering::Acquire), 0);
        assert_eq!(f.run().seen, 0);
        assert!(
            job_state::row(&f.store.db.lock().unwrap(), &f.instance, "job1")
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn dropping_unstarted_work_revokes_its_in_memory_permission_and_recovers_as_interrupted() {
        let f = fixture(false);
        let w = f.start();
        let request = w.id.clone();
        drop(w);
        let (value, work) = f.call("jobs.status", Value::Null).unwrap();
        assert!(work.is_none());
        assert_eq!(value["execution"]["active"], false);
        assert_eq!(value["run"]["phase"], "interrupted");
        assert_eq!(f.run().id, request);
    }
    fn shell(f: &Fixture, page: Page) -> (Surface, String) {
        f.call("jobs.open", serde_json::to_value(&page).unwrap())
            .unwrap();
        let surface = f
            .service
            .surfaces
            .lock()
            .unwrap()
            .list(&f.instance)
            .into_iter()
            .find(|s| s.page == page)
            .unwrap();
        let value = widgets::open_display_session(
            &f.store,
            &f.sessions,
            &surface.shell(),
            &f.instance,
            None,
        )
        .unwrap();
        let token = value["token"].as_str().unwrap().to_owned();
        f.service
            .bind(&f.store, &surface.shell(), &f.instance, &token)
            .unwrap();
        (surface, token)
    }
    #[test]
    fn dedicated_shells_preserve_the_primary_card_and_other_knowledge_contexts() {
        let f = fixture(false);
        let a = job_surfaces::Knowledge {
            knowledge: "Vue".into(),
            job_name: "开发".into(),
            industry: "互联网".into(),
            extra: "性能".into(),
        };
        let mut b = a.clone();
        b.extra = "测试".into();
        let (sa, ta) = shell(&f, Page::Knowledge { context: a });
        let (sb, tb) = shell(&f, Page::Knowledge { context: b });
        assert!(f.sessions.0.lock().unwrap().contains_key(&f.token));
        assert!(f.sessions.0.lock().unwrap().contains_key(&ta));
        assert!(f.sessions.0.lock().unwrap().contains_key(&tb));
        let info = f
            .service
            .access(
                &f.store,
                &f.sessions,
                &sa.shell(),
                &ta,
                "jobs.surface.info",
                Value::Null,
            )
            .unwrap()
            .0;
        assert_eq!(info["surface"]["page"]["context"]["extra"], "性能");
        let value =
            widgets::open_display_session(&f.store, &f.sessions, &sa.shell(), &f.instance, None)
                .unwrap();
        let next = value["token"].as_str().unwrap();
        f.service
            .bind(&f.store, &sa.shell(), &f.instance, next)
            .unwrap();
        assert!(!f.sessions.0.lock().unwrap().contains_key(&ta));
        assert!(f.sessions.0.lock().unwrap().contains_key(&tb));
        assert!(f.service.current_window(&sb.shell(), &f.instance));
    }
    #[test]
    fn obsolete_shell_versions_and_invalid_window_actions_cannot_write_settings() {
        let f = fixture(false);
        let (s, t) = shell(&f, Page::Search);
        for method in ["jobs.surface.layout", "jobs.surface.overlay"] {
            assert!(f
                .service
                .access(
                    &f.store,
                    &f.sessions,
                    &f.window,
                    &f.token,
                    method,
                    json!({})
                )
                .is_err());
        }
        f.store
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE widget_packages SET digest=?1 WHERE id=?2",
                rusqlite::params!["b".repeat(64), job_protocol::PACKAGE],
            )
            .unwrap();
        assert!(f.service.access(&f.store,&f.sessions,&s.shell(),&t,"jobs.configure",json!({"requestId":uid(),"expectedRevision":1,"config":job_state::Config::default()})).is_err());
        assert_eq!(source(&f.store, &f.instance).unwrap().control.revision, 1);
    }
    #[tokio::test]
    async fn closing_a_shell_revokes_the_worker_before_a_late_model_result_is_saved() {
        let f = fixture(false);
        let (s, t) = shell(&f, Page::Search);
        let request = uid();
        let w = f
            .service
            .access(
                &f.store,
                &f.sessions,
                &s.shell(),
                &t,
                "jobs.start",
                json!({"requestId":request,"configRevision":1,"generation":0,"modelRevision":1}),
            )
            .unwrap()
            .1
            .unwrap();
        let service = f.service.clone();
        let store = f.store.clone();
        let sessions = f.sessions.clone();
        let label = s.shell();
        let token = t.clone();
        let model = FakeModel {
            hook: Some(Box::new(move || {
                service
                    .access(
                        &store,
                        &sessions,
                        &label,
                        &token,
                        "jobs.surface.close",
                        Value::Null,
                    )
                    .map(|_| ())
            })),
            ..Default::default()
        };
        assert!(f
            .service
            .execute(&f.store, w, &FakeBrowser::default(), &model)
            .await
            .is_err());
        assert_eq!(f.run().phase, Phase::Stopped);
        assert!(f.run().uncertain_request());
        assert!(!f.service.current_window(&s.shell(), &f.instance));
        assert!(f
            .service
            .access(
                &f.store,
                &f.sessions,
                &s.shell(),
                &t,
                "jobs.model.clear",
                json!({"requestId":uid(),"expectedRevision":1})
            )
            .is_err());
    }
    #[test]
    fn hidden_instances_cannot_start_new_operations_and_background_permission_is_respected() {
        let f = fixture(false);
        let w = f.start();
        f.store
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE widget_instances SET visible=0 WHERE id=?1",
                [&f.instance],
            )
            .unwrap();
        f.service.hide_instance(&f.store, &f.instance).unwrap();
        assert!(w.cancel.is_cancelled());
        assert_eq!(f.run().phase, Phase::Stopped);
        assert!(f.call("jobs.open", json!({"page":"search"})).is_err());
        let f = fixture(true);
        let w = f.start();
        f.store
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE widget_instances SET visible=0 WHERE id=?1",
                [&f.instance],
            )
            .unwrap();
        f.service.hide_instance(&f.store, &f.instance).unwrap();
        assert!(!w.cancel.is_cancelled());
        assert!(f.service.current(&f.store, &w).is_ok());
    }
    #[test]
    fn credential_mutation_preview_and_arbitrary_window_creation_are_denied() {
        let f = fixture(false);
        let bytes = crate::presets::find("local.fixture.surface")
            .unwrap()
            .unwrap()
            .archive()
            .unwrap();
        let package = widgets::stage_bytes(&f.store, bytes).unwrap();
        let preview = Arc::new(Mutex::new(widgets::Preview {
            settings: json!({}),
            storage: json!({}),
            package,
            ready_since: None,
            failure: None,
        }));
        let token = uid();
        f.sessions.0.lock().unwrap().insert(
            token.clone(),
            widgets::Session {
                instance: f.instance.clone(),
                window: f.window.clone(),
                preview: Some(preview),
                shared_cursor: None,
            },
        );
        for (method, args) in [
            (
                "jobs.model.configure",
                json!({"requestId":uid(),"expectedRevision":1,"apiBaseUrl":"https://other.example/v1","apiKey":"不可写入","model":"m","enabled":true}),
            ),
            ("jobs.open", json!({"page":"search"})),
            (
                "jobs.start",
                json!({"requestId":uid(),"configRevision":1,"generation":0,"modelRevision":1}),
            ),
        ] {
            assert!(
                matches!(f.service.access(&f.store,&f.sessions,&f.window,&token,method,args),Err(error) if error.starts_with("CAPABILITY_DENIED:"))
            );
        }
        assert_eq!(
            ai_credentials::status(&f.store, &f.instance).unwrap()["revision"],
            1
        );
        assert!(f
            .service
            .surfaces
            .lock()
            .unwrap()
            .list(&f.instance)
            .is_empty());
        assert!(source(&f.store, &f.instance).unwrap().control.run.is_none());
    }
    #[tokio::test]
    async fn deleting_an_instance_revokes_shells_and_clears_only_its_job_and_model_data() {
        let f = fixture(false);
        let (s, t) = shell(&f, Page::Analysis);
        let w = f.start();
        f.service
            .execute(&f.store, w, &FakeBrowser::default(), &FakeModel::default())
            .await
            .unwrap();
        let other = widgets::create_instance(&f.store, job_protocol::PACKAGE).unwrap();
        let config = job_state::Config {
            search: job_protocol::Search {
                search_keyword: "其他实例".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        job_state::configure(
            &mut f.store.db.lock().unwrap(),
            &other,
            &uid(),
            0,
            config,
            1,
        )
        .unwrap();
        widgets::remove_instance_before(&f.store, &f.sessions, &f.instance, || {
            f.service.revoke(&f.instance)
        })
        .unwrap();
        let db = f.store.db.lock().unwrap();
        let count: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM app_settings WHERE key=?1 OR key LIKE ?2 OR key=?3",
                rusqlite::params![
                    format!("job.control/{}", f.instance),
                    format!("job.row/{}/%", f.instance),
                    format!("ai.credential/{}", f.instance)
                ],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(job_state::load(&db, &other).unwrap().revision, 1);
        drop(db);
        assert!(!f.service.current_window(&s.shell(), &f.instance));
        assert!(!f.sessions.0.lock().unwrap().contains_key(&t));
        assert!(
            f.service.surfaces.lock().unwrap().snapshot(&f.instance)["nativeCleanupPending"]
                .as_bool()
                .unwrap()
        );
    }
    #[test]
    fn closing_waits_for_the_current_commit_guard_before_revoking_the_task() {
        let f = fixture(false);
        let (surface, _) = shell(&f, Page::Search);
        let w = f.start();
        let hold = f.service.hold_entry(&w).unwrap();
        let service = f.service.clone();
        let (started, seen) = std::sync::mpsc::sync_channel(1);
        let (done, finished) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || {
            started.send(()).unwrap();
            let result = service.closed_memory(&surface, true, false);
            done.send(result).unwrap();
        });
        seen.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            finished.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        assert!(!w.cancel.is_cancelled());
        drop(hold);
        finished
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        thread.join().unwrap();
        assert!(w.cancel.is_cancelled());
        f.call("jobs.status", Value::Null).unwrap();
        assert_eq!(f.run().phase, Phase::Stopped);
    }
    #[tokio::test]
    async fn knowledge_save_failure_during_shutdown_still_revokes_the_search_and_every_window() {
        let f = fixture(false);
        let context = job_surfaces::Knowledge {
            knowledge: "Vue".into(),
            job_name: "开发".into(),
            industry: "互联网".into(),
            extra: String::new(),
        };
        let (s, t) = shell(&f, Page::Knowledge { context });
        let chat = f
            .service
            .knowledge
            .access(
                &f.store,
                &f.sessions,
                &f.service.surfaces,
                &s.shell(),
                &t,
                "jobs.knowledge.generate",
                json!({"kind":"wiki","requestId":uid(),"expectedRevision":0,"modelRevision":1}),
            )
            .unwrap()
            .1
            .unwrap();
        let search = f.start();
        f.store.db.lock().unwrap().execute_batch("CREATE TEMP TRIGGER refuse_chat_stop BEFORE UPDATE ON app_settings WHEN NEW.key LIKE 'job.chat/%' BEGIN SELECT RAISE(ABORT,'故障注入'); END;").unwrap();
        assert!(f.service.shutdown(&f.store).is_err());
        assert!(search.cancel.is_cancelled());
        assert_eq!(f.run().phase, Phase::Interrupted);
        assert!(!f.service.current_window(&s.shell(), &f.instance));
        assert_eq!(
            f.service.surfaces.lock().unwrap().snapshot(&f.instance)["nativeCleanupPending"],
            true
        );
        let model = FakeModel::default();
        assert!(f
            .service
            .knowledge
            .execute(&f.store, &f.service.surfaces, chat, &model)
            .await
            .is_err());
        assert_eq!(model.calls.load(Ordering::Acquire), 0);
    }
}
