//! 固定网页助手的展示账本与会话网关；原生回调只接触内存，不等待窗口操作。
use crate::{
    assistant_state::{self, Config, Document},
    assistant_webviews::{Navigation, Owner, PageState},
    clipboard_shortcuts::{Driver as ShortcutDriver, Registry, Route, Spec, Status},
    store::{err, AppResult, Store},
    widgets,
};
use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};
use tauri::Emitter;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

pub(crate) const PACKAGE: &str = "local.souldesk.widgetjs.ai-assistant";
pub(crate) const CAPABILITY: &str = "assistant.web";
struct Source {
    digest: String,
    document: Document,
}
fn source(store: &Store, instance: &str) -> AppResult<Source> {
    assistant_state::key(instance)?;
    let db = store.db.lock().map_err(err)?;
    let row: Option<(String,String,String)> = db.query_row(
        "SELECT p.id,p.manifest,p.digest FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE i.id=?1",
        [instance], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()
        .map_err(|_| "STORAGE_ERROR: 助手实例读取失败")?;
    let (package, raw, digest) = row.ok_or("SESSION_EXPIRED: 助手实例已移除")?;
    let manifest: widgets::Manifest =
        serde_json::from_str(&raw).map_err(|_| "DATA_UNAVAILABLE: 助手清单无效")?;
    if package != PACKAGE
        || manifest.id != PACKAGE
        || !manifest.capabilities.iter().any(|c| c == CAPABILITY)
    {
        return Err("CAPABILITY_DENIED: 仅声明网页助手能力的固定组件可访问".into());
    }
    let hidden: Option<String> = db
        .query_row(
            "SELECT value FROM app_settings WHERE key='hidden_all'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "STORAGE_ERROR: 助手显隐状态读取失败")?;
    match hidden.as_deref() {
        Some("true") => return Err("PRESENTATION_EXPIRED: 当前已隐藏全部组件".into()),
        None | Some("false") => (),
        _ => return Err("DATA_UNAVAILABLE: 助手显隐状态无效".into()),
    }
    Ok(Source {
        digest,
        document: assistant_state::load(&db, instance)?,
    })
}
pub(crate) fn shell_role(label: &str, instance: &str) -> bool {
    let prefix = format!("assistant-shell-{instance}-");
    label.strip_prefix(&prefix).is_some_and(canonical_uuid)
}
fn canonical_uuid(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id)
}
fn candidates(store: &Store) -> AppResult<Vec<String>> {
    let db = store.db.lock().map_err(err)?;
    let mut stmt=db.prepare("SELECT i.id FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE p.id=?1 ORDER BY i.id").map_err(err)?;
    let rows = stmt.query_map([PACKAGE], |r| r.get(0)).map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}
struct AppShortcuts<'a>(&'a tauri::AppHandle);
impl ShortcutDriver for AppShortcuts<'_> {
    fn register(&mut self, key: tauri_plugin_global_shortcut::Shortcut) -> AppResult<()> {
        if self.0.global_shortcut().is_registered(key) {
            return Err("SHORTCUT_ERROR: 已有宿主入口使用此快捷键".into());
        }
        self.0.global_shortcut().register(key).map_err(err)
    }
    fn unregister(&mut self, key: tauri_plugin_global_shortcut::Shortcut) -> AppResult<()> {
        self.0.global_shortcut().unregister(key).map_err(err)
    }
}
fn empty(args: &Value) -> AppResult<()> {
    if args.is_null() || args.as_object().is_some_and(|v| v.is_empty()) {
        Ok(())
    } else {
        Err("INVALID_ARGUMENT: 此助手方法不接受参数".into())
    }
}
#[derive(Clone)]
struct Page {
    state: PageState,
    tag: Option<String>,
}
#[derive(Clone)]
pub(crate) struct Surface {
    pub owner: Owner,
    pub config: Config,
    pub digest: String,
    pub serial: u64,
    applied: u64,
    pub visible: bool,
    pub ready: bool,
    pub fault: Option<String>,
    token: Option<String>,
    focused: bool,
    overlays: HashSet<String>,
    pages: Vec<Page>,
}
impl Surface {
    pub(crate) fn overlay(&self) -> bool {
        !self.ready || self.fault.is_some() || !self.overlays.is_empty()
    }
    pub(crate) fn snapshot(&self) -> Value {
        json!({"presentation":self.owner.presentation,"visible":self.visible,"serial":self.serial,"nativeSettled":self.applied==self.serial,"overlays":self.overlays,
            "phase":if self.fault.is_some(){"failed"}else if !self.visible{"hidden"}else if self.ready{"ready"}else{"opening"},
            "error":self.fault,"pages":self.pages.iter().map(|p|p.state.snapshot()).collect::<Vec<_>>()})
    }
}
#[derive(Clone)]
pub(crate) struct Permit {
    pub label: String,
    pub owner: Owner,
    slot: u8,
    epoch: u64,
    tag: String,
}
#[derive(Default)]
struct Book {
    entries: HashMap<String, Surface>,
    retired: Vec<Owner>,
    auth: HashMap<String, Permit>,
    closing_auth: Vec<String>,
    native_errors: HashMap<String, String>,
    sequence: u64,
}
impl Book {
    fn next(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }
    fn revoke_auth(&mut self, instance: &str, slot: Option<u8>) {
        let labels = self
            .auth
            .iter()
            .filter(|(_, p)| p.owner.instance == instance && slot.is_none_or(|s| s == p.slot))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for label in labels {
            self.auth.remove(&label);
            self.closing_auth.push(label);
        }
    }
    fn cancel(&mut self, instance: &str) {
        self.revoke_auth(instance, None);
        if let Some(s) = self.entries.remove(instance) {
            self.retired.push(s.owner);
        }
    }
    fn sync(&mut self, instance: &str, source: &Source) -> AppResult<()> {
        let Some(old) = self.entries.get(instance) else {
            return Ok(());
        };
        if old.digest != source.digest {
            self.cancel(instance);
            return Ok(());
        }
        if old.owner.revision > source.document.revision {
            return Err("REVISION_CONFLICT: 助手展示配置已更新".into());
        }
        if old.owner.revision == source.document.revision {
            return if old.config == source.document.config {
                Ok(())
            } else {
                Err("DATA_UNAVAILABLE: 同版本助手配置不一致".into())
            };
        }
        let config = &source.document.config;
        let proxy_changed = old.config.proxy != config.proxy;
        let reset = (0..4)
            .filter(|i| {
                proxy_changed
                    || *i >= config.page_count as usize
                    || *i >= old.pages.len()
                    || old.config.platforms[*i] != config.platforms[*i]
            })
            .collect::<HashSet<_>>();
        if proxy_changed {
            self.revoke_auth(instance, None);
        } else {
            for slot in &reset {
                self.revoke_auth(instance, Some(*slot as u8));
            }
        }
        let serial = self.next();
        let s = self.entries.get_mut(instance).expect("当前展示存在");
        s.pages.truncate(config.page_count as usize);
        s.overlays.retain(|id| {
            id.strip_prefix("select")
                .and_then(|v| v.parse::<u8>().ok())
                .is_none_or(|slot| slot < config.page_count)
        });
        for i in 0..config.page_count as usize {
            if i >= s.pages.len() {
                s.pages.push(Page {
                    state: PageState::new(config.platforms[i], serial),
                    tag: None,
                });
            } else if reset.contains(&i) {
                let tag = if proxy_changed {
                    None
                } else {
                    s.pages[i].tag.clone()
                };
                s.pages[i] = Page {
                    state: PageState::new(config.platforms[i], serial),
                    tag,
                };
            }
        }
        s.serial = serial;
        s.owner.revision = source.document.revision;
        s.config = config.clone();
        if config.tip_progress >= 3 {
            s.overlays.remove("guide");
        }
        if !s.visible && !config.background_enabled {
            self.cancel(instance);
        }
        Ok(())
    }
    fn open(&mut self, instance: &str, source: &Source, retry: bool) -> AppResult<Surface> {
        self.sync(instance, source)?;
        if retry
            || self
                .entries
                .get(instance)
                .is_some_and(|s| s.fault.is_some())
        {
            self.cancel(instance);
        }
        let serial = self.next();
        let s = self.entries.entry(instance.into()).or_insert_with(|| {
            let presentation = uuid::Uuid::new_v4().to_string();
            Surface {
                owner: Owner {
                    instance: instance.into(),
                    parent: format!("assistant-panel-{instance}-{presentation}"),
                    presentation,
                    revision: source.document.revision,
                },
                config: source.document.config.clone(),
                digest: source.digest.clone(),
                serial,
                applied: 0,
                visible: true,
                ready: false,
                fault: None,
                token: None,
                focused: false,
                overlays: if source.document.config.tip_progress < 3 {
                    HashSet::from(["guide".into()])
                } else {
                    HashSet::new()
                },
                pages: (0..source.document.config.page_count as usize)
                    .map(|i| Page {
                        state: PageState::new(source.document.config.platforms[i], serial),
                        tag: None,
                    })
                    .collect(),
            }
        });
        s.visible = true;
        s.serial = serial;
        Ok(s.clone())
    }
    fn hide(&mut self, instance: &str) {
        self.revoke_auth(instance, None);
        let serial = self.next();
        if let Some(s) = self.entries.get_mut(instance) {
            if s.config.background_enabled {
                s.visible = false;
                s.focused = false;
                s.serial = serial;
            } else {
                self.cancel(instance);
            }
        }
    }
    fn current(&self, plan: &Surface) -> bool {
        self.entries
            .get(&plan.owner.instance)
            .is_some_and(|s| s.owner == plan.owner && s.serial == plan.serial)
    }
    fn shell(&self, window: &str, instance: &str) -> bool {
        self.entries
            .get(instance)
            .is_some_and(|s| s.owner.shell() == window)
    }
    fn bind(&mut self, window: &str, instance: &str, token: &str) -> AppResult<()> {
        if !self.shell(window, instance) {
            return Err("PRESENTATION_EXPIRED: 助手本机壳已失效".into());
        }
        if self.entries[instance].token.as_deref() != Some(token) {
            self.revoke_auth(instance, None);
            let serial = self.next();
            let s = self.entries.get_mut(instance).unwrap();
            s.token = Some(token.into());
            s.ready = false;
            s.serial = serial;
            s.overlays.clear();
            if s.config.tip_progress < 3 {
                s.overlays.insert("guide".into());
            }
        }
        Ok(())
    }
    fn shell_token(&self, window: &str, instance: &str, token: &str) -> AppResult<()> {
        if self.shell(window, instance) && self.entries[instance].token.as_deref() == Some(token) {
            Ok(())
        } else {
            Err("SESSION_EXPIRED: 助手本机壳会话已失效".into())
        }
    }
    fn ready(&mut self, window: &str, instance: &str, token: &str) -> AppResult<()> {
        self.shell_token(window, instance, token)?;
        if !self.entries[instance].ready {
            let serial = self.next();
            let s = self.entries.get_mut(instance).unwrap();
            s.ready = true;
            s.serial = serial;
        }
        Ok(())
    }
    fn overlay(&mut self, instance: &str, id: &str, visible: bool) -> AppResult<()> {
        if ![
            "guide", "settings", "select0", "select1", "select2", "select3",
        ]
        .contains(&id)
        {
            return Err("INVALID_ARGUMENT: 助手覆盖层标识无效".into());
        }
        if visible {
            self.revoke_auth(instance, None);
        }
        let serial = self.next();
        let s = self
            .entries
            .get_mut(instance)
            .ok_or("PRESENTATION_EXPIRED: 助手窗口已关闭")?;
        if visible {
            s.overlays.insert(id.into());
        } else {
            s.overlays.remove(id);
        }
        s.serial = serial;
        Ok(())
    }
    fn page(&mut self, owner: &Owner, slot: u8, tag: &str, event: Option<Navigation>) -> bool {
        let Some(s) = self.entries.get_mut(&owner.instance).filter(|s| {
            s.owner.presentation == owner.presentation
                && s.owner.parent == owner.parent
                && s.fault.is_none()
        }) else {
            return false;
        };
        if !s.ready || !s.visible && !s.config.background_enabled {
            return false;
        }
        let Some(p) = s.pages.get_mut(slot as usize) else {
            return false;
        };
        match event {
            None => {
                if s.owner.revision != owner.revision
                    || !s.visible
                    || !s.overlays.is_empty()
                    || !canonical_uuid(tag)
                {
                    return false;
                }
                p.tag = Some(tag.into());
                p.state = PageState::new(p.state.platform, p.state.epoch);
                self.revoke_auth(&owner.instance, Some(slot));
                true
            }
            Some(event) => p.tag.as_deref() == Some(tag) && p.state.event(event),
        }
    }
    fn permit_current(&self, p: &Permit) -> bool {
        self.entries.get(&p.owner.instance).is_some_and(|s| {
            s.owner.presentation == p.owner.presentation
                && s.visible
                && s.ready
                && !s.overlay()
                && s.pages.get(p.slot as usize).is_some_and(|page| {
                    page.state.epoch == p.epoch
                        && page.tag.as_deref() == Some(p.tag.as_str())
                        && page.state.popup_ready()
                })
        })
    }
    #[cfg(test)]
    fn reserve_popup(&mut self, owner: &Owner, slot: u8, tag: &str) -> AppResult<Permit> {
        self.reserve_popup_from(owner, slot, tag, None)
    }
    fn reserve_popup_from(
        &mut self,
        owner: &Owner,
        slot: u8,
        tag: &str,
        from: Option<&str>,
    ) -> AppResult<Permit> {
        let s = self
            .entries
            .get(&owner.instance)
            .ok_or("PRESENTATION_EXPIRED: 助手已关闭")?;
        let page = s
            .pages
            .get(slot as usize)
            .ok_or("PRESENTATION_EXPIRED: 助手分屏已移除")?;
        let label = format!(
            "assistant-auth-{}-{}-{slot}-{}",
            owner.instance,
            owner.presentation,
            uuid::Uuid::new_v4()
        );
        let permit = Permit {
            label,
            owner: s.owner.clone(),
            slot,
            epoch: page.state.epoch,
            tag: tag.into(),
        };
        if owner.presentation != s.owner.presentation
            || owner.parent != s.owner.parent
            || if let Some(label) = from {
                !self.auth.get(label).is_some_and(|p| {
                    p.owner.instance == owner.instance
                        && p.slot == slot
                        && p.tag == tag
                        && self.permit_current(p)
                })
            } else {
                !s.focused
            }
            || !self.permit_current(&permit)
            || self
                .auth
                .values()
                .filter(|p| p.owner.instance == owner.instance)
                .count()
                >= 4
        {
            return Err("CAPABILITY_DENIED: 当前助手页面不能打开登录窗口".into());
        }
        self.auth.insert(permit.label.clone(), permit.clone());
        Ok(permit)
    }
    fn auth_current(&self, label: &str) -> bool {
        self.auth.get(label).is_some_and(|p| self.permit_current(p))
    }
    fn fault(&mut self, plan: &Surface) {
        if !self.current(plan) {
            return;
        }
        self.revoke_auth(&plan.owner.instance, None);
        let serial = self.next();
        let s = self.entries.get_mut(&plan.owner.instance).unwrap();
        s.fault = Some("助手原生显示操作失败，请重试；已保存的设置与登录资料保留".into());
        s.ready = false;
        s.serial = serial;
    }
}

#[derive(Default)]
pub(crate) struct Service {
    book: Mutex<Book>,
    stopped: AtomicBool,
    controller: Mutex<crate::assistant_webviews::Controller>,
    jobs: Mutex<HashMap<String, u64>>,
    // 复用已有的底层键登记器，不调用剪切板采集、粘贴或搜索业务。
    shortcuts: Mutex<Registry>,
    shortcut_states: Mutex<HashMap<String, (u64, Status)>>,
    ticking: AtomicBool,
}
impl Service {
    fn shortcuts_with(&self, store: &Store, driver: &mut impl ShortcutDriver) -> AppResult<()> {
        let mut sources = Vec::new();
        if !self.stopped.load(Ordering::Acquire) {
            for id in candidates(store)? {
                if let Ok(s) = source(store, &id) {
                    sources.push((id, s));
                }
            }
        }
        let specs = sources
            .iter()
            .filter(|(_, s)| s.document.config.background_enabled)
            .map(|(id, s)| Spec {
                instance: id.clone(),
                revision: s.document.revision,
                shortcut: s.document.config.shortcut.clone(),
            })
            .collect::<Vec<_>>();
        let primary = store
            .preference("shortcut")
            .unwrap_or_else(|| "Ctrl+Alt+Space".into());
        let mut keys = self.shortcuts.lock().map_err(err)?;
        if self.stopped.load(Ordering::Acquire) {
            keys.release(driver)?;
            self.shortcut_states.lock().map_err(err)?.clear();
            return Ok(());
        }
        let states = keys.sync(&specs, &primary, driver);
        *self.shortcut_states.lock().map_err(err)? = sources
            .into_iter()
            .map(|(id, s)| {
                let mut status = states.get(&id).cloned().unwrap_or_default();
                status.error = status.error.map(|e| e.replace("剪切板", "网页助手"));
                (id, (s.document.revision, status))
            })
            .collect();
        Ok(())
    }
    pub(crate) fn stop_shortcuts(&self, app: &tauri::AppHandle) -> AppResult<()> {
        self.shortcuts
            .lock()
            .map_err(err)?
            .release(&mut AppShortcuts(app))
    }
    pub(crate) fn monitor_changed(&self, app: &tauri::AppHandle) -> AppResult<()> {
        let ids = {
            let mut b = self.book.lock().map_err(err)?;
            let serial = b.next();
            b.entries
                .iter_mut()
                .filter(|(_, s)| s.visible && s.fault.is_none())
                .map(|(id, s)| {
                    s.serial = serial;
                    id.clone()
                })
                .collect::<Vec<_>>()
        };
        for id in ids {
            self.kick(app, &id);
        }
        Ok(())
    }
    pub(crate) fn shortcut_route(&self, event: u32) -> Option<Spec> {
        if self.stopped.load(Ordering::Acquire) {
            return None;
        }
        // 系统回调不等待正在主线程注册/释放按键的工作线程。
        match self.shortcuts.try_lock().ok()?.route(event, "") {
            Route::Clipboard(s) => Some(s),
            _ => None,
        }
    }
    fn toggle(&self, store: &Store, id: &str, expected: Option<&Spec>) -> AppResult<()> {
        let _native = store.native_actions.lock().map_err(err)?;
        if self.stopped.load(Ordering::Acquire) {
            return Err("SHUTTING_DOWN: 网页助手已停止".into());
        }
        let src = source(store, id)?;
        if !src.document.config.background_enabled {
            return Err("CAPABILITY_DENIED: 助手后台入口未启用".into());
        }
        if expected.is_some_and(|s| {
            !src.document.config.background_enabled
                || s.revision != src.document.revision
                || s.shortcut != src.document.config.shortcut
        }) {
            return Err("PRESENTATION_EXPIRED: 助手快捷键配置已变化".into());
        }
        let mut b = self.book.lock().map_err(err)?;
        b.sync(id, &src)?;
        if b.entries
            .get(id)
            .is_some_and(|s| s.visible && s.fault.is_none())
        {
            b.hide(id);
        } else {
            b.open(id, &src, false)?;
        }
        Ok(())
    }
    pub(crate) fn shortcut_activated(&self, app: &tauri::AppHandle, spec: &Spec) -> AppResult<()> {
        use tauri::Manager;
        self.toggle(
            &app.state::<crate::Core>().store,
            &spec.instance,
            Some(spec),
        )?;
        self.kick(app, &spec.instance);
        Ok(())
    }
    pub(crate) fn tray(&self, app: &tauri::AppHandle) -> AppResult<()> {
        use tauri::Manager;
        let store = &app.state::<crate::Core>().store;
        let mut ids = candidates(store)?;
        ids.retain(|id| source(store, id).is_ok_and(|s| s.document.config.background_enabled));
        if ids.is_empty() {
            return crate::windows::show(app, true);
        }
        for id in ids {
            self.toggle(store, &id, None)?;
            self.kick(app, &id);
        }
        Ok(())
    }
    pub(crate) fn tick(&self, app: &tauri::AppHandle) {
        if self.stopped.load(Ordering::Acquire) || self.ticking.swap(true, Ordering::AcqRel) {
            return;
        }
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            use tauri::Manager;
            let core = handle.state::<crate::Core>();
            let s = &core.assistant;
            let _ = s.shortcuts_with(&core.store, &mut AppShortcuts(&handle));
            let plans = s
                .book
                .lock()
                .map(|b| b.entries.values().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            for p in plans {
                let changed = (|| -> AppResult<bool> {
                    let _native = core.store.native_actions.lock().map_err(err)?;
                    let latest = source(&core.store, &p.owner.instance);
                    let mut b = s.book.lock().map_err(err)?;
                    // 同步外部升级/删除/隐藏全部；后台轮询不会自行重开网页或重试故障。
                    if let Ok(src) = latest {
                        b.sync(&p.owner.instance, &src)?;
                    } else {
                        b.cancel(&p.owner.instance);
                    }
                    Ok(!b.current(&p))
                })()
                .unwrap_or(false);
                if changed {
                    s.kick(&handle, &p.owner.instance);
                }
            }
            s.ticking.store(false, Ordering::Release);
        });
    }
    pub(crate) fn kick(&self, app: &tauri::AppHandle, instance: &str) {
        let Ok(mut jobs) = self.jobs.lock() else {
            return;
        };
        if let Some(ticket) = jobs.get_mut(instance) {
            *ticket += 1;
            return;
        }
        jobs.insert(instance.into(), 0);
        drop(jobs);
        let handle = app.clone();
        let id = instance.to_owned();
        tauri::async_runtime::spawn_blocking(move || loop {
            use tauri::Manager;
            let core = handle.state::<crate::Core>();
            let service = &core.assistant;
            let stamp = service.jobs.lock().ok().and_then(|j| j.get(&id).copied());
            let Some(stamp) = stamp else {
                break;
            };
            if let Err(error) = crate::windows::serialized(&handle, |app| service.render(app, &id))
            {
                if !error.starts_with("SESSION_EXPIRED:")
                    && !error.starts_with("PRESENTATION_EXPIRED:")
                {
                    // 不记录浏览器原始错误，避免地址或登录参数进入诊断日志。
                    let _ = handle.emit(
                        "host:assistant-error",
                        json!({"instance":id,"message":"网页助手原生操作失败，请重试"}),
                    );
                }
            }
            let Ok(mut jobs) = service.jobs.lock() else {
                break;
            };
            if jobs.get(&id) == Some(&stamp) {
                jobs.remove(&id);
                break;
            }
        });
    }
    pub(crate) fn current(&self, plan: &Surface) -> bool {
        !self.stopped.load(Ordering::Acquire) && self.book.lock().is_ok_and(|b| b.current(plan))
    }
    pub(crate) fn shell_failed(&self, window: &str, instance: &str) -> AppResult<()> {
        let mut b = self.book.lock().map_err(err)?;
        let plan = b
            .entries
            .get(instance)
            .filter(|s| s.owner.shell() == window)
            .cloned()
            .ok_or("PRESENTATION_EXPIRED: 助手本机壳已关闭")?;
        b.fault(&plan);
        Ok(())
    }
    pub(crate) fn window_event(
        &self,
        app: &tauri::AppHandle,
        label: &str,
        event: &tauri::WindowEvent,
    ) -> bool {
        if !label.starts_with("assistant-panel-") && !label.starts_with("assistant-auth-") {
            return false;
        }
        let mut kick = None;
        if let Ok(mut b) = self.book.lock() {
            if label.starts_with("assistant-auth-") {
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    b.auth.remove(label);
                }
                return true;
            }
            let current = b
                .entries
                .iter()
                .find(|(_, s)| s.owner.parent == label)
                .map(|(id, s)| (id.clone(), s.clone()));
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    if let Some((id, _)) = current {
                        b.hide(&id);
                        kick = Some(id);
                    }
                }
                tauri::WindowEvent::Focused(focused) => {
                    if let Some((id, _)) = current {
                        b.entries.get_mut(&id).unwrap().focused = *focused;
                    }
                }
                tauri::WindowEvent::Destroyed => {
                    if let Some((id, p)) = current {
                        b.fault(&p);
                        kick = Some(id);
                    }
                }
                _ => (),
            }
        }
        if let Some(id) = kick {
            self.kick(app, &id);
        }
        true
    }
    pub(crate) fn cleanup(&self, app: &tauri::AppHandle) -> AppResult<()> {
        let ids = {
            let b = self.book.lock().map_err(err)?;
            b.retired
                .iter()
                .map(|o| o.instance.clone())
                .collect::<HashSet<_>>()
        };
        for id in ids {
            self.render(app, &id)?;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    fn render(&self, _app: &tauri::AppHandle, _instance: &str) -> AppResult<()> {
        Err("UNSUPPORTED: 当前网页助手仅支持Windows".into())
    }
    pub(crate) fn current_window(&self, window: &str, instance: &str) -> bool {
        !self.stopped.load(Ordering::Acquire)
            && self.book.lock().is_ok_and(|b| b.shell(window, instance))
    }
    pub(crate) fn bind(&self, window: &str, instance: &str, token: &str) -> AppResult<()> {
        self.book.lock().map_err(err)?.bind(window, instance, token)
    }
    pub(crate) fn close_session(&self, window: &str, token: &str) -> AppResult<Option<String>> {
        let mut b = self.book.lock().map_err(err)?;
        let id = b
            .entries
            .iter()
            .find(|(_, s)| s.owner.shell() == window && s.token.as_deref() == Some(token))
            .map(|(id, _)| id.clone());
        if let Some(id) = &id {
            b.revoke_auth(id, None);
            let serial = b.next();
            let s = b.entries.get_mut(id).unwrap();
            s.ready = false;
            s.token = None;
            s.serial = serial;
        }
        Ok(id)
    }
    pub(crate) fn cancel(&self, instance: &str) -> AppResult<()> {
        self.book.lock().map_err(err)?.cancel(instance);
        Ok(())
    }
    pub(crate) fn cancel_all(&self) -> AppResult<()> {
        let mut b = self.book.lock().map_err(err)?;
        let ids = b.entries.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            b.cancel(&id);
        }
        Ok(())
    }
    pub(crate) fn stop(&self) -> AppResult<()> {
        self.stopped.store(true, Ordering::Release);
        self.cancel_all()
    }
    pub(crate) fn page(
        &self,
        owner: &Owner,
        slot: u8,
        tag: &str,
        event: Option<Navigation>,
    ) -> bool {
        !self.stopped.load(Ordering::Acquire)
            && self
                .book
                .lock()
                .is_ok_and(|mut b| b.page(owner, slot, tag, event))
    }
    pub(crate) fn access(
        &self,
        store: &Store,
        sessions: &widgets::Sessions,
        window: &str,
        token: &str,
        method: &str,
        args: Value,
    ) -> AppResult<(String, Value)> {
        let _native = store.native_actions.lock().map_err(err)?;
        if self.stopped.load(Ordering::Acquire) {
            return Err("SHUTTING_DOWN: 网页助手已停止".into());
        }
        widgets::with_capability(store, sessions, window, token, CAPABILITY, |session| {
            let instance = &session.instance;
            let src = source(store, instance)?;
            let mut b = self.book.lock().map_err(err)?;
            b.sync(instance, &src)?;
            let shell = window.starts_with("assistant-");
            if shell {
                b.shell_token(window, instance, token)?;
            }
            let value = match method {
                "assistant.status" => {
                    empty(&args)?;
                    Value::Null
                }
                "runtime.ready" if shell => {
                    empty(&args)?;
                    b.ready(window, instance, token)?;
                    Value::Null
                }
                "assistant.configure" => {
                    // 提交失败时账本不变化；同一回执重放返回当前配置，不重放旧副作用。
                    let result = assistant_state::mutate(store, instance, args)?;
                    let current = source(store, instance)?;
                    b.sync(instance, &current)?;
                    result
                }
                "assistant.open" | "assistant.retry" | "assistant.hide" => {
                    #[derive(Deserialize)]
                    #[serde(rename_all = "camelCase", deny_unknown_fields)]
                    struct Request {
                        expected_revision: u64,
                    }
                    let r: Request = serde_json::from_value(args)
                        .map_err(|_| "INVALID_ARGUMENT: 助手动作只接受expectedRevision")?;
                    if r.expected_revision != src.document.revision {
                        return Err("REVISION_CONFLICT: 助手设置已更新，请重新读取".into());
                    }
                    if method == "assistant.hide" {
                        b.hide(instance);
                    } else {
                        b.open(instance, &src, method == "assistant.retry")?;
                    }
                    Value::Null
                }
                "assistant.overlay" if shell => {
                    #[derive(Deserialize)]
                    #[serde(rename_all = "camelCase", deny_unknown_fields)]
                    struct Request {
                        expected_revision: u64,
                        id: String,
                        visible: bool,
                    }
                    let r: Request = serde_json::from_value(args)
                        .map_err(|_| "INVALID_ARGUMENT: 助手覆盖层参数无效")?;
                    if r.expected_revision != src.document.revision {
                        return Err("REVISION_CONFLICT: 助手设置已更新".into());
                    }
                    b.overlay(instance, &r.id, r.visible)?;
                    Value::Null
                }
                _ => return Err("CAPABILITY_DENIED: 未开放此助手方法".into()),
            };
            let current = source(store, instance)?;
            let mut result = json!({"revision":current.document.revision,"config":current.document.config,
                "surface":b.entries.get(instance).map(Surface::snapshot),
                "nativeCleanupPending":b.retired.iter().any(|o|o.instance==*instance),"nativeError":b.native_errors.get(instance)});
            let shortcut = self
                .shortcut_states
                .lock()
                .map_err(err)?
                .get(instance)
                .filter(|(rev, _)| *rev == current.document.revision)
                .map(|(_, s)| s.clone())
                .unwrap_or_default();
            result["shortcutRegistered"] = json!(shortcut.registered);
            result["shortcutError"] = json!(shortcut.error);
            if !value.is_null() {
                result["replayed"] = value["replayed"].clone();
            }
            Ok((instance.clone(), result))
        })
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use crate::{
        assistant_state::Position,
        assistant_webviews::{self},
        Core,
    };
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    use tauri::{
        webview::{NewWindowFeatures, NewWindowResponse, WebviewBuilder},
        AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, Window,
    };

    fn destroy(app: &AppHandle, label: &str) -> AppResult<()> {
        if let Some(window) = app.get_window(label) {
            window.destroy().map_err(err)?;
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while app.get_window(label).is_some() {
            if Instant::now() >= deadline {
                return Err("NATIVE_ERROR: 助手旧窗口释放尚未确认".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
    fn driver(app: &AppHandle) -> assistant_webviews::native::Native {
        let popup_app = app.clone();
        let page_app = app.clone();
        assistant_webviews::native::Native::new(
            app.clone(),
            Arc::new(move |owner, slot, tag, url, features| {
                popup(&popup_app, owner, slot, tag, url, features, None)
            }),
            Arc::new(move |owner, slot, tag, event| {
                page_app
                    .state::<Core>()
                    .assistant
                    .page(owner, slot, tag, event)
            }),
        )
    }
    fn popup(
        app: &AppHandle,
        owner: &Owner,
        slot: u8,
        tag: &str,
        url: url::Url,
        features: NewWindowFeatures,
        from: Option<&str>,
    ) -> NewWindowResponse<tauri::Wry> {
        if !assistant_webviews::web_navigation_allowed(&url) {
            return NewWindowResponse::Deny;
        }
        if from.is_some_and(|label| {
            !app.get_webview_window(label)
                .is_some_and(|w| w.is_focused().unwrap_or(false))
        }) {
            return NewWindowResponse::Deny;
        }
        let core = app.state::<Core>();
        let service = &core.assistant;
        if service.stopped.load(Ordering::Acquire) || core.quitting.load(Ordering::Acquire) {
            return NewWindowResponse::Deny;
        }
        let accepted = (|| -> AppResult<_> {
            let mut b = service.book.lock().map_err(err)?;
            let permit = b.reserve_popup_from(owner, slot, tag, from)?;
            let config = b.entries[&owner.instance].config.clone();
            Ok((permit, config))
        })();
        let Ok((permit, config)) = accepted else {
            return NewWindowResponse::Deny;
        };
        let label = permit.label.clone();
        let nav_app = app.clone();
        let nav_label = label.clone();
        let nested_app = app.clone();
        let nested_owner = owner.clone();
        let nested_tag = tag.to_owned();
        let nested_label = label.clone();
        let attempt = (|| -> AppResult<_> {
            // 新窗口继承发起页的WebView2环境；不注入宿主脚本，不授予IPC权限。
            let mut builder = WebviewWindowBuilder::new(
                app,
                &label,
                WebviewUrl::External("about:blank".parse().map_err(err)?),
            )
            .title("网页助手 · 手动登录")
            .inner_size(760., 700.)
            .visible(false)
            .focused(false)
            .devtools(false)
            .skip_taskbar(false)
            .window_features(features)
            .data_directory(assistant_state::profile_directory(
                &core.store.root,
                &owner.instance,
            )?)
            .on_navigation(move |url| {
                assistant_webviews::web_navigation_allowed(url)
                    && nav_app
                        .state::<Core>()
                        .assistant
                        .book
                        .lock()
                        .is_ok_and(|b| b.auth_current(&nav_label))
            })
            .on_new_window(move |url, features| {
                popup(
                    &nested_app,
                    &nested_owner,
                    slot,
                    &nested_tag,
                    url,
                    features,
                    Some(&nested_label),
                )
            });
            if let Some(args) = config.proxy.browser_arguments()? {
                builder = builder.additional_browser_args(&args);
            }
            let window = builder.build().map_err(err)?;
            if service.stopped.load(Ordering::Acquire)
                || !service.book.lock().map_err(err)?.auth_current(&label)
            {
                let _ = window.destroy();
                return Err("PRESENTATION_EXPIRED: 登录窗口授权已撤销".into());
            }
            window.show().map_err(err)?;
            window.set_focus().map_err(err)?;
            Ok(window)
        })();
        match attempt {
            Ok(window) => NewWindowResponse::Create { window },
            Err(_) => {
                if let Ok(mut b) = service.book.lock() {
                    b.auth.remove(&label);
                    b.closing_auth.push(label);
                }
                service.kick(app, &owner.instance);
                NewWindowResponse::Deny
            }
        }
    }
    fn area(
        app: &AppHandle,
        window: Option<&Window>,
    ) -> AppResult<crate::desktop_geometry::WorkArea> {
        let monitor = window.and_then(|w| w.current_monitor().ok().flatten());
        if let Some(m) = monitor {
            let work = m.work_area();
            Ok(crate::desktop_geometry::WorkArea {
                left: work.position.x,
                top: work.position.y,
                width: work.size.width,
                height: work.size.height,
                scale_factor: m.scale_factor(),
            })
        } else {
            crate::windows::area(app, true)
        }
    }
    fn geometry(
        app: &AppHandle,
        window: Option<&Window>,
        config: &Config,
    ) -> AppResult<(
        crate::desktop_geometry::WorkArea,
        crate::desktop_geometry::PanelGeometry,
    )> {
        let a = area(app, window)?;
        let mut g = crate::desktop_geometry::panel_geometry(a, config.width_ratio, 320.)?;
        if config.position == Position::Left {
            g.left = a.left;
        }
        Ok((a, g))
    }
    fn build(app: &AppHandle, plan: &Surface) -> AppResult<Window> {
        let directory = plan.owner.shell_profile(&app.state::<Core>().store.root)?;
        std::fs::create_dir_all(&directory).map_err(err)?;
        let (area, g) = geometry(app, None, &plan.config)?;
        let window = tauri::window::WindowBuilder::new(app, &plan.owner.parent)
            .title("网页 AI 助手")
            .decorations(false)
            .resizable(false)
            .skip_taskbar(true)
            .visible(false)
            .focused(false)
            .inner_size(
                g.width as f64 / area.scale_factor,
                g.height as f64 / area.scale_factor,
            )
            .build()
            .map_err(err)?;
        let outside = if plan.config.position == Position::Right {
            area.left + area.width as i32
        } else {
            area.left - g.width as i32
        };
        window
            .set_position(tauri::PhysicalPosition::new(outside, g.top))
            .map_err(err)?;
        let url = format!("index.html?assistantPanel={}", plan.owner.instance);
        let shell = WebviewBuilder::new(plan.owner.shell(), WebviewUrl::App(url.into()))
            .data_directory(directory)
            .initialization_script(crate::windows::host_script(app))
            .auto_resize()
            .focused(false)
            .on_navigation({
                let app = app.clone();
                move |url| {
                    let origin = url.origin().ascii_serialization();
                    origin == "http://tauri.localhost"
                        || origin == "https://tauri.localhost"
                        || url.scheme() == "tauri" && url.host_str() == Some("localhost")
                        || app
                            .config()
                            .build
                            .dev_url
                            .as_ref()
                            .is_some_and(|dev| dev.origin() == url.origin())
                }
            })
            .on_new_window(|_, _| NewWindowResponse::Deny);
        window
            .add_child(
                shell,
                tauri::LogicalPosition::new(0., 0.),
                tauri::LogicalSize::new(
                    g.width as f64 / area.scale_factor,
                    g.height as f64 / area.scale_factor,
                ),
            )
            .map_err(err)?;
        Ok(window)
    }
    fn slide(window: &Window, target: (i32, i32), current: impl Fn() -> bool) -> AppResult<()> {
        let start = window.outer_position().map_err(err)?;
        let began = Instant::now();
        loop {
            if !current() {
                return Err("SESSION_EXPIRED: 助手滑动已被更新动作替换".into());
            }
            let t = (began.elapsed().as_secs_f64() / 0.5).min(1.);
            let eased = 1. - (1. - t).powi(3);
            window
                .set_position(tauri::PhysicalPosition::new(
                    (start.x as f64 + (target.0 - start.x) as f64 * eased).round() as i32,
                    (start.y as f64 + (target.1 - start.y) as f64 * eased).round() as i32,
                ))
                .map_err(err)?;
            if t >= 1. {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(16));
        }
    }
    impl Service {
        fn plan(&self, instance: &str) -> AppResult<Option<Surface>> {
            Ok(self
                .book
                .lock()
                .map_err(err)?
                .entries
                .get(instance)
                .cloned())
        }
        pub(super) fn render(&self, app: &AppHandle, instance: &str) -> AppResult<()> {
            let core = app.state::<Core>();
            {
                let _native = core.store.native_actions.lock().map_err(err)?;
                let latest = source(&core.store, instance);
                let mut b = self.book.lock().map_err(err)?;
                if self.stopped.load(Ordering::Acquire) || core.quitting.load(Ordering::Acquire) {
                    b.cancel(instance);
                } else if let Ok(src) = latest {
                    b.sync(instance, &src)?;
                } else {
                    b.cancel(instance);
                }
            }
            let mut native = driver(app);
            let mut controller = self.controller.lock().map_err(err)?;
            let expected = self.plan(instance)?;
            let cleanup = (|| -> AppResult<()> {
                let retired = {
                    self.book
                        .lock()
                        .map_err(err)?
                        .retired
                        .iter()
                        .filter(|o| o.instance == instance)
                        .cloned()
                        .collect::<Vec<_>>()
                };
                for owner in retired {
                    // 先关闭该实例的所有网页/登录窗并确认释放，再使用相同资料目录。
                    controller.release(instance, &mut native)?;
                    destroy(app, &owner.parent)?;
                    core.sessions
                        .0
                        .lock()
                        .map_err(err)?
                        .retain(|_, s| s.window != owner.shell());
                    self.book
                        .lock()
                        .map_err(err)?
                        .retired
                        .retain(|o| o != &owner);
                }
                let closing = {
                    self.book
                        .lock()
                        .map_err(err)?
                        .closing_auth
                        .iter()
                        .filter(|label| label.starts_with(&format!("assistant-auth-{instance}-")))
                        .cloned()
                        .collect::<Vec<_>>()
                };
                for label in closing {
                    destroy(app, &label)?;
                    self.book
                        .lock()
                        .map_err(err)?
                        .closing_auth
                        .retain(|old| old != &label);
                }
                Ok(())
            })();
            if let Err(error) = cleanup {
                let mut b = self.book.lock().map_err(err)?;
                b.native_errors.insert(
                    instance.into(),
                    "助手旧窗口释放尚未确认，请重试或退出程序；登录资料保留".into(),
                );
                if let Some(plan) = expected {
                    b.fault(&plan);
                }
                return Err(error);
            }
            self.book
                .lock()
                .map_err(err)?
                .native_errors
                .remove(instance);
            let Some(mut plan) = self.plan(instance)? else {
                return Ok(());
            };
            if plan.fault.is_some() {
                controller.release(instance, &mut native)?;
                return Ok(());
            }
            let work = (|| -> AppResult<()> {
                let window = match app.get_window(&plan.owner.parent) {
                    Some(w) => w,
                    None => {
                        if !self.current(&plan) {
                            return Err("SESSION_EXPIRED: 助手窗口计划已失效".into());
                        }
                        build(app, &plan)?
                    }
                };
                let presentation = plan.owner.presentation.clone();
                if !plan.ready {
                    let (area, g) = geometry(app, Some(&window), &plan.config)?;
                    controller.reconcile(
                        &core.store.root,
                        &plan.owner,
                        &plan.config,
                        (
                            g.width as f64 / area.scale_factor,
                            g.height as f64 / area.scale_factor,
                        ),
                        plan.visible,
                        true,
                        &mut native,
                        || self.current(&plan),
                    )?;
                }
                let deadline = Instant::now() + Duration::from_secs(12);
                loop {
                    plan = self
                        .plan(instance)?
                        .filter(|p| p.owner.presentation == presentation)
                        .ok_or("SESSION_EXPIRED: 助手窗口已被关闭或替换")?;
                    if plan.ready || !plan.visible || plan.fault.is_some() {
                        break;
                    }
                    if Instant::now() >= deadline {
                        return Err("NATIVE_ERROR: 助手本机界面连接超时".into());
                    }
                    std::thread::sleep(Duration::from_millis(30));
                }
                if plan.fault.is_some() {
                    controller.release(instance, &mut native)?;
                    return Ok(());
                }
                let (area, g) = geometry(app, Some(&window), &plan.config)?;
                let size = (
                    g.width as f64 / area.scale_factor,
                    g.height as f64 / area.scale_factor,
                );
                window
                    .set_size(tauri::PhysicalSize::new(g.width, g.height))
                    .map_err(err)?;
                window
                    .set_always_on_top(plan.config.always_top)
                    .map_err(err)?;
                let current = || self.current(&plan) && !core.quitting.load(Ordering::Acquire);
                if !current() {
                    return Err("SESSION_EXPIRED: 助手窗口计划已失效".into());
                }
                if !plan.visible {
                    if window.is_visible().map_err(err)? {
                        let outside = if plan.config.position == Position::Right {
                            area.left + area.width as i32
                        } else {
                            area.left - g.width as i32
                        };
                        slide(&window, (outside, g.top), current)?;
                        window.hide().map_err(err)?;
                    }
                    controller.reconcile(
                        &core.store.root,
                        &plan.owner,
                        &plan.config,
                        size,
                        false,
                        plan.overlay(),
                        &mut native,
                        current,
                    )?;
                } else {
                    controller.reconcile(
                        &core.store.root,
                        &plan.owner,
                        &plan.config,
                        size,
                        true,
                        plan.overlay(),
                        &mut native,
                        current,
                    )?;
                    if !current() {
                        return Err("SESSION_EXPIRED: 助手窗口计划已失效".into());
                    }
                    if !window.is_visible().map_err(err)? {
                        window.show().map_err(err)?;
                        window.set_focus().map_err(err)?;
                        slide(&window, (g.left, g.top), current)?;
                    } else {
                        window
                            .set_position(tauri::PhysicalPosition::new(g.left, g.top))
                            .map_err(err)?;
                    }
                }
                if !current() {
                    return Err("SESSION_EXPIRED: 助手窗口计划已失效".into());
                }
                let mut book = self.book.lock().map_err(err)?;
                if book.current(&plan) {
                    book.entries.get_mut(instance).unwrap().applied = plan.serial;
                }
                Ok(())
            })();
            if work.as_ref().is_err_and(|e| {
                !e.starts_with("SESSION_EXPIRED:") && !e.starts_with("PRESENTATION_EXPIRED:")
            }) {
                self.book.lock().map_err(err)?.fault(&plan);
                let _ = controller.release(instance, &mut native);
            }
            work
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant_state::{Platform, Protocol, Proxy};
    fn uid() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn fixture() -> (Store, widgets::Sessions, Service) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("artifacts")
            .join(format!("widgetjs-assistant-host-test-{}", uid()));
        (
            Store::open(root).unwrap(),
            widgets::Sessions::default(),
            Service::default(),
        )
    }
    fn add(
        s: &Store,
        sessions: &widgets::Sessions,
        package: &str,
        cap: &str,
    ) -> (String, String, String) {
        let manifest = json!({"schemaVersion":1,"id":package,"name":"网页助手宿主检查","version":"0.1.0","hostApiMajor":1,"entry":"dist/index.html","defaultSize":{"width":500,"height":400},"minSize":{"width":200,"height":160},"capabilities":[cap],"settingsSchema":{"type":"object","additionalProperties":false,"properties":{}}});
        s.db.lock().unwrap().execute("INSERT INTO widget_packages(id,manifest,directory,digest) VALUES(?1,?2,'assistant-test','assistant-test') ON CONFLICT(id) DO UPDATE SET manifest=excluded.manifest",rusqlite::params![package,manifest.to_string()]).unwrap();
        let id = widgets::create_instance(s, package).unwrap();
        let window = format!("float-{id}");
        let session = widgets::open_display_session(s, sessions, &window, &id, None).unwrap();
        (id, window, session["token"].as_str().unwrap().into())
    }
    fn request(rev: u64, c: &Config) -> Value {
        json!({"requestId":uid(),"expectedRevision":rev,"config":c})
    }
    fn call(
        service: &Service,
        s: &Store,
        sessions: &widgets::Sessions,
        row: &(String, String, String),
        method: &str,
        args: Value,
    ) -> AppResult<Value> {
        service
            .access(s, sessions, &row.1, &row.2, method, args)
            .map(|(_, v)| v)
    }
    fn src(config: Config, revision: u64) -> Source {
        let mut document = Document::default();
        document.config = config;
        document.revision = revision;
        Source {
            digest: "test-digest".into(),
            document,
        }
    }
    fn live(b: &mut Book, id: &str, c: Config) -> Surface {
        let p = b.open(id, &src(c, 1), false).unwrap();
        b.bind(&p.owner.shell(), id, "token").unwrap();
        b.ready(&p.owner.shell(), id, "token").unwrap();
        b.entries.get_mut(id).unwrap().focused = true;
        b.entries[id].clone()
    }
    fn bind_page(b: &mut Book, p: &Surface, slot: u8) -> String {
        let tag = uid();
        assert!(b.page(&p.owner, slot, &tag, None));
        assert!(b.page(
            &p.owner,
            slot,
            &tag,
            Some(Navigation::Starting {
                id: 1,
                url: p.config.platforms[slot as usize].url().parse().unwrap()
            })
        ));
        tag
    }
    fn known() -> Config {
        Config {
            tip_progress: 3,
            ..Config::default()
        }
    }
    #[derive(Default)]
    struct FakeShortcuts {
        active: HashSet<u32>,
        registers: usize,
        releases: usize,
        fail: bool,
    }
    impl ShortcutDriver for FakeShortcuts {
        fn register(&mut self, key: tauri_plugin_global_shortcut::Shortcut) -> AppResult<()> {
            self.registers += 1;
            if self.fail || !self.active.insert(key.id()) {
                Err("夹具拒绝登记".into())
            } else {
                Ok(())
            }
        }
        fn unregister(&mut self, key: tauri_plugin_global_shortcut::Shortcut) -> AppResult<()> {
            self.releases += 1;
            self.active.remove(&key.id());
            Ok(())
        }
    }
    #[test]
    fn assistant_host_shortcuts_default_off_and_disabled_or_stale_events_cannot_open() {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, PACKAGE, CAPABILITY);
        let mut io = FakeShortcuts::default();
        service.shortcuts_with(&s, &mut io).unwrap();
        assert_eq!(io.registers, 0);
        assert!(service.toggle(&s, &row.0, None).is_err());
        let c = Config {
            background_enabled: true,
            ..known()
        };
        call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.configure",
            request(0, &c),
        )
        .unwrap();
        service.shortcuts_with(&s, &mut io).unwrap();
        assert_eq!(io.registers, 1);
        let key = c
            .shortcut
            .parse::<tauri_plugin_global_shortcut::Shortcut>()
            .unwrap()
            .id();
        let old = service.shortcut_route(key).unwrap();
        service.toggle(&s, &row.0, Some(&old)).unwrap();
        let mut off = c;
        off.background_enabled = false;
        call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.configure",
            request(1, &off),
        )
        .unwrap();
        assert!(service.toggle(&s, &row.0, Some(&old)).is_err());
        service.shortcuts_with(&s, &mut io).unwrap();
        assert!(service.shortcut_route(key).is_none());
        assert_eq!(io.releases, 1);
        assert!(io.active.is_empty());
    }
    #[test]
    fn assistant_host_shortcut_conflict_is_visible_and_retries_only_after_new_revision() {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, PACKAGE, CAPABILITY);
        let mut io = FakeShortcuts {
            fail: true,
            ..Default::default()
        };
        let mut c = Config {
            background_enabled: true,
            ..known()
        };
        call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.configure",
            request(0, &c),
        )
        .unwrap();
        service.shortcuts_with(&s, &mut io).unwrap();
        service.shortcuts_with(&s, &mut io).unwrap();
        assert_eq!(io.registers, 1);
        let status = call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.status",
            Value::Null,
        )
        .unwrap();
        assert_eq!(status["shortcutRegistered"], false);
        assert!(status["shortcutError"].is_string());
        c.width_ratio = 0.4;
        call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.configure",
            request(1, &c),
        )
        .unwrap();
        io.fail = false;
        service.shortcuts_with(&s, &mut io).unwrap();
        assert_eq!(io.registers, 2);
        assert_eq!(
            call(
                &service,
                &s,
                &sessions,
                &row,
                "assistant.status",
                Value::Null
            )
            .unwrap()["shortcutRegistered"],
            true
        );
        service.stop().unwrap();
        service.shortcuts_with(&s, &mut io).unwrap();
        assert!(io.active.is_empty());
        assert_eq!(io.releases, 1);
    }
    #[test]
    fn assistant_host_same_key_instances_keep_existing_owner_and_never_steal_primary() {
        let (s, sessions, service) = fixture();
        let first = add(&s, &sessions, PACKAGE, CAPABILITY);
        let second = add(&s, &sessions, PACKAGE, CAPABILITY);
        let mut io = FakeShortcuts::default();
        let c = Config {
            background_enabled: true,
            ..known()
        };
        call(
            &service,
            &s,
            &sessions,
            &first,
            "assistant.configure",
            request(0, &c),
        )
        .unwrap();
        service.shortcuts_with(&s, &mut io).unwrap();
        call(
            &service,
            &s,
            &sessions,
            &second,
            "assistant.configure",
            request(0, &c),
        )
        .unwrap();
        service.shortcuts_with(&s, &mut io).unwrap();
        assert_eq!(io.registers, 1);
        let key = c
            .shortcut
            .parse::<tauri_plugin_global_shortcut::Shortcut>()
            .unwrap()
            .id();
        assert_eq!(service.shortcut_route(key).unwrap().instance, first.0);
        let mut conflicting = c;
        conflicting.shortcut = "Ctrl+Alt+Space".into();
        call(
            &service,
            &s,
            &sessions,
            &first,
            "assistant.configure",
            request(1, &conflicting),
        )
        .unwrap();
        service.shortcuts_with(&s, &mut io).unwrap();
        assert_eq!(service.shortcut_route(key).unwrap().instance, second.0);
        assert!(service
            .shortcut_route(
                conflicting
                    .shortcut
                    .parse::<tauri_plugin_global_shortcut::Shortcut>()
                    .unwrap()
                    .id()
            )
            .is_none());
        assert!(service.shortcut_states.lock().unwrap()[&first.0]
            .1
            .error
            .as_ref()
            .unwrap()
            .contains("主面板"));
    }
    #[test]
    fn assistant_host_shell_session_coexists_with_primary_and_accepts_only_current_handshake() {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, PACKAGE, CAPABILITY);
        call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.open",
            json!({"expectedRevision":0}),
        )
        .unwrap();
        let p = service.book.lock().unwrap().entries[&row.0].clone();
        let first =
            widgets::open_display_session(&s, &sessions, &p.owner.shell(), &row.0, None).unwrap();
        let first_token = first["token"].as_str().unwrap();
        assert_eq!(first["role"], "assistant-shell");
        service.bind(&p.owner.shell(), &row.0, first_token).unwrap();
        assert!(sessions.0.lock().unwrap().contains_key(&row.2));
        service
            .access(
                &s,
                &sessions,
                &p.owner.shell(),
                first_token,
                "runtime.ready",
                Value::Null,
            )
            .unwrap();
        let next =
            widgets::open_display_session(&s, &sessions, &p.owner.shell(), &row.0, None).unwrap();
        let next_token = next["token"].as_str().unwrap();
        service.bind(&p.owner.shell(), &row.0, next_token).unwrap();
        assert!(service
            .close_session(&p.owner.shell(), first_token)
            .unwrap()
            .is_none());
        assert!(service
            .access(
                &s,
                &sessions,
                &p.owner.shell(),
                first_token,
                "runtime.ready",
                Value::Null
            )
            .is_err());
        assert!(!service.book.lock().unwrap().entries[&row.0].ready);
        service
            .access(
                &s,
                &sessions,
                &p.owner.shell(),
                next_token,
                "runtime.ready",
                Value::Null,
            )
            .unwrap();
        assert!(service
            .access(
                &s,
                &sessions,
                &p.owner.shell(),
                next_token,
                "storage.get",
                Value::Null
            )
            .is_err());
        assert!(service
            .access(
                &s,
                &sessions,
                &p.owner.shell(),
                next_token,
                "assistant.overlay",
                json!({"expectedRevision":0,"id":"select0","visible":true})
            )
            .is_ok());
    }
    #[test]
    fn assistant_host_delete_revokes_before_transaction_and_does_not_remove_other_profile_data() {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, PACKAGE, CAPABILITY);
        let other = add(&s, &sessions, PACKAGE, CAPABILITY);
        for r in [&row, &other] {
            call(
                &service,
                &s,
                &sessions,
                r,
                "assistant.configure",
                request(0, &known()),
            )
            .unwrap();
            call(
                &service,
                &s,
                &sessions,
                r,
                "assistant.open",
                json!({"expectedRevision":1}),
            )
            .unwrap();
        }
        let old = service.book.lock().unwrap().entries[&row.0].clone();
        let other_plan = service.book.lock().unwrap().entries[&other.0].clone();
        let path = assistant_state::profile_directory(&s.root, &row.0).unwrap();
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("profile-marker"), "不删除登录资料").unwrap();
        widgets::remove_instance_before(&s, &sessions, &row.0, || {
            service.cancel(&row.0)?;
            assert!(!service.current(&old));
            Ok(())
        })
        .unwrap();
        assert!(service.current(&other_plan));
        assert_eq!(assistant_state::status(&s, &row.0).unwrap()["revision"], 0);
        assert_eq!(
            assistant_state::status(&s, &other.0).unwrap()["revision"],
            1
        );
        assert!(path.join("profile-marker").is_file());
        assert!(!sessions.0.lock().unwrap().contains_key(&row.2));
    }
    #[test]
    fn assistant_host_unconfirmed_native_cleanup_is_reported_even_after_display_was_revoked() {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, PACKAGE, CAPABILITY);
        call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.open",
            json!({"expectedRevision":0}),
        )
        .unwrap();
        let value = call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.hide",
            json!({"expectedRevision":0}),
        )
        .unwrap();
        assert!(value["surface"].is_null());
        assert_eq!(value["nativeCleanupPending"], true);
        service
            .book
            .lock()
            .unwrap()
            .native_errors
            .insert(row.0.clone(), "旧窗口释放未确认".into());
        let value = call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.status",
            Value::Null,
        )
        .unwrap();
        assert_eq!(value["nativeCleanupPending"], true);
        assert_eq!(value["nativeError"], "旧窗口释放未确认");
    }
    #[test]
    fn assistant_host_gateway_binds_fixed_package_capability_and_current_session() {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, PACKAGE, CAPABILITY);
        let v = call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.status",
            Value::Null,
        )
        .unwrap();
        assert_eq!(v["config"]["backgroundEnabled"], false);
        assert!(v["surface"].is_null());
        assert!(service
            .access(
                &s,
                &sessions,
                "other-window",
                &row.2,
                "assistant.status",
                Value::Null
            )
            .is_err());
        let other = add(&s, &sessions, "local.test.foreign-assistant", CAPABILITY);
        assert!(call(
            &service,
            &s,
            &sessions,
            &other,
            "assistant.open",
            json!({"expectedRevision":0})
        )
        .unwrap_err()
        .contains("固定组件"));
        let no_cap = add(&s, &sessions, PACKAGE, "storage.instance");
        assert!(call(
            &service,
            &s,
            &sessions,
            &no_cap,
            "assistant.status",
            Value::Null
        )
        .unwrap_err()
        .contains("未声明"));
    }
    #[test]
    fn assistant_host_gateway_rejects_arbitrary_browser_operations_and_foreign_instances() {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, PACKAGE, CAPABILITY);
        for (method, args) in [
            ("assistant.eval", json!({"script":"document.cookie"})),
            ("assistant.navigate", json!({"url":"https://other.test"})),
            (
                "assistant.open",
                json!({"expectedRevision":0,"instance":uid()}),
            ),
            ("assistant.status", json!({"extra":true})),
            (
                "assistant.overlay",
                json!({"id":"settings","visible":true,"expectedRevision":0}),
            ),
            ("runtime.ready", Value::Null),
        ] {
            assert!(
                call(&service, &s, &sessions, &row, method, args).is_err(),
                "{method}"
            );
        }
        assert!(service.book.lock().unwrap().entries.is_empty());
        assert!(!assistant_state::profile_directory(&s.root, &row.0)
            .unwrap()
            .exists());
    }
    #[test]
    fn assistant_host_mutation_failure_and_stale_request_preserve_config_and_surface() {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, PACKAGE, CAPABILITY);
        call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.open",
            json!({"expectedRevision":0}),
        )
        .unwrap();
        let before = service.book.lock().unwrap().entries[&row.0].clone();
        let c = Config {
            background_enabled: true,
            ..Config::default()
        };
        s.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_assistant BEFORE INSERT ON app_settings WHEN NEW.key LIKE 'assistant.config/%' BEGIN SELECT RAISE(ABORT,'拒绝写入'); END;").unwrap();
        assert!(call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.configure",
            request(0, &c)
        )
        .is_err());
        assert!(service.book.lock().unwrap().current(&before));
        s.db.lock()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_assistant;")
            .unwrap();
        let args = request(0, &c);
        call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.configure",
            args.clone(),
        )
        .unwrap();
        let updated = service.book.lock().unwrap().entries[&row.0].clone();
        assert_eq!(updated.owner.revision, 1);
        assert_eq!(
            call(&service, &s, &sessions, &row, "assistant.configure", args).unwrap()["replayed"],
            true
        );
        assert!(service.book.lock().unwrap().current(&updated));
        assert!(call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.hide",
            json!({"expectedRevision":0})
        )
        .is_err());
        assert!(service.book.lock().unwrap().current(&updated));
    }
    #[test]
    fn assistant_host_default_hide_retires_only_target_and_background_hide_retains_page() {
        let mut b = Book::default();
        let a = uid();
        let z = uid();
        let p = live(&mut b, &a, known());
        let other = live(&mut b, &z, known());
        b.hide(&a);
        assert!(!b.entries.contains_key(&a));
        assert_eq!(b.retired[0], p.owner);
        assert!(b.current(&other));
        let mut c = known();
        c.background_enabled = true;
        let p = live(&mut b, &a, c);
        let tag = bind_page(&mut b, &p, 0);
        b.hide(&a);
        assert_eq!(b.entries[&a].pages[0].tag.as_deref(), Some(tag.as_str()));
        assert!(!b.entries[&a].visible);
        assert!(b.page(
            &p.owner,
            0,
            &tag,
            Some(Navigation::Completed {
                id: 1,
                success: true,
                error: 0
            })
        ));
        let p2 = b.open(&a, &src(p.config.clone(), 1), false).unwrap();
        assert_eq!(p.owner, p2.owner);
        assert!(p2.ready);
        let mut off = p.config;
        off.background_enabled = false;
        b.hide(&a);
        b.sync(&a, &src(off, 2)).unwrap();
        assert!(!b.entries.contains_key(&a));
    }
    #[test]
    fn assistant_host_switching_page_preserves_other_tags_but_revokes_old_navigation_and_popup() {
        let mut b = Book::default();
        let id = uid();
        let c = Config {
            page_count: 2,
            ..known()
        };
        let p = live(&mut b, &id, c.clone());
        let t0 = bind_page(&mut b, &p, 0);
        let t1 = bind_page(&mut b, &p, 1);
        let auth = b.reserve_popup(&p.owner, 0, &t0).unwrap();
        let mut changed = c;
        changed.platforms[0] = Platform::Kimi;
        b.sync(&id, &src(changed, 2)).unwrap();
        assert!(!b.auth_current(&auth.label));
        assert_eq!(b.closing_auth, vec![auth.label]);
        assert_eq!(b.entries[&id].pages[1].tag.as_deref(), Some(t1.as_str()));
        assert!(!b.page(
            &p.owner,
            0,
            &t0,
            Some(Navigation::Completed {
                id: 1,
                success: true,
                error: 0
            })
        ));
        assert!(!b.page(
            &p.owner,
            0,
            &t0,
            Some(Navigation::Starting {
                id: 2,
                url: Platform::Chatgpt.url().parse().unwrap()
            })
        ));
        assert!(b.page(
            &p.owner,
            0,
            &t0,
            Some(Navigation::Starting {
                id: 3,
                url: Platform::Kimi.url().parse().unwrap()
            })
        ));
        assert!(b.page(
            &p.owner,
            1,
            &t1,
            Some(Navigation::Completed {
                id: 1,
                success: false,
                error: 7
            })
        ));
    }
    #[test]
    fn assistant_host_nested_login_windows_require_current_opener_and_share_the_four_window_limit()
    {
        let mut b = Book::default();
        let id = uid();
        let p = live(&mut b, &id, known());
        let tag = bind_page(&mut b, &p, 0);
        let first = b.reserve_popup(&p.owner, 0, &tag).unwrap();
        b.entries.get_mut(&id).unwrap().focused = false;
        assert!(b.reserve_popup(&p.owner, 0, &tag).is_err());
        assert!(b
            .reserve_popup_from(&p.owner, 0, &tag, Some("unknown"))
            .is_err());
        let nested = b
            .reserve_popup_from(&p.owner, 0, &tag, Some(&first.label))
            .unwrap();
        assert!(b.auth_current(&nested.label));
        assert!(b
            .reserve_popup_from(&p.owner, 0, &uid(), Some(&first.label))
            .is_err());
        b.reserve_popup_from(&p.owner, 0, &tag, Some(&nested.label))
            .unwrap();
        b.reserve_popup_from(&p.owner, 0, &tag, Some(&first.label))
            .unwrap();
        assert!(b
            .reserve_popup_from(&p.owner, 0, &tag, Some(&nested.label))
            .is_err());
        b.hide(&id);
        assert!(!b.auth_current(&nested.label));
        assert!(b.auth.is_empty());
    }
    #[test]
    fn assistant_host_new_native_view_tag_blocks_old_callbacks_even_when_navigation_ids_repeat() {
        let mut b = Book::default();
        let id = uid();
        let p = live(&mut b, &id, known());
        let old = bind_page(&mut b, &p, 0);
        let auth = b.reserve_popup(&p.owner, 0, &old).unwrap();
        let new = bind_page(&mut b, &p, 0);
        assert_ne!(old, new);
        assert!(!b.auth_current(&auth.label));
        assert!(b.closing_auth.contains(&auth.label));
        assert!(!b.page(
            &p.owner,
            0,
            &old,
            Some(Navigation::Completed {
                id: 1,
                success: false,
                error: 7
            })
        ));
        assert!(b.page(
            &p.owner,
            0,
            &new,
            Some(Navigation::Completed {
                id: 1,
                success: true,
                error: 0
            })
        ));
        assert_eq!(b.entries[&id].pages[0].state.snapshot()["phase"], "ended");
    }
    #[test]
    fn assistant_host_width_top_and_guide_changes_do_not_restart_pages_or_auth() {
        let mut b = Book::default();
        let id = uid();
        let c = known();
        let p = live(&mut b, &id, c.clone());
        let tag = bind_page(&mut b, &p, 0);
        let auth = b.reserve_popup(&p.owner, 0, &tag).unwrap();
        let mut next = c;
        next.width_ratio = 0.4;
        next.always_top = true;
        next.tip_progress = 3;
        b.sync(&id, &src(next, 2)).unwrap();
        assert!(b.auth_current(&auth.label));
        assert!(b.page(
            &p.owner,
            0,
            &tag,
            Some(Navigation::Completed {
                id: 1,
                success: true,
                error: 0
            })
        ));
        b.entries.get_mut(&id).unwrap().focused = false;
        assert!(b.auth_current(&auth.label));
        assert!(b.reserve_popup(&p.owner, 0, &tag).is_err());
    }
    #[test]
    fn assistant_host_proxy_change_retires_auth_and_requires_fresh_native_view_tag() {
        let mut b = Book::default();
        let id = uid();
        let c = known();
        let p = live(&mut b, &id, c.clone());
        let tag = bind_page(&mut b, &p, 0);
        let auth = b.reserve_popup(&p.owner, 0, &tag).unwrap();
        let mut next = c;
        next.proxy = Proxy::Manual {
            protocol: Protocol::Https,
            host: "127.0.0.1".into(),
            port: 7890,
        };
        b.sync(&id, &src(next, 2)).unwrap();
        assert!(!b.auth_current(&auth.label));
        assert!(!b.page(
            &p.owner,
            0,
            &tag,
            Some(Navigation::Starting {
                id: 2,
                url: Platform::Chatgpt.url().parse().unwrap()
            })
        ));
        assert!(!b.page(&p.owner, 0, &uid(), None));
        let new = b.entries[&id].clone();
        assert!(b.page(&new.owner, 0, &uid(), None));
    }
    #[test]
    fn assistant_host_overlay_and_new_shell_session_hide_remote_and_revoke_auth() {
        let mut b = Book::default();
        let id = uid();
        let p = live(&mut b, &id, known());
        let tag = bind_page(&mut b, &p, 0);
        let auth = b.reserve_popup(&p.owner, 0, &tag).unwrap();
        b.overlay(&id, "settings", true).unwrap();
        assert!(b.entries[&id].overlay());
        assert!(!b.auth_current(&auth.label));
        assert!(b.reserve_popup(&p.owner, 0, &tag).is_err());
        b.overlay(&id, "settings", false).unwrap();
        assert!(!b.entries[&id].overlay());
        b.overlay(&id, "select0", true).unwrap();
        b.bind(&p.owner.shell(), &id, "new-token").unwrap();
        assert!(b.entries[&id].overlay());
        assert!(b.entries[&id].overlays.is_empty());
        assert!(b.ready(&p.owner.shell(), &id, "token").is_err());
        b.ready(&p.owner.shell(), &id, "new-token").unwrap();
        assert_eq!(b.entries[&id].pages[0].tag.as_deref(), Some(tag.as_str()));
        assert!(b.overlay(&id, "arbitrary", true).is_err());
    }
    #[test]
    fn assistant_host_retry_digest_change_and_stop_never_reactivate_old_presentation() {
        let mut b = Book::default();
        let id = uid();
        let c = known();
        let old = live(&mut b, &id, c.clone());
        let tag = bind_page(&mut b, &old, 0);
        b.fault(&old);
        assert_eq!(b.entries[&id].snapshot()["phase"], "failed");
        let next = b.open(&id, &src(c.clone(), 1), true).unwrap();
        assert_ne!(next.owner.presentation, old.owner.presentation);
        assert!(!b.page(&old.owner, 0, &tag, None));
        b.fault(&old);
        assert!(b.entries[&id].fault.is_none());
        let mut upgrade = src(c, 1);
        upgrade.digest = "new-digest".into();
        b.sync(&id, &upgrade).unwrap();
        assert!(!b.entries.contains_key(&id));
        let service = Service {
            book: Mutex::new(b),
            ..Service::default()
        };
        service.stop().unwrap();
        assert!(!service.page(&next.owner, 0, &uid(), None));
    }
    #[test]
    fn assistant_host_removing_a_slot_releases_its_selector_overlay_and_preserves_settings_and_other_page(
    ) {
        let mut b = Book::default();
        let id = uid();
        let c = Config {
            page_count: 4,
            ..known()
        };
        let p = live(&mut b, &id, c.clone());
        let tag = bind_page(&mut b, &p, 0);
        b.overlay(&id, "select3", true).unwrap();
        b.overlay(&id, "settings", true).unwrap();
        let mut next = c;
        next.page_count = 1;
        b.sync(&id, &src(next, 2)).unwrap();
        assert!(!b.entries[&id].overlays.contains("select3"));
        assert!(b.entries[&id].overlays.contains("settings"));
        assert_eq!(b.entries[&id].pages[0].tag.as_deref(), Some(tag.as_str()));
        b.overlay(&id, "settings", false).unwrap();
        assert!(!b.entries[&id].overlay());
    }
    #[test]
    fn assistant_host_popup_limit_and_removing_slot_revoke_only_matching_permits() {
        let mut b = Book::default();
        let id = uid();
        let c = Config {
            page_count: 2,
            ..known()
        };
        let p = live(&mut b, &id, c.clone());
        let t0 = bind_page(&mut b, &p, 0);
        let t1 = bind_page(&mut b, &p, 1);
        let a = b.reserve_popup(&p.owner, 0, &t0).unwrap();
        let z = b.reserve_popup(&p.owner, 1, &t1).unwrap();
        b.reserve_popup(&p.owner, 0, &t0).unwrap();
        b.reserve_popup(&p.owner, 0, &t0).unwrap();
        assert!(b.reserve_popup(&p.owner, 0, &t0).is_err());
        let mut next = c;
        next.page_count = 1;
        b.sync(&id, &src(next, 2)).unwrap();
        assert!(b.auth_current(&a.label));
        assert!(!b.auth_current(&z.label));
    }
    #[test]
    fn assistant_host_hidden_deleted_or_revoked_sessions_cannot_create_new_work() {
        let (s, sessions, service) = fixture();
        let row = add(&s, &sessions, PACKAGE, CAPABILITY);
        s.db.lock()
            .unwrap()
            .execute(
                "INSERT INTO app_settings(key,value) VALUES('hidden_all','true') ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [],
            )
            .unwrap();
        assert!(call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.open",
            json!({"expectedRevision":0})
        )
        .is_err());
        s.db.lock()
            .unwrap()
            .execute(
                "UPDATE app_settings SET value='false' WHERE key='hidden_all'",
                [],
            )
            .unwrap();
        sessions.0.lock().unwrap().remove(&row.2);
        assert!(call(
            &service,
            &s,
            &sessions,
            &row,
            "assistant.open",
            json!({"expectedRevision":0})
        )
        .is_err());
        let session = widgets::open_display_session(&s, &sessions, &row.1, &row.0, None).unwrap();
        s.db.lock()
            .unwrap()
            .execute("DELETE FROM widget_instances WHERE id=?1", [&row.0])
            .unwrap();
        assert!(service
            .access(
                &s,
                &sessions,
                &row.1,
                session["token"].as_str().unwrap(),
                "assistant.open",
                json!({"expectedRevision":0})
            )
            .is_err());
        assert!(service.book.lock().unwrap().entries.is_empty());
    }
}
