//! 知识任务只接受当前知识窗口的上下文，密钥不进入记录或前端快照。
use crate::{
    ai_credentials::{self, Profile},
    ai_protocol, business, job_host,
    job_knowledge::{self as data, Kind, Phase},
    job_surfaces::{Knowledge, Page, Registry, Surface},
    network::NetworkConfig,
    store::{err, now_ms, AppResult, Store},
    widgets,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
const EXPIRED: &str = "PRESENTATION_EXPIRED: 知识任务或模型配置已改变，旧结果未写入";
fn scope(instance: &str, context: &Knowledge, kind: Kind) -> AppResult<String> {
    Ok(format!("{instance}/{}/{}", context.id()?, kind.name()))
}
struct Active {
    instance: String,
    window: String,
    context: Knowledge,
    kind: Kind,
    id: String,
    model_revision: u64,
    cancel: CancellationToken,
    partial: String,
    ending: Option<Phase>,
    saved: usize,
    saved_at: Instant,
}
pub(crate) struct Work {
    instance: String,
    window: String,
    context: Knowledge,
    kind: Kind,
    id: String,
    scope: String,
    digest: String,
    model_revision: u64,
    profile: Profile,
    body: Value,
    network: NetworkConfig,
    cancel: CancellationToken,
}
impl Drop for Work {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
#[derive(Default)]
pub(crate) struct Service {
    active: Mutex<HashMap<String, Active>>,
    stopped: AtomicBool,
}
impl Service {
    fn snapshot(&self, d: &data::Document, instance: &str) -> AppResult<Value> {
        let mut v = d.snapshot();
        let key = scope(instance, &d.context, d.kind)?;
        if let Some(a) = self.active.lock().map_err(err)?.get(&key).filter(|a| {
            !a.cancel.is_cancelled()
                && d.job
                    .as_ref()
                    .is_some_and(|r| r.id == a.id && r.phase == Phase::Running)
        }) {
            v["job"]["partial"] = json!(a.partial);
        }
        Ok(v)
    }
    fn reconcile_one_locked(
        &self,
        store: &Store,
        instance: &str,
        context: &Knowledge,
        kind: Kind,
    ) -> AppResult<data::Document> {
        let d = data::load(&*store.db.lock().map_err(err)?, instance, context, kind)?;
        if let Some(job) = d.job.as_ref().filter(|r| r.phase == Phase::Running) {
            let current = self
                .active
                .lock()
                .map_err(err)?
                .get(&scope(instance, context, kind)?)
                .map(|a| {
                    (
                        a.id.clone(),
                        a.cancel.is_cancelled(),
                        a.ending,
                        a.partial.clone(),
                    )
                });
            if let Some((id, cancelled, ending, partial)) = current.filter(|a| a.0 == job.id) {
                if !cancelled {
                    return Ok(d);
                }
                return data::update(
                    &mut *store.db.lock().map_err(err)?,
                    instance,
                    context,
                    kind,
                    &id,
                    job.model_revision,
                    &partial,
                    ending.unwrap_or(Phase::Cancelled),
                    Some("知识任务已取消，未自动补发请求"),
                    now_ms(),
                );
            }
            return data::recover(
                &mut *store.db.lock().map_err(err)?,
                instance,
                context,
                kind,
                now_ms(),
            );
        }
        Ok(d)
    }
    pub(crate) fn access(
        &self,
        store: &Store,
        sessions: &widgets::Sessions,
        surfaces: &Mutex<Registry>,
        window: &str,
        token: &str,
        method: &str,
        args: Value,
    ) -> AppResult<(Value, Option<Work>)> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("SESSION_EXPIRED: 知识服务正在退出".into());
        }
        if serde_json::to_vec(&args).map_err(err)?.len() > 64 * 1024 {
            return Err("CAPACITY_EXCEEDED: 知识请求超过容量".into());
        }
        let _native = store.native_actions.lock().map_err(err)?;
        widgets::with_capability(
            store,
            sessions,
            window,
            token,
            crate::job_protocol::CAPABILITY,
            |session| {
                let instance = &session.instance;
                let src = job_host::source(store, instance)?;
                let surface = surfaces.lock().map_err(err)?.authorized(window, token)?;
                if surface.owner.instance != *instance || surface.owner.digest != src.digest {
                    return Err(EXPIRED.into());
                }
                let Page::Knowledge { context } = &surface.page else {
                    return Err("CAPABILITY_DENIED: 只有当前知识窗口可访问聊天与生成".into());
                };
                match method {
                    "jobs.knowledge.get" => {
                        #[derive(Deserialize)]
                        #[serde(deny_unknown_fields)]
                        struct Input {
                            kind: Kind,
                        }
                        let input: Input = serde_json::from_value(args)
                            .map_err(|_| "INVALID_ARGUMENT: 读取知识只接受kind")?;
                        let d = self.reconcile_one_locked(store, instance, context, input.kind)?;
                        Ok((self.snapshot(&d, instance)?, None))
                    }
                    "jobs.knowledge.generate" => {
                        #[derive(Deserialize)]
                        #[serde(rename_all = "camelCase", deny_unknown_fields)]
                        struct Input {
                            kind: Kind,
                            request_id: String,
                            expected_revision: u64,
                            model_revision: u64,
                            text: Option<String>,
                        }
                        let input: Input = serde_json::from_value(args)
                            .map_err(|_| "INVALID_ARGUMENT: 知识生成参数无效")?;
                        if src.hidden || !src.visible || !surface.visible {
                            return Err("PRESENTATION_EXPIRED: 隐藏的知识窗口不启动新请求".into());
                        }
                        let (revision, profile) = ai_credentials::profile(store, instance)?;
                        if revision != input.model_revision {
                            return Err("REVISION_CONFLICT: 模型配置已有新版本".into());
                        }
                        let profile = profile
                            .filter(|p| p.enabled)
                            .ok_or("CONFIG_REQUIRED: 请先保存自己的模型连接并显式启用")?;
                        ai_protocol::endpoint(&profile.base_url)?;
                        let key = scope(instance, context, input.kind)?;
                        let mut active = self.active.lock().map_err(err)?;
                        if active
                            .values()
                            .filter(|a| a.instance == *instance && !a.cancel.is_cancelled())
                            .count()
                            >= 8
                            && !active.contains_key(&key)
                        {
                            return Err("BUSY: 当前同时生成任务过多，请先等待部分任务结束".into());
                        }
                        let network = business::resolve_network(
                            &serde_json::from_str::<Value>(
                                &store
                                    .preference("network")
                                    .unwrap_or("{\"mode\":\"system\"}".into()),
                            )
                            .map_err(|_| "DATA_UNAVAILABLE: 网络设置无效")?,
                        )?;
                        let reserved = data::reserve(
                            &mut *store.db.lock().map_err(err)?,
                            instance,
                            context,
                            input.kind,
                            &input.request_id,
                            input.expected_revision,
                            revision,
                            &src.digest,
                            &profile.model,
                            input.text.as_deref(),
                            now_ms(),
                        )?;
                        let work = if let Some(body) = reserved.body {
                            if let Some(old) = active.remove(&key) {
                                old.cancel.cancel();
                            }
                            let cancel = CancellationToken::new();
                            active.insert(
                                key.clone(),
                                Active {
                                    instance: instance.clone(),
                                    window: window.into(),
                                    context: context.clone(),
                                    kind: input.kind,
                                    id: input.request_id.clone(),
                                    model_revision: revision,
                                    cancel: cancel.clone(),
                                    partial: String::new(),
                                    ending: None,
                                    saved: 0,
                                    saved_at: Instant::now(),
                                },
                            );
                            Some(Work {
                                instance: instance.clone(),
                                window: window.into(),
                                context: context.clone(),
                                kind: input.kind,
                                id: input.request_id,
                                scope: key,
                                digest: src.digest,
                                model_revision: revision,
                                profile,
                                body,
                                network,
                                cancel,
                            })
                        } else {
                            None
                        };
                        drop(active);
                        let mut value = self.snapshot(&reserved.document, instance)?;
                        value["replayed"] = json!(work.is_none());
                        Ok((value, work))
                    }
                    "jobs.knowledge.clear" => {
                        #[derive(Deserialize)]
                        #[serde(rename_all = "camelCase", deny_unknown_fields)]
                        struct Input {
                            kind: Kind,
                            request_id: String,
                            expected_revision: u64,
                        }
                        let input: Input = serde_json::from_value(args)
                            .map_err(|_| "INVALID_ARGUMENT: 清空知识参数无效")?;
                        let (d, replayed) = data::clear(
                            &mut *store.db.lock().map_err(err)?,
                            instance,
                            context,
                            input.kind,
                            &input.request_id,
                            input.expected_revision,
                        )?;
                        if !replayed {
                            if let Some(a) = self
                                .active
                                .lock()
                                .map_err(err)?
                                .remove(&scope(instance, context, input.kind)?)
                            {
                                a.cancel.cancel();
                            }
                        }
                        let mut value = self.snapshot(&d, instance)?;
                        value["replayed"] = json!(replayed);
                        Ok((value, None))
                    }
                    "jobs.knowledge.stop" => {
                        #[derive(Deserialize)]
                        #[serde(rename_all = "camelCase", deny_unknown_fields)]
                        struct Input {
                            kind: Kind,
                            request_id: String,
                        }
                        let input: Input = serde_json::from_value(args)
                            .map_err(|_| "INVALID_ARGUMENT: 停止知识参数无效")?;
                        let d = data::load(
                            &*store.db.lock().map_err(err)?,
                            instance,
                            context,
                            input.kind,
                        )?;
                        if !d.job.as_ref().is_some_and(|r| r.id == input.request_id) {
                            return Err(EXPIRED.into());
                        }
                        if let Some(a) = self
                            .active
                            .lock()
                            .map_err(err)?
                            .get_mut(&scope(instance, context, input.kind)?)
                            .filter(|a| a.id == input.request_id)
                        {
                            a.cancel.cancel();
                            a.ending.get_or_insert(Phase::Cancelled);
                        }
                        let d = self.reconcile_one_locked(store, instance, context, input.kind)?;
                        Ok((self.snapshot(&d, instance)?, None))
                    }
                    _ => {
                        Err("CAPABILITY_DENIED: 未知知识方法，不接受任意提示词、地址或脚本".into())
                    }
                }
            },
        )
    }
    fn live(&self, store: &Store, surfaces: &Mutex<Registry>, w: &Work) -> AppResult<()> {
        if self.stopped.load(Ordering::Acquire) || w.cancel.is_cancelled() {
            return Err("CANCELLED".into());
        }
        if !self
            .active
            .lock()
            .map_err(err)?
            .get(&w.scope)
            .is_some_and(|a| a.id == w.id && !a.cancel.is_cancelled())
        {
            return Err(EXPIRED.into());
        }
        let src = job_host::source(store, &w.instance)?;
        if src.digest != w.digest {
            return Err(EXPIRED.into());
        }
        let (revision, profile) = ai_credentials::profile(store, &w.instance)?;
        if revision != w.model_revision || !profile.is_some_and(|p| p.enabled) {
            return Err("REVISION_CONFLICT: 模型配置已改变或停用".into());
        }
        if !src.control.config.background_enabled
            && (src.hidden
                || !src.visible
                || surfaces
                    .lock()
                    .map_err(err)?
                    .shell(&w.window, &w.instance)
                    .is_none())
        {
            return Err("CANCELLED".into());
        }
        Ok(())
    }
    fn push(
        &self,
        store: &Store,
        surfaces: &Mutex<Registry>,
        w: &Work,
        text: &str,
    ) -> AppResult<()> {
        if !data::valid_output(text) {
            return Err("CAPACITY_EXCEEDED: 知识输出超过512KiB或包含无效控制字符".into());
        }
        let _native = store.native_actions.lock().map_err(err)?;
        self.live(store, surfaces, w)?;
        let mut active = self.active.lock().map_err(err)?;
        let a = active
            .get_mut(&w.scope)
            .filter(|a| a.id == w.id && !a.cancel.is_cancelled())
            .ok_or(EXPIRED)?;
        a.partial = text.into();
        if text.len().saturating_sub(a.saved) >= 4096
            || a.saved_at.elapsed() >= Duration::from_secs(1)
        {
            data::update(
                &mut *store.db.lock().map_err(err)?,
                &w.instance,
                &w.context,
                w.kind,
                &w.id,
                w.model_revision,
                text,
                Phase::Running,
                None,
                now_ms(),
            )?;
            a.saved = text.len();
            a.saved_at = Instant::now();
        }
        Ok(())
    }
    fn finish(
        &self,
        store: &Store,
        surfaces: &Mutex<Registry>,
        w: &Work,
        result: AppResult<String>,
    ) -> AppResult<()> {
        let _native = store.native_actions.lock().map_err(err)?;
        let live = self.live(store, surfaces, w);
        let mut active = self.active.lock().map_err(err)?;
        let Some(a) = active.get(&w.scope).filter(|a| a.id == w.id) else {
            return Ok(());
        };
        let outcome = match (live, result) {
            (Ok(()), Ok(text)) if !a.cancel.is_cancelled() && a.ending.is_none() => Ok(text),
            (Err(e), _) | (_, Err(e)) => Err(e),
            _ => Err("CANCELLED".into()),
        };
        if job_host::source(store, &w.instance).is_ok() {
            let d = data::load(
                &*store.db.lock().map_err(err)?,
                &w.instance,
                &w.context,
                w.kind,
            )?;
            if d.job
                .as_ref()
                .is_some_and(|r| r.id == w.id && r.phase == Phase::Running)
            {
                match outcome {
                    Ok(text) => {
                        data::update(
                            &mut *store.db.lock().map_err(err)?,
                            &w.instance,
                            &w.context,
                            w.kind,
                            &w.id,
                            w.model_revision,
                            &text,
                            Phase::Completed,
                            None,
                            now_ms(),
                        )?;
                    }
                    Err(ref error) => {
                        let phase = a.ending.unwrap_or(if error == "CANCELLED" {
                            Phase::Cancelled
                        } else {
                            Phase::Failed
                        });
                        let message = if phase == Phase::Cancelled {
                            "知识生成已取消，保留未完成内容；未自动补发"
                        } else if phase == Phase::Interrupted {
                            "知识生成中断，保留未完成内容；未自动补发"
                        } else {
                            "知识生成失败，保留未完成内容；请检查模型和网络后手动决定是否重新生成"
                        };
                        data::update(
                            &mut *store.db.lock().map_err(err)?,
                            &w.instance,
                            &w.context,
                            w.kind,
                            &w.id,
                            w.model_revision,
                            &a.partial,
                            phase,
                            Some(message),
                            now_ms(),
                        )?;
                    }
                }
            }
        }
        if let Some(a) = active.remove(&w.scope) {
            a.cancel.cancel();
        }
        Ok(())
    }
    pub(crate) async fn execute(
        &self,
        store: &Store,
        surfaces: &Mutex<Registry>,
        work: Work,
        model: &impl job_host::Model,
    ) -> AppResult<()> {
        struct Guard<'a> {
            service: &'a Service,
            store: &'a Store,
            surfaces: &'a Mutex<Registry>,
            work: &'a Work,
            done: bool,
        }
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                if !self.done {
                    if let Ok(mut active) = self.service.active.lock() {
                        if let Some(a) = active
                            .get_mut(&self.work.scope)
                            .filter(|a| a.id == self.work.id)
                        {
                            a.cancel.cancel();
                            a.ending.get_or_insert(Phase::Interrupted);
                        }
                    }
                    let _ = self.service.finish(
                        self.store,
                        self.surfaces,
                        self.work,
                        Err("CANCELLED".into()),
                    );
                }
            }
        }
        let mut guard = Guard {
            service: self,
            store,
            surfaces,
            work: &work,
            done: false,
        };
        {
            let _native = store.native_actions.lock().map_err(err)?;
            self.live(store, surfaces, &work)?;
        }
        let result = tokio::select! {biased;_=work.cancel.cancelled()=>Err("CANCELLED".into()),r=model.stream(&work.network,&work.profile,work.body.clone(),&work.cancel,Box::new(|text|self.push(store,surfaces,&work,text)))=>r};
        self.finish(store, surfaces, &work, result)?;
        guard.done = true;
        Ok(())
    }
    pub(crate) fn revoke(&self, instance: &str) {
        if let Ok(mut active) = self.active.lock() {
            for a in active.values_mut().filter(|a| a.instance == instance) {
                a.cancel.cancel();
                a.ending.get_or_insert(Phase::Cancelled);
            }
        }
    }
    pub(crate) fn closed_memory(&self, surface: &Surface, force: bool) {
        if force || !surface.background {
            if let Ok(mut active) = self.active.lock() {
                for a in active.values_mut().filter(|a| a.window == surface.shell()) {
                    a.cancel.cancel();
                    a.ending.get_or_insert(Phase::Cancelled);
                }
            }
        }
    }
    pub(crate) fn cancel_revision_locked(
        &self,
        store: &Store,
        instance: &str,
        revision: u64,
    ) -> AppResult<()> {
        let contexts = {
            let mut active = self.active.lock().map_err(err)?;
            active
                .values_mut()
                .filter(|a| a.instance == instance && a.model_revision != revision)
                .map(|a| {
                    a.cancel.cancel();
                    a.ending.get_or_insert(Phase::Cancelled);
                    (a.context.clone(), a.kind)
                })
                .collect::<Vec<_>>()
        };
        for (c, k) in contexts {
            self.reconcile_one_locked(store, instance, &c, k)?;
        }
        Ok(())
    }
    pub(crate) fn cancel_instance_locked(&self, store: &Store, instance: &str) -> AppResult<()> {
        self.revoke(instance);
        let contexts = self
            .active
            .lock()
            .map_err(err)?
            .values()
            .filter(|a| a.instance == instance)
            .map(|a| (a.context.clone(), a.kind))
            .collect::<Vec<_>>();
        for (c, k) in contexts {
            self.reconcile_one_locked(store, instance, &c, k)?;
        }
        Ok(())
    }
    pub(crate) fn recover_instance_locked(&self, store: &Store, instance: &str) -> AppResult<()> {
        let list = data::running(&*store.db.lock().map_err(err)?, instance)?;
        for (c, k) in list {
            self.reconcile_one_locked(store, instance, &c, k)?;
        }
        Ok(())
    }
    pub(crate) fn shutdown_locked(&self, store: &Store) -> AppResult<()> {
        self.stopped.store(true, Ordering::Release);
        let contexts = {
            let mut active = self.active.lock().map_err(err)?;
            active
                .values_mut()
                .map(|a| {
                    a.cancel.cancel();
                    a.ending.get_or_insert(Phase::Interrupted);
                    (a.instance.clone(), a.context.clone(), a.kind)
                })
                .collect::<Vec<_>>()
        };
        for (i, c, k) in contexts {
            self.reconcile_one_locked(store, &i, &c, k)?;
        }
        Ok(())
    }
    pub(crate) fn stop_memory(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(mut active) = self.active.lock() {
            for a in active.values_mut() {
                a.cancel.cancel();
                a.ending.get_or_insert(Phase::Interrupted);
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
    use tauri::Manager;
    let core = app.state::<crate::Core>();
    if core.quitting.load(Ordering::Acquire) {
        return Err("SESSION_EXPIRED: 宿主正在退出".into());
    }
    let (value, work) = core.jobs.knowledge.access(
        &core.store,
        &core.sessions,
        &core.jobs.surfaces,
        window,
        token,
        method,
        args,
    )?;
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
            if let Err(error) = core
                .jobs
                .knowledge
                .execute(&core.store, &core.jobs.surfaces, work, &job_host::HttpModel)
                .await
            {
                core.report(error);
            }
        });
    }
    Ok(value)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{
        future::Future,
        pin::Pin,
        sync::{atomic::AtomicUsize, Arc},
    };
    struct Fixture {
        store: Arc<Store>,
        sessions: Arc<widgets::Sessions>,
        parent: Arc<job_host::Service>,
        instance: String,
        window: String,
        token: String,
        context: Knowledge,
        card: String,
        card_token: String,
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
                "desktop-integration-widgetjs-job-knowledge-{}",
                uid()
            ));
        let store = Arc::new(Store::open(root).unwrap());
        store
            .save_preference("network", "{\"mode\":\"direct\"}")
            .unwrap();
        let manifest = json!({"schemaVersion":1,"id":crate::job_protocol::PACKAGE,"name":"知识离线检查","version":"0.1.0","hostApiMajor":1,"entry":"dist/index.html","defaultSize":{"width":600,"height":450},"minSize":{"width":300,"height":200},"capabilities":[crate::job_protocol::CAPABILITY],"settingsSchema":{"type":"object","additionalProperties":false,"properties":{}}});
        store.db.lock().unwrap().execute("INSERT INTO widget_packages(id,manifest,directory,digest) VALUES(?1,?2,'knowledge-fixture',?3)",rusqlite::params![crate::job_protocol::PACKAGE,manifest.to_string(),"a".repeat(64)]).unwrap();
        let instance = widgets::create_instance(&store, crate::job_protocol::PACKAGE).unwrap();
        let card = format!("float-{instance}");
        let sessions = Arc::new(widgets::Sessions::default());
        let card_session =
            widgets::open_display_session(&store, &sessions, &card, &instance, None).unwrap();
        let card_token = card_session["token"].as_str().unwrap().to_owned();
        let parent = Arc::new(job_host::Service::default());
        parent.access(&store,&sessions,&card,&card_token,"jobs.configure",json!({"requestId":uid(),"expectedRevision":0,"config":{"search":{"skills":"","selfIntroduction":"","desiredSalary":"","location":"","searchKeyword":"开发","disadvantages":"","advantages":[]},"backgroundEnabled":background}})).unwrap();
        parent.access(&store,&sessions,&card,&card_token,"jobs.model.configure",json!({"requestId":uid(),"expectedRevision":0,"apiBaseUrl":"https://model.example/v1","apiKey":"knowledge-fixture-key","model":"fixture-model","enabled":true})).unwrap();
        let context = Knowledge {
            knowledge: "Vue".into(),
            job_name: "开发".into(),
            industry: "互联网".into(),
            extra: "性能".into(),
        };
        let surface = parent
            .surfaces
            .lock()
            .unwrap()
            .open(
                &instance,
                &"a".repeat(64),
                Page::Knowledge {
                    context: context.clone(),
                },
                background,
            )
            .unwrap();
        let window = surface.shell();
        let session =
            widgets::open_display_session(&store, &sessions, &window, &instance, None).unwrap();
        let token = session["token"].as_str().unwrap().to_owned();
        parent.bind(&store, &window, &instance, &token).unwrap();
        Fixture {
            store,
            sessions,
            parent,
            instance,
            window,
            token,
            context,
            card,
            card_token,
        }
    }
    impl Fixture {
        fn call(&self, method: &str, args: Value) -> AppResult<(Value, Option<Work>)> {
            self.parent.knowledge.access(
                &self.store,
                &self.sessions,
                &self.parent.surfaces,
                &self.window,
                &self.token,
                method,
                args,
            )
        }
        fn generate(&self, kind: Kind, expected: u64, id: &str) -> Work {
            let mut input =
                json!({"kind":kind,"requestId":id,"expectedRevision":expected,"modelRevision":1});
            if kind == Kind::Interview {
                input["text"] = json!("开始");
            }
            self.call("jobs.knowledge.generate", input)
                .unwrap()
                .1
                .unwrap()
        }
        fn get(&self, kind: Kind) -> Value {
            self.call("jobs.knowledge.get", json!({"kind":kind}))
                .unwrap()
                .0
        }
        async fn run(&self, w: Work, model: &impl job_host::Model) {
            self.parent
                .knowledge
                .execute(&self.store, &self.parent.surfaces, w, model)
                .await
                .unwrap();
        }
    }
    #[derive(Default)]
    struct FakeModel {
        calls: AtomicUsize,
        fail: bool,
        invalid: bool,
        hook: Option<Box<dyn Fn() -> AppResult<()> + Send + Sync>>,
    }
    impl job_host::Model for FakeModel {
        fn stream<'a>(
            &'a self,
            _: &'a NetworkConfig,
            p: &'a Profile,
            body: Value,
            cancel: &'a CancellationToken,
            mut update: Box<dyn FnMut(&str) -> AppResult<()> + Send + 'a>,
        ) -> Pin<Box<dyn Future<Output = AppResult<String>> + Send + 'a>> {
            Box::pin(async move {
                if cancel.is_cancelled() {
                    return Err("CANCELLED".into());
                }
                self.calls.fetch_add(1, Ordering::AcqRel);
                assert_eq!(p.key, "knowledge-fixture-key");
                assert!(!body.to_string().contains(&p.key));
                update("# 部分内容")?;
                if let Some(h) = &self.hook {
                    h()?;
                }
                if self.fail {
                    return Err("NETWORK_ERROR: 离线模拟中断".into());
                }
                if self.invalid {
                    update("无效\0结果")?;
                }
                let text = "# 完整知识\n\n- Vue";
                update(text)?;
                Ok(text.into())
            })
        }
    }
    #[tokio::test]
    async fn reading_three_tabs_never_generates_and_explicit_generation_is_durable_and_idempotent()
    {
        let f = fixture(false);
        for k in [Kind::Wiki, Kind::Mindmap, Kind::Interview] {
            let v = f.get(k);
            assert!(v["job"].is_null());
            assert_eq!(v["generated"], false);
        }
        let id = uid();
        let w = f.generate(Kind::Wiki, 0, &id);
        let model = FakeModel::default();
        f.run(w, &model).await;
        let v = f.get(Kind::Wiki);
        assert_eq!(v["job"]["phase"], "completed");
        assert_eq!(v["revision"], 2);
        assert!(v["latestText"].as_str().unwrap().contains("完整知识"));
        assert!(!v.to_string().contains("knowledge-fixture-key"));
        let (_, work) = f
            .call(
                "jobs.knowledge.generate",
                json!({"kind":"wiki","requestId":id,"expectedRevision":0,"modelRevision":1}),
            )
            .unwrap();
        assert!(work.is_none());
        assert_eq!(model.calls.load(Ordering::Acquire), 1);
    }
    #[tokio::test]
    async fn wiki_mindmap_and_interview_use_independent_slots_and_complete_histories() {
        let f = fixture(false);
        let wiki = f.generate(Kind::Wiki, 0, &uid());
        let map = f.generate(Kind::Mindmap, 0, &uid());
        let interview = f.generate(Kind::Interview, 0, &uid());
        let model = FakeModel::default();
        f.run(wiki, &model).await;
        f.run(map, &model).await;
        f.run(interview, &model).await;
        for k in [Kind::Wiki, Kind::Mindmap, Kind::Interview] {
            assert_eq!(f.get(k)["job"]["phase"], "completed");
        }
        let v = f.get(Kind::Interview);
        assert_eq!(v["messages"].as_array().unwrap().len(), 3);
        assert_eq!(model.calls.load(Ordering::Acquire), 3);
    }
    #[tokio::test]
    async fn closing_without_background_preserves_partial_text_and_rejects_the_late_completion() {
        let f = fixture(false);
        let w = f.generate(Kind::Wiki, 0, &uid());
        let parent = f.parent.clone();
        let window = f.window.clone();
        let model = FakeModel {
            hook: Some(Box::new(move || {
                let surface = parent
                    .surfaces
                    .lock()
                    .map_err(err)?
                    .parent(&window.replacen("-shell-", "-panel-", 1))
                    .ok_or("缺少窗口")?;
                let result = parent
                    .surfaces
                    .lock()
                    .map_err(err)?
                    .close(&surface.parent(), false)?
                    .unwrap();
                parent.closed_memory(&result.0, result.1, false)
            })),
            ..Default::default()
        };
        f.run(w, &model).await;
        let d = data::load(
            &f.store.db.lock().unwrap(),
            &f.instance,
            &f.context,
            Kind::Wiki,
        )
        .unwrap();
        assert_eq!(d.job.as_ref().unwrap().phase, Phase::Cancelled);
        assert_eq!(d.job.as_ref().unwrap().partial, "# 部分内容");
        assert_eq!(d.snapshot()["generated"], false);
        assert_eq!(model.calls.load(Ordering::Acquire), 1);
    }
    #[tokio::test]
    async fn enabled_background_finishes_after_its_knowledge_window_is_closed() {
        let f = fixture(true);
        let w = f.generate(Kind::Wiki, 0, &uid());
        let parent = f.parent.clone();
        let window = f.window.clone();
        let instance = f.instance.clone();
        let model = FakeModel {
            hook: Some(Box::new(move || {
                let surface = parent
                    .surfaces
                    .lock()
                    .map_err(err)?
                    .shell(&window, &instance)
                    .unwrap();
                let (s, retired) = parent
                    .surfaces
                    .lock()
                    .map_err(err)?
                    .close(&surface.parent(), false)?
                    .unwrap();
                parent.closed_memory(&s, retired, false)
            })),
            ..Default::default()
        };
        f.run(w, &model).await;
        assert_eq!(
            data::load(
                &f.store.db.lock().unwrap(),
                &f.instance,
                &f.context,
                Kind::Wiki
            )
            .unwrap()
            .job
            .unwrap()
            .phase,
            Phase::Completed
        );
    }
    #[tokio::test]
    async fn clearing_one_tab_does_not_cancel_the_other_or_replay_an_old_clear_on_new_work() {
        let f = fixture(false);
        let old = f.generate(Kind::Wiki, 0, &uid());
        let map = f.generate(Kind::Mindmap, 0, &uid());
        let clear = uid();
        f.call(
            "jobs.knowledge.clear",
            json!({"kind":"wiki","requestId":clear,"expectedRevision":1}),
        )
        .unwrap();
        assert!(old.cancel.is_cancelled());
        assert!(!map.cancel.is_cancelled());
        let next = f.generate(Kind::Wiki, 2, &uid());
        f.call(
            "jobs.knowledge.clear",
            json!({"kind":"wiki","requestId":clear,"expectedRevision":1}),
        )
        .unwrap();
        assert!(!next.cancel.is_cancelled());
        let model = FakeModel::default();
        f.run(next, &model).await;
        f.run(map, &model).await;
        assert_eq!(model.calls.load(Ordering::Acquire), 2);
        assert_eq!(f.get(Kind::Wiki)["job"]["phase"], "completed");
    }
    #[tokio::test]
    async fn model_configuration_changes_cancel_the_old_request_and_keep_it_out_of_history() {
        let f = fixture(false);
        let w = f.generate(Kind::Interview, 0, &uid());
        let parent = f.parent.clone();
        let store = f.store.clone();
        let sessions = f.sessions.clone();
        let card = f.card.clone();
        let token = f.card_token.clone();
        let model = FakeModel {
            hook: Some(Box::new(move || {
                parent
                    .access(
                        &store,
                        &sessions,
                        &card,
                        &token,
                        "jobs.model.clear",
                        json!({"requestId":uid(),"expectedRevision":1}),
                    )
                    .map(|_| ())
            })),
            ..Default::default()
        };
        f.run(w, &model).await;
        let v = f.get(Kind::Interview);
        assert_eq!(v["job"]["phase"], "cancelled");
        assert_eq!(v["messages"].as_array().unwrap().len(), 2);
        assert_eq!(v["generated"], false);
    }
    #[tokio::test]
    async fn a_failed_stream_is_recorded_as_incomplete_and_is_not_retried() {
        let f = fixture(false);
        let model = FakeModel {
            fail: true,
            ..Default::default()
        };
        let w = f.generate(Kind::Mindmap, 0, &uid());
        f.run(w, &model).await;
        let v = f.get(Kind::Mindmap);
        assert_eq!(v["job"]["phase"], "failed");
        assert_eq!(v["job"]["partial"], "# 部分内容");
        assert_eq!(v["generated"], false);
        assert_eq!(model.calls.load(Ordering::Acquire), 1);
    }
    #[test]
    fn a_dropped_or_restarted_request_never_produces_a_new_work_item_on_read() {
        let f = fixture(false);
        let w = f.generate(Kind::Wiki, 0, &uid());
        let id = w.id.clone();
        drop(w);
        let v = f.get(Kind::Wiki);
        assert_eq!(v["job"]["phase"], "cancelled");
        assert_eq!(v["job"]["id"], id);
        let id = uid();
        data::reserve(
            &mut f.store.db.lock().unwrap(),
            &f.instance,
            &f.context,
            Kind::Mindmap,
            &id,
            0,
            1,
            &"a".repeat(64),
            "model",
            None,
            1,
        )
        .unwrap();
        assert_eq!(f.get(Kind::Mindmap)["job"]["phase"], "interrupted");
    }
    #[test]
    fn unknown_prompts_contexts_and_card_sessions_cannot_generate() {
        let f = fixture(false);
        assert!(f
            .parent
            .knowledge
            .access(
                &f.store,
                &f.sessions,
                &f.parent.surfaces,
                &f.card,
                &f.card_token,
                "jobs.knowledge.generate",
                json!({"kind":"wiki","requestId":uid(),"expectedRevision":0,"modelRevision":1})
            )
            .is_err());
        for field in ["prompt", "context", "url", "script"] {
            let mut v =
                json!({"kind":"wiki","requestId":uid(),"expectedRevision":0,"modelRevision":1});
            v[field] = json!("不接受");
            assert!(f.call("jobs.knowledge.generate", v).is_err());
        }
        assert!(f.get(Kind::Wiki)["job"].is_null());
    }
    #[test]
    fn shutdown_marks_running_records_interrupted_and_never_returns_a_new_body() {
        let f = fixture(false);
        let w = f.generate(Kind::Wiki, 0, &uid());
        f.parent.shutdown(&f.store).unwrap();
        assert!(w.cancel.is_cancelled());
        assert_eq!(
            data::load(
                &f.store.db.lock().unwrap(),
                &f.instance,
                &f.context,
                Kind::Wiki
            )
            .unwrap()
            .job
            .unwrap()
            .phase,
            Phase::Interrupted
        );
        assert!(f
            .call("jobs.knowledge.get", json!({"kind":"wiki"}))
            .is_err());
    }
    #[tokio::test]
    async fn invalid_stream_content_never_poisons_the_saved_partial_or_reports_completion() {
        let f = fixture(false);
        let model = FakeModel {
            invalid: true,
            ..Default::default()
        };
        f.run(f.generate(Kind::Wiki, 0, &uid()), &model).await;
        let v = f.get(Kind::Wiki);
        assert_eq!(v["job"]["phase"], "failed");
        assert_eq!(v["job"]["partial"], "# 部分内容");
        assert_eq!(v["generated"], false);
        assert_eq!(model.calls.load(Ordering::Acquire), 1);
    }
    #[tokio::test]
    async fn deleting_an_instance_clears_its_knowledge_but_keeps_the_other_instance() {
        let f = fixture(false);
        f.run(f.generate(Kind::Wiki, 0, &uid()), &FakeModel::default())
            .await;
        let other = widgets::create_instance(&f.store, crate::job_protocol::PACKAGE).unwrap();
        data::reserve(
            &mut f.store.db.lock().unwrap(),
            &other,
            &f.context,
            Kind::Wiki,
            &uid(),
            0,
            1,
            &"a".repeat(64),
            "model",
            None,
            1,
        )
        .unwrap();
        widgets::remove_instance_before(&f.store, &f.sessions, &f.instance, || {
            f.parent.revoke(&f.instance)
        })
        .unwrap();
        let db = f.store.db.lock().unwrap();
        let count: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM app_settings WHERE key LIKE ?1",
                [format!("job.chat/{}/%", f.instance)],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        assert!(data::load(&db, &other, &f.context, Kind::Wiki)
            .unwrap()
            .job
            .is_some());
    }
    struct BlockingModel {
        started: Arc<tokio::sync::Notify>,
    }
    impl job_host::Model for BlockingModel {
        fn stream<'a>(
            &'a self,
            _: &'a NetworkConfig,
            _: &'a Profile,
            _: Value,
            _: &'a CancellationToken,
            mut update: Box<dyn FnMut(&str) -> AppResult<()> + Send + 'a>,
        ) -> Pin<Box<dyn Future<Output = AppResult<String>> + Send + 'a>> {
            Box::pin(async move {
                update("# 尚未完成")?;
                self.started.notify_one();
                std::future::pending().await
            })
        }
    }
    #[tokio::test]
    async fn aborting_a_knowledge_worker_preserves_an_interrupted_partial_without_retry() {
        let f = fixture(false);
        let work = f.generate(Kind::Mindmap, 0, &uid());
        let parent = f.parent.clone();
        let store = f.store.clone();
        let started = Arc::new(tokio::sync::Notify::new());
        let signal = started.clone();
        let task = tokio::spawn(async move {
            parent
                .knowledge
                .execute(
                    &store,
                    &parent.surfaces,
                    work,
                    &BlockingModel { started: signal },
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), started.notified())
            .await
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let value = f.get(Kind::Mindmap);
        assert_eq!(value["job"]["phase"], "interrupted");
        assert_eq!(value["job"]["partial"], "# 尚未完成");
        assert_eq!(value["generated"], false);
    }
}
