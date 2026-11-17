//! AI直聘专用窗口账本；远程BOSS和详情页不拥有本机壳会话。
use crate::{
    job_protocol,
    job_webviews::Owner,
    store::{err, AppResult},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Knowledge {
    pub knowledge: String,
    #[serde(default)]
    pub job_name: String,
    #[serde(default)]
    pub industry: String,
    #[serde(default)]
    pub extra: String,
}
impl Knowledge {
    pub(crate) fn validate(&self) -> AppResult<()> {
        if self.knowledge.trim().is_empty()
            || [
                (self.knowledge.as_str(), 200),
                (self.job_name.as_str(), 300),
                (self.industry.as_str(), 300),
                (self.extra.as_str(), 2000),
            ]
            .iter()
            .any(|(v, max)| {
                v.chars().count() > *max
                    || v.chars()
                        .any(|c| c.is_control() && !['\n', '\r', '\t'].contains(&c))
            })
        {
            return Err("INVALID_ARGUMENT: 知识点、职位、行业或说明无效".into());
        }
        Ok(())
    }
    pub(crate) fn id(&self) -> AppResult<String> {
        self.validate()?;
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).map_err(err)?)
        ))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "page",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum Page {
    Search,
    Analysis,
    Knowledge { context: Knowledge },
    Detail { job_id: String },
}
impl<'de> Deserialize<'de> for Page {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let fields = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("页面参数须为对象"))?;
        match fields.get("page").and_then(Value::as_str) {
            Some("search") if fields.len() == 1 => Ok(Self::Search),
            Some("analysis") if fields.len() == 1 => Ok(Self::Analysis),
            Some("knowledge") if fields.len() == 2 && fields.contains_key("context") => {
                Ok(Self::Knowledge {
                    context: serde_json::from_value(value["context"].clone())
                        .map_err(serde::de::Error::custom)?,
                })
            }
            Some("detail")
                if fields.len() == 2 && fields.get("jobId").is_some_and(Value::is_string) =>
            {
                Ok(Self::Detail {
                    job_id: fields["jobId"].as_str().expect("已核对为字符串").into(),
                })
            }
            _ => Err(serde::de::Error::custom(
                "页面或字段无效，不接受网址、脚本和附加字段",
            )),
        }
    }
}
impl Page {
    pub(crate) fn key(&self) -> AppResult<String> {
        Ok(match self {
            Self::Search => "search".into(),
            Self::Analysis => "analysis".into(),
            Self::Knowledge { context } => format!("knowledge/{}", context.id()?),
            Self::Detail { job_id } => {
                job_protocol::job_id(job_id)?;
                format!("detail/{job_id}")
            }
        })
    }
    fn prefix(&self) -> &'static str {
        match self {
            Self::Search => "job",
            Self::Analysis => "job-analysis",
            Self::Knowledge { .. } => "job-knowledge",
            Self::Detail { .. } => "job-detail",
        }
    }
    pub(crate) fn remote(&self) -> bool {
        matches!(self, Self::Detail { .. })
    }
    pub(crate) fn title(&self) -> String {
        match self {
            Self::Search => "AI直聘".into(),
            Self::Analysis => "AI直聘 · 职位分析".into(),
            Self::Knowledge { context } => format!("AI直聘 · {}", context.knowledge),
            Self::Detail { .. } => "AI直聘 · 职位详情".into(),
        }
    }
}
#[derive(Clone)]
pub(crate) struct Surface {
    pub owner: Owner,
    pub page: Page,
    pub background: bool,
    pub visible: bool,
    pub ready: bool,
    pub overlay: bool,
    pub rect: Option<(f64, f64, f64, f64)>,
    pub serial: u64,
    pub applied: u64,
    pub focus: bool,
    pub fault: Option<String>,
    token: Option<String>,
    pub boss_initialized: bool,
}
impl Surface {
    pub(crate) fn shell(&self) -> String {
        if self.page.remote() {
            self.parent()
        } else {
            format!(
                "{}-shell-{}-{}",
                self.page.prefix(),
                self.owner.instance,
                self.owner.presentation
            )
        }
    }
    pub(crate) fn parent(&self) -> String {
        format!(
            "{}-panel-{}-{}",
            self.page.prefix(),
            self.owner.instance,
            self.owner.presentation
        )
    }
    pub(crate) fn search(&self) -> bool {
        self.page == Page::Search
    }
    pub(crate) fn snapshot(&self) -> Value {
        json!({"presentationId":self.owner.presentation,"page":self.page,"visible":self.visible,"ready":self.ready,"overlay":self.overlay,"serial":self.serial,"nativeSettled":self.serial==self.applied,"fault":self.fault})
    }
}
#[derive(Default)]
pub(crate) struct Registry {
    entries: HashMap<String, Surface>,
    pub retired: Vec<Surface>,
    sequence: u64,
    pub errors: HashMap<String, String>,
}
impl Registry {
    fn next(&mut self) -> AppResult<u64> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("LIMIT: AI直聘窗口版本达到上限")?;
        Ok(self.sequence)
    }
    pub(crate) fn open(
        &mut self,
        instance: &str,
        digest: &str,
        page: Page,
        background: bool,
    ) -> AppResult<Surface> {
        let key = page.key()?;
        let probe = Owner {
            instance: instance.into(),
            presentation: uuid::Uuid::new_v4().to_string(),
            digest: digest.into(),
        };
        probe.validate()?;
        if let Some(label) = self
            .entries
            .iter()
            .find(|(_, s)| s.owner.instance == instance && s.page.key().is_ok_and(|k| k == key))
            .map(|(k, _)| k.clone())
        {
            if self.entries[&label].owner.digest == digest && self.entries[&label].fault.is_none() {
                let serial = self.next()?;
                let s = self.entries.get_mut(&label).expect("现有窗口已定位");
                s.visible = true;
                s.focus = true;
                s.serial = serial;
                s.background = background;
                return Ok(s.clone());
            }
            if let Some(old) = self.entries.remove(&label) {
                self.retired.push(old);
            }
        }
        if self
            .entries
            .values()
            .filter(|s| s.owner.instance == instance)
            .count()
            >= 32
        {
            return Err("CAPACITY_EXCEEDED: 当前实例窗口过多，请先关闭部分窗口".into());
        }
        let serial = self.next()?;
        let surface = Surface {
            owner: probe,
            page,
            background,
            visible: true,
            ready: false,
            overlay: false,
            rect: None,
            serial,
            applied: 0,
            focus: true,
            fault: None,
            token: None,
            boss_initialized: false,
        };
        self.entries.insert(surface.shell(), surface.clone());
        Ok(surface)
    }
    pub(crate) fn list(&self, instance: &str) -> Vec<Surface> {
        self.entries
            .values()
            .filter(|s| s.owner.instance == instance)
            .cloned()
            .collect()
    }
    pub(crate) fn instances(&self) -> Vec<String> {
        let mut ids = self
            .entries
            .values()
            .chain(self.retired.iter())
            .map(|s| s.owner.instance.clone())
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        ids
    }
    pub(crate) fn current(&self, s: &Surface) -> bool {
        self.entries
            .get(&s.shell())
            .is_some_and(|v| v.owner == s.owner && v.page == s.page)
    }
    pub(crate) fn shell(&self, label: &str, instance: &str) -> Option<Surface> {
        self.entries
            .get(label)
            .filter(|s| s.owner.instance == instance && !s.page.remote())
            .cloned()
    }
    pub(crate) fn parent(&self, label: &str) -> Option<Surface> {
        self.entries.values().find(|s| s.parent() == label).cloned()
    }
    pub(crate) fn bind(&mut self, label: &str, instance: &str, token: &str) -> AppResult<()> {
        let s = self
            .entries
            .get_mut(label)
            .filter(|s| s.owner.instance == instance && !s.page.remote())
            .ok_or("PRESENTATION_EXPIRED: AI直聘本机壳已关闭或替换")?;
        s.token = Some(token.into());
        s.ready = false;
        s.overlay = false;
        Ok(())
    }
    pub(crate) fn authorized(&self, label: &str, token: &str) -> AppResult<Surface> {
        self.entries
            .get(label)
            .filter(|s| s.token.as_deref() == Some(token) && !s.page.remote())
            .cloned()
            .ok_or_else(|| "SESSION_EXPIRED: AI直聘本机壳会话已撤销".into())
    }
    pub(crate) fn close_session(&mut self, label: &str, token: &str) -> AppResult<()> {
        let s = self.authorized(label, token)?;
        let serial = self.next()?;
        let s = self.entries.get_mut(&s.shell()).expect("会话归属已核对");
        s.token = None;
        s.ready = false;
        s.serial = serial;
        Ok(())
    }
    pub(crate) fn ready(&mut self, label: &str, token: &str) -> AppResult<()> {
        self.authorized(label, token)?;
        let serial = self.next()?;
        let s = self.entries.get_mut(label).expect("会话归属已核对");
        s.ready = true;
        s.fault = None;
        s.serial = serial;
        Ok(())
    }
    pub(crate) fn layout(
        &mut self,
        label: &str,
        token: &str,
        rect: (f64, f64, f64, f64),
    ) -> AppResult<()> {
        let s = self.authorized(label, token)?;
        if !s.search() || !crate::job_webviews::valid_rect(rect, (16384., 16384.)) {
            return Err("INVALID_ARGUMENT: 此窗口不允许BOSS布局或矩形无效".into());
        }
        let serial = self.next()?;
        let s = self.entries.get_mut(label).expect("会话归属已核对");
        s.rect = Some(rect);
        s.serial = serial;
        Ok(())
    }
    pub(crate) fn overlay(&mut self, label: &str, token: &str, active: bool) -> AppResult<()> {
        let s = self.authorized(label, token)?;
        if !s.search() {
            return Err("INVALID_ARGUMENT: 此窗口不承载BOSS网页".into());
        }
        let serial = self.next()?;
        let s = self.entries.get_mut(label).expect("会话归属已核对");
        s.overlay = active;
        s.serial = serial;
        Ok(())
    }
    pub(crate) fn fail(&mut self, label: &str) -> AppResult<Surface> {
        let serial = self.next()?;
        let s = self
            .entries
            .get_mut(label)
            .ok_or("PRESENTATION_EXPIRED: AI直聘窗口已关闭")?;
        s.ready = false;
        s.fault = Some("本机界面加载失败，请关闭后重新打开".into());
        s.serial = serial;
        Ok(s.clone())
    }
    pub(crate) fn configure(&mut self, instance: &str, background: bool) -> AppResult<()> {
        let serial = self.next()?;
        for s in self
            .entries
            .values_mut()
            .filter(|s| s.owner.instance == instance)
        {
            s.background = background;
            s.serial = serial;
        }
        Ok(())
    }
    pub(crate) fn hide(&mut self, instance: &str) -> AppResult<Vec<Surface>> {
        let serial = self.next()?;
        let mut result = Vec::new();
        for s in self
            .entries
            .values_mut()
            .filter(|s| s.owner.instance == instance)
        {
            s.visible = false;
            s.focus = false;
            s.serial = serial;
            result.push(s.clone());
        }
        Ok(result)
    }
    pub(crate) fn close(
        &mut self,
        parent: &str,
        quitting: bool,
    ) -> AppResult<Option<(Surface, bool)>> {
        let Some(s) = self.parent(parent) else {
            return Ok(None);
        };
        if s.search() && s.background && !quitting {
            let serial = self.next()?;
            let s = self.entries.get_mut(&s.shell()).expect("当前窗口已定位");
            s.visible = false;
            s.focus = false;
            s.serial = serial;
            return Ok(Some((s.clone(), false)));
        }
        self.entries.remove(&s.shell());
        self.retired.push(s.clone());
        Ok(Some((s, true)))
    }
    pub(crate) fn revoke(&mut self, instance: &str) {
        let labels = self
            .entries
            .iter()
            .filter(|(_, s)| s.owner.instance == instance)
            .map(|(l, _)| l.clone())
            .collect::<Vec<_>>();
        for label in labels {
            if let Some(s) = self.entries.remove(&label) {
                self.retired.push(s);
            }
        }
    }
    pub(crate) fn confirmed(&mut self, s: &Surface) {
        if let Some(current) = self
            .entries
            .get_mut(&s.shell())
            .filter(|v| v.owner == s.owner && v.serial == s.serial)
        {
            current.applied = s.serial;
            current.focus = false;
            self.errors.remove(&s.owner.instance);
        }
    }
    pub(crate) fn focused(&mut self, s: &Surface) {
        if let Some(v) = self
            .entries
            .get_mut(&s.shell())
            .filter(|v| v.owner == s.owner)
        {
            v.focus = false;
        }
    }
    pub(crate) fn retired_confirmed(&mut self, s: &Surface) {
        self.retired
            .retain(|r| r.owner != s.owner || r.page != s.page);
    }
    pub(crate) fn snapshot(&self, instance: &str) -> Value {
        json!({"surfaces":self.list(instance).iter().map(Surface::snapshot).collect::<Vec<_>>(),"nativeCleanupPending":self.retired.iter().any(|s|s.owner.instance==instance),"nativeError":self.errors.get(instance)})
    }
}
pub(crate) fn shell_role(label: &str, instance: &str) -> bool {
    ["job-shell-", "job-analysis-shell-", "job-knowledge-shell-"]
        .iter()
        .any(|p| {
            label
                .strip_prefix(&format!("{p}{instance}-"))
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id))
        })
}
pub(crate) fn role(label: &str) -> Option<&'static str> {
    if label.starts_with("job-shell-") {
        Some("job-search")
    } else if label.starts_with("job-analysis-shell-") {
        Some("job-analysis")
    } else if label.starts_with("job-knowledge-shell-") {
        Some("job-knowledge")
    } else {
        None
    }
}
pub(crate) fn parent_role(label: &str) -> bool {
    [
        "job-panel-",
        "job-analysis-panel-",
        "job-knowledge-panel-",
        "job-detail-panel-",
    ]
    .iter()
    .any(|p| {
        label.strip_prefix(p).is_some_and(|tail| {
            tail.len() == 73
                && tail.is_ascii()
                && tail.as_bytes()[36] == b'-'
                && [&tail[..36], &tail[37..]]
                    .iter()
                    .all(|id| uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == *id))
        })
    })
}

#[cfg(windows)]
pub(crate) mod native {
    use super::*;
    use crate::{job_host, job_webviews, Core};
    use std::sync::atomic::Ordering;
    use tauri::{
        webview::{NewWindowResponse, WebviewBuilder},
        AppHandle, Manager, WebviewUrl, WebviewWindowBuilder,
    };
    fn local(app: &AppHandle, url: &url::Url) -> bool {
        let origin = url.origin().ascii_serialization();
        matches!(
            origin.as_str(),
            "http://tauri.localhost" | "https://tauri.localhost"
        ) || url.scheme() == "tauri" && url.host_str() == Some("localhost")
            || app
                .config()
                .build
                .dev_url
                .as_ref()
                .is_some_and(|dev| dev.origin() == url.origin())
    }
    fn current(app: &AppHandle, s: &Surface) -> bool {
        let core = app.state::<Core>();
        !core.quitting.load(Ordering::Acquire)
            && !core.jobs.stopped()
            && core.jobs.surfaces.lock().is_ok_and(|r| r.current(s))
            && job_host::source(&core.store, &s.owner.instance)
                .is_ok_and(|src| src.digest == s.owner.digest)
    }
    fn create(app: &AppHandle, s: &Surface) -> AppResult<()> {
        let core = app.state::<Core>();
        if app.get_window(&s.parent()).is_some() {
            return Ok(());
        }
        if let Page::Detail { job_id } = &s.page {
            let url = job_protocol::detail_url(job_id)?;
            let profile = s.owner.profile(&core.store.root)?;
            std::fs::create_dir_all(&profile).map_err(err)?;
            WebviewWindowBuilder::new(app, s.parent(), WebviewUrl::External(url))
                .data_directory(profile)
                .title(s.page.title())
                .visible(false)
                .focused(false)
                .inner_size(1000., 760.)
                .min_inner_size(640., 480.)
                .devtools(false)
                .on_navigation(job_protocol::site_navigation)
                .on_new_window(|_, _| NewWindowResponse::Deny)
                .build()
                .map_err(|_| "NATIVE_ERROR: 职位详情窗口创建失败")?;
            return Ok(());
        }
        let directory = core.store.root.join("webview").join(s.shell());
        std::fs::create_dir_all(&directory).map_err(err)?;
        let window = tauri::window::WindowBuilder::new(app, s.parent())
            .title(s.page.title())
            .visible(false)
            .focused(false)
            .inner_size(1150., 850.)
            .min_inner_size(640., 480.)
            .build()
            .map_err(|_| "NATIVE_ERROR: AI直聘窗口创建失败")?;
        let url = format!(
            "index.html?jobPanel={}&jobPresentation={}",
            s.owner.instance, s.owner.presentation
        );
        let handle = app.clone();
        let shell = WebviewBuilder::new(s.shell(), WebviewUrl::App(url.into()))
            .data_directory(directory)
            .initialization_script(crate::windows::host_script(app))
            .auto_resize()
            .focused(false)
            .on_navigation(move |url| local(&handle, url))
            .on_new_window(|_, _| NewWindowResponse::Deny);
        if window
            .add_child(
                shell,
                tauri::LogicalPosition::new(0., 0.),
                tauri::LogicalSize::new(1150., 850.),
            )
            .is_err()
        {
            let _ = window.destroy();
            return Err("NATIVE_ERROR: AI直聘本机壳创建失败".into());
        }
        Ok(())
    }
    fn cleanup(app: &AppHandle, s: &Surface) -> AppResult<()> {
        let core = app.state::<Core>();
        core.sessions
            .0
            .lock()
            .map_err(err)?
            .retain(|_, v| v.window != s.shell());
        if s.search() {
            job_webviews::native::Native {
                app: app.clone(),
                book: core.jobs.views.clone(),
            }
            .close(&s.owner)?;
        }
        if let Some(window) = app.get_window(&s.parent()) {
            window
                .destroy()
                .map_err(|_| "NATIVE_ERROR: AI直聘窗口释放失败")?;
        }
        if app.get_window(&s.parent()).is_some() || app.get_webview(&s.shell()).is_some() {
            return Err("NATIVE_ERROR: AI直聘窗口释放尚未确认".into());
        }
        core.jobs.surfaces.lock().map_err(err)?.retired_confirmed(s);
        Ok(())
    }
    pub(crate) fn render(app: &AppHandle, instance: &str) -> AppResult<()> {
        let core = app.state::<Core>();
        let _render = core.jobs.render_lock.lock().map_err(err)?;
        let src = job_host::source(&core.store, instance);
        let plans = core.jobs.surfaces.lock().map_err(err)?.list(instance);
        if core.quitting.load(Ordering::Acquire)
            || core.jobs.stopped()
            || src.as_ref().is_err()
            || plans
                .iter()
                .any(|s| src.as_ref().is_ok_and(|src| src.digest != s.owner.digest))
        {
            core.jobs.revoke(instance)?;
        } else if src.as_ref().is_ok_and(|src| src.hidden || !src.visible) {
            core.jobs.surfaces.lock().map_err(err)?.hide(instance)?;
        }
        let retired = core
            .jobs
            .surfaces
            .lock()
            .map_err(err)?
            .retired
            .iter()
            .filter(|s| s.owner.instance == instance)
            .cloned()
            .collect::<Vec<_>>();
        for s in retired {
            cleanup(app, &s)?;
        }
        let plans = core.jobs.surfaces.lock().map_err(err)?.list(instance);
        for s in plans {
            if s.serial == s.applied
                && !s.focus
                && app.get_window(&s.parent()).is_some()
                && (!s.search() || app.get_webview(&s.owner.boss()).is_some())
            {
                continue;
            }
            if s.fault.is_some() {
                if let Some(view) = app.get_webview(&s.owner.boss()) {
                    let _ = view.hide();
                }
                continue;
            }
            if let Err(error) = create(app, &s) {
                core.jobs
                    .surfaces
                    .lock()
                    .map_err(err)?
                    .fail(&s.shell())
                    .ok();
                return Err(error);
            }
            if !current(app, &s) {
                if let Some(w) = app.get_window(&s.parent()) {
                    let _ = w.hide();
                }
                continue;
            }
            if s.search() {
                let native = job_webviews::native::Native {
                    app: app.clone(),
                    book: core.jobs.views.clone(),
                };
                if app.get_webview(&s.owner.boss()).is_none() {
                    if let Err(error) = native.create(&s.owner, &core.store.root) {
                        core.jobs
                            .surfaces
                            .lock()
                            .map_err(err)?
                            .fail(&s.shell())
                            .ok();
                        return Err(error);
                    }
                }
                if !s.boss_initialized {
                    native.navigate(
                        &s.owner,
                        "https://www.zhipin.com/web/user/?ka=header-login"
                            .parse()
                            .map_err(err)?,
                    )?;
                    if let Some(v) = core
                        .jobs
                        .surfaces
                        .lock()
                        .map_err(err)?
                        .entries
                        .get_mut(&s.shell())
                        .filter(|v| v.owner == s.owner)
                    {
                        v.boss_initialized = true;
                    }
                }
                let shown = s.visible && s.ready && !s.overlay;
                if let Some(rect) = s.rect {
                    if let Err(e) = native.place(&s.owner, rect, shown) {
                        if let Some(v) = app.get_webview(&s.owner.boss()) {
                            let _ = v.hide();
                        }
                        return Err(e);
                    }
                } else if let Some(v) = app.get_webview(&s.owner.boss()) {
                    v.hide().map_err(err)?;
                }
            }
            let window = app
                .get_window(&s.parent())
                .ok_or("PRESENTATION_EXPIRED: AI直聘原生窗口不存在")?;
            if s.visible && current(app, &s) {
                if !window.is_visible().map_err(err)? {
                    window.show().map_err(err)?;
                }
                if s.focus {
                    if window.is_minimized().map_err(err)? {
                        window.unminimize().map_err(err)?;
                    }
                    window.set_focus().map_err(err)?;
                    core.jobs.surfaces.lock().map_err(err)?.focused(&s);
                }
            } else {
                if window.is_visible().map_err(err)? {
                    window.hide().map_err(err)?;
                }
            }
            if !current(app, &s) {
                let _ = window.hide();
            } else {
                core.jobs.surfaces.lock().map_err(err)?.confirmed(&s);
            }
        }
        Ok(())
    }
    pub(crate) fn window_event(app: &AppHandle, label: &str, event: &tauri::WindowEvent) -> bool {
        if !parent_role(label) {
            return false;
        }
        let core = app.state::<Core>();
        let surface = core.jobs.surfaces.lock().ok().and_then(|r| r.parent(label));
        if let Some(s) = surface {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let Ok(mut sessions) = core.sessions.0.lock() else {
                        return true;
                    };
                    let result =
                        core.jobs.surfaces.lock().map_err(err).and_then(|mut r| {
                            r.close(label, core.quitting.load(Ordering::Acquire))
                        });
                    if let Ok(Some((s, retired))) = result {
                        if retired {
                            sessions.retain(|_, v| v.window != s.shell());
                        }
                        let _ = core.jobs.closed_memory(&s, retired, false);
                        core.jobs.kick(app, &s.owner.instance);
                    }
                }
                tauri::WindowEvent::Destroyed => {
                    let Ok(mut sessions) = core.sessions.0.lock() else {
                        return true;
                    };
                    sessions.retain(|_, v| v.window != s.shell());
                    let _ = core.jobs.surfaces.lock().map(|mut r| r.close(label, true));
                    let _ = core.jobs.closed_memory(&s, true, true);
                    core.jobs.kick(app, &s.owner.instance);
                }
                tauri::WindowEvent::Resized(_) => core.jobs.kick(app, &s.owner.instance),
                _ => (),
            }
        } else if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn uid() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn knowledge(extra: &str) -> Page {
        Page::Knowledge {
            context: Knowledge {
                knowledge: "Vue".into(),
                job_name: "开发".into(),
                industry: "互联网".into(),
                extra: extra.into(),
            },
        }
    }
    #[test]
    fn knowledge_identity_binds_all_context_fields() {
        let a = knowledge("性能");
        let b = knowledge("测试");
        assert_ne!(a.key().unwrap(), b.key().unwrap());
        let mut c = match a.clone() {
            Page::Knowledge { context } => context,
            _ => unreachable!(),
        };
        c.job_name = "架构".into();
        assert_ne!(
            Page::Knowledge { context: c }.key().unwrap(),
            a.key().unwrap()
        );
        assert!(serde_json::from_value::<Page>(
            json!({"page":"search","url":"https://evil.example"})
        )
        .is_err());
        assert!(serde_json::from_value::<Page>(
            json!({"page":"knowledge","context":{"knowledge":"Vue","script":"不接受"}})
        )
        .is_err());
    }
    #[test]
    fn fixed_surface_labels_do_not_match_remote_webviews() {
        let mut r = Registry::default();
        let i = uid();
        for p in [Page::Search, Page::Analysis, knowledge("")] {
            let s = r.open(&i, &"a".repeat(64), p, false).unwrap();
            assert!(shell_role(&s.shell(), &i));
            assert!(!shell_role(&s.owner.boss(), &i));
            assert!(!shell_role(&s.parent(), &i));
        }
        assert!(!shell_role(&format!("job-shell-{i}-../x"), &i));
    }
    #[test]
    fn reopening_a_context_reuses_it_and_other_contexts_keep_distinct_sessions() {
        let mut r = Registry::default();
        let i = uid();
        let a = r
            .open(&i, &"a".repeat(64), knowledge("性能"), false)
            .unwrap();
        let b = r
            .open(&i, &"a".repeat(64), knowledge("测试"), false)
            .unwrap();
        assert_ne!(a.shell(), b.shell());
        r.bind(&a.shell(), &i, "a").unwrap();
        r.bind(&b.shell(), &i, "b").unwrap();
        assert!(r.authorized(&a.shell(), "a").is_ok());
        assert!(r.authorized(&b.shell(), "b").is_ok());
        let again = r
            .open(&i, &"a".repeat(64), knowledge("性能"), false)
            .unwrap();
        assert_eq!(a.shell(), again.shell());
    }
    #[test]
    fn old_session_and_old_native_ack_cannot_overwrite_a_newer_surface() {
        let mut r = Registry::default();
        let i = uid();
        let old = r.open(&i, &"a".repeat(64), Page::Search, false).unwrap();
        r.bind(&old.shell(), &i, "old").unwrap();
        r.bind(&old.shell(), &i, "new").unwrap();
        assert!(r.ready(&old.shell(), "old").is_err());
        r.ready(&old.shell(), "new").unwrap();
        r.confirmed(&old);
        assert_ne!(r.list(&i)[0].serial, r.list(&i)[0].applied);
        let newer = r.list(&i)[0].clone();
        r.confirmed(&newer);
        assert_eq!(r.list(&i)[0].serial, r.list(&i)[0].applied);
    }
    #[test]
    fn closing_search_hides_only_when_background_was_enabled() {
        let mut r = Registry::default();
        let i = uid();
        let s = r.open(&i, &"a".repeat(64), Page::Search, false).unwrap();
        r.bind(&s.shell(), &i, "token").unwrap();
        assert!(r.close(&s.parent(), false).unwrap().unwrap().1);
        assert!(r.authorized(&s.shell(), "token").is_err());
        let s = r.open(&i, &"a".repeat(64), Page::Search, true).unwrap();
        assert!(!r.close(&s.parent(), false).unwrap().unwrap().1);
        assert!(r.current(&s));
        assert!(!r.list(&i)[0].visible);
        assert!(r.close(&s.parent(), true).unwrap().unwrap().1);
    }
    #[test]
    fn version_change_and_revocation_leave_cleanup_pending_until_confirmed() {
        let mut r = Registry::default();
        let i = uid();
        let old = r.open(&i, &"a".repeat(64), Page::Analysis, false).unwrap();
        r.bind(&old.shell(), &i, "old").unwrap();
        let next = r.open(&i, &"b".repeat(64), Page::Analysis, false).unwrap();
        assert!(!r.current(&old));
        assert!(r.current(&next));
        assert!(r.authorized(&old.shell(), "old").is_err());
        assert_eq!(r.snapshot(&i)["nativeCleanupPending"], true);
        r.retired_confirmed(&old);
        r.revoke(&i);
        assert!(!r.current(&next));
        assert!(r.snapshot(&i)["nativeCleanupPending"].as_bool().unwrap());
        r.retired_confirmed(&next);
        assert_eq!(r.snapshot(&i)["nativeCleanupPending"], false);
    }
    #[test]
    fn parent_labels_and_page_parameters_reject_unicode_or_extra_fields_without_panicking() {
        assert!(!parent_role(&format!("job-panel-{}a", "你".repeat(24))));
        for value in [
            json!({"page":"analysis","script":"不接受"}),
            json!({"page":"knowledge","context":{"knowledge":"Vue"},"url":"不接受"}),
            json!({"page":"detail","jobId":"id1","url":"不接受"}),
        ] {
            assert!(serde_json::from_value::<Page>(value).is_err());
        }
        let page: Page = serde_json::from_value(json!({"page":"detail","jobId":"id1"})).unwrap();
        assert_eq!(page.key().unwrap(), "detail/id1");
    }
    #[test]
    fn a_focus_request_is_consumed_once_even_when_layout_confirmation_is_later() {
        let mut r = Registry::default();
        let instance = uid();
        let first = r
            .open(&instance, &"a".repeat(64), Page::Search, false)
            .unwrap();
        r.bind(&first.shell(), &instance, "token").unwrap();
        r.layout(&first.shell(), "token", (10., 100., 800., 500.))
            .unwrap();
        r.focused(&first);
        let current = r.list(&instance)[0].clone();
        assert!(!current.focus);
        assert_ne!(current.serial, current.applied);
        r.confirmed(&current);
        assert!(!r.list(&instance)[0].focus);
        assert!(
            r.open(&instance, &"a".repeat(64), Page::Search, false)
                .unwrap()
                .focus
        );
    }
}
