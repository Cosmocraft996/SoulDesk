//! 番茄专用窗的固定网关；系统动作在鉴权后的原生适配器执行，页面不能指定网址、坐标或句柄。
use crate::{
    pomodoro_window::{self as state, Book, Document, Owner, Preferences},
    store::{err, AppResult, Store},
    widgets,
};
use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

pub(crate) trait Displays: Send + Sync {
    fn area(&self, owner: Option<&Owner>) -> AppResult<crate::desktop_geometry::WorkArea>;
}
pub(crate) struct Source {
    pub digest: String,
    pub document: Document,
    pub visible: bool,
    pub hidden: bool,
}
pub(crate) fn source(store: &Store, instance: &str) -> AppResult<Source> {
    let db = store.db.lock().map_err(err)?;
    let row:Option<(String,String,String,i64)>=db.query_row("SELECT p.id,p.manifest,p.digest,i.visible FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE i.id=?1",[instance],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|"STORAGE_ERROR: 番茄专用窗实例读取失败")?;
    let (package, raw, digest, visible) = row.ok_or("SESSION_EXPIRED: 番茄实例已移除")?;
    let manifest: widgets::Manifest =
        serde_json::from_str(&raw).map_err(|_| "DATA_UNAVAILABLE: 番茄清单无效")?;
    if package != state::PACKAGE
        || manifest.id != state::PACKAGE
        || ![state::CAPABILITY, "focus.pomodoro"]
            .iter()
            .all(|wanted| manifest.capabilities.iter().any(|c| c == wanted))
    {
        return Err("CAPABILITY_DENIED: 只有声明专用窗能力的固定番茄组件可访问".into());
    }
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
    {
        return Err("DATA_UNAVAILABLE: 番茄包版本标识无效".into());
    }
    let hidden: Option<String> = db
        .query_row(
            "SELECT value FROM app_settings WHERE key='hidden_all'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "STORAGE_ERROR: 全局显隐状态读取失败")?;
    if ![0, 1].contains(&visible)
        || hidden
            .as_deref()
            .is_some_and(|v| v != "true" && v != "false")
    {
        return Err("DATA_UNAVAILABLE: 番茄显隐状态无效".into());
    }
    Ok(Source {
        digest,
        document: state::load(&db, instance)?,
        visible: visible == 1,
        hidden: hidden.as_deref() == Some("true"),
    })
}
pub(crate) fn selected_source(store: &Store, instance: &str) -> AppResult<String> {
    crate::shared_library::native_source(
        &*store.db.lock().map_err(err)?,
        instance,
        "focus.pomodoro",
    )
}
fn candidates(store: &Store) -> AppResult<Vec<String>> {
    let db = store.db.lock().map_err(err)?;
    let mut stmt=db.prepare("SELECT i.id FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE p.id=?1 ORDER BY i.id").map_err(err)?;
    let result = stmt
        .query_map([state::PACKAGE], |r| r.get(0))
        .map_err(err)?
        .collect::<Result<Vec<String>, _>>()
        .map_err(err)?;
    Ok(result)
}
pub(crate) struct DragPermit {
    pub owner: Owner,
    pub serial: u64,
}
pub(crate) struct Reply {
    pub value: Value,
    pub drag: Option<DragPermit>,
    pub menu: Option<SceneMenu>,
}
pub(crate) struct SceneMenu {
    pub owner: Owner,
    pub token: String,
    pub source: String,
    pub revision: u64,
    pub scenes: Vec<(String, String, bool)>,
    pub can_select: bool,
}
#[derive(Default)]
pub(crate) struct Service {
    pub book: Mutex<Book>,
    errors: Mutex<std::collections::HashMap<String, String>>,
    stopped: AtomicBool,
}
impl Service {
    pub(crate) fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub(crate) fn current_window(&self, label: &str, instance: &str) -> bool {
        !self.stopped()
            && self.book.lock().is_ok_and(|b| {
                b.by_label(label)
                    .is_some_and(|s| s.owner.instance == instance)
            })
    }
    pub(crate) fn snapshot(&self, store: &Store, instance: &str) -> AppResult<Value> {
        let src = source(store, instance)?;
        let mut value = src.document.snapshot();
        let book = self.book.lock().map_err(err)?;
        value["surface"] = book
            .get(instance)
            .map(|s| s.snapshot())
            .unwrap_or(Value::Null);
        value["nativeCleanupPending"] = json!(book.retired.iter().any(|o| o.instance == instance));
        drop(book);
        value["nativeError"] = json!(self.errors.lock().map_err(err)?.get(instance));
        Ok(value)
    }
    pub(crate) fn bind_session(
        &self,
        store: &Store,
        label: &str,
        instance: &str,
        token: &str,
    ) -> AppResult<()> {
        let _native = store.native_actions.lock().map_err(err)?;
        if self.stopped() {
            return Err("SESSION_EXPIRED: 番茄专用窗正在退出".into());
        }
        let src = source(store, instance)?;
        let selected = selected_source(store, instance)?;
        let mut book = self.book.lock().map_err(err)?;
        let s = book
            .by_label(label)
            .filter(|s| {
                s.owner.instance == instance
                    && s.owner.digest == src.digest
                    && src.document.config.enabled
            })
            .ok_or("PRESENTATION_EXPIRED: 番茄专用窗已失效")?;
        book.bind(&s.owner.label(), token)?;
        book.bind_source(&s.owner.label(), token, selected)
    }
    pub(crate) fn session_closed(&self, label: &str, token: &str) -> AppResult<()> {
        let mut book = self.book.lock().map_err(err)?;
        if book.authorized(label, token).is_ok() {
            book.close_session(label, token)?;
        }
        Ok(())
    }
    pub(crate) fn business(
        &self,
        store: &Store,
        sessions: &widgets::Sessions,
        label: &str,
        token: &str,
        method: &str,
        args: Value,
        now: u64,
    ) -> AppResult<Value> {
        let _native = store.native_actions.lock().map_err(err)?;
        if self.stopped() {
            return Err("SESSION_EXPIRED: 专用窗正在退出".into());
        }
        widgets::with_capability(
            store,
            sessions,
            label,
            token,
            state::CAPABILITY,
            |session| {
                let src = source(store, &session.instance)?;
                let selected = selected_source(store, &session.instance)?;
                let book = self.book.lock().map_err(err)?;
                let s = book.authorized(label, token)?;
                if s.owner.digest != src.digest
                    || !src.document.config.enabled
                    || s.data_source.as_deref() != Some(&selected)
                {
                    return Err("PRESENTATION_EXPIRED: 专用窗或计时来源已改变，请重新加载".into());
                }
                let value = match method {
                    "pomodoro.get" => {
                        empty(&args)?;
                        crate::pomodoro::read(store, &selected, now)
                    }
                    "pomodoro.change" if !src.hidden && src.visible && s.target_visible() => {
                        crate::pomodoro::change(store, &selected, args, now)
                    }
                    _ => Err("CAPABILITY_DENIED: 专用窗只开放当前计时和固定窗口动作".into()),
                }?;
                drop(book);
                crate::shared_library::native_cursor(session, &selected, &value);
                Ok(value)
            },
        )
    }
    pub(crate) fn revoke(&self, instance: &str) -> AppResult<()> {
        self.book.lock().map_err(err)?.retire(instance);
        Ok(())
    }
    pub(crate) fn stop(&self) -> AppResult<()> {
        self.stopped.store(true, Ordering::Release);
        let mut book = self.book.lock().map_err(err)?;
        for s in book.list() {
            book.retire(&s.owner.instance);
        }
        Ok(())
    }
    pub(crate) fn error(&self, instance: &str, error: &str) -> AppResult<()> {
        self.errors
            .lock()
            .map_err(err)?
            .insert(instance.into(), error.chars().take(300).collect());
        Ok(())
    }
    pub(crate) fn clear_error(&self, instance: &str) -> AppResult<()> {
        self.errors.lock().map_err(err)?.remove(instance);
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
        now: u64,
        displays: &impl Displays,
    ) -> AppResult<Reply> {
        if self.stopped() {
            return Err("SESSION_EXPIRED: 番茄专用窗正在退出".into());
        }
        if serde_json::to_vec(&args).map_err(err)?.len() > 8 * 1024 {
            return Err("CAPACITY_EXCEEDED: 专用窗请求过大".into());
        }
        let _native = store.native_actions.lock().map_err(err)?;
        widgets::with_capability(
            store,
            sessions,
            window,
            token,
            state::CAPABILITY,
            |session| {
                let instance = &session.instance;
                let src = source(store, instance)?;
                let surface = if window.starts_with("pomodoro-window-") {
                    let s = self.book.lock().map_err(err)?.authorized(window, token)?;
                    if s.owner.instance != *instance
                        || s.owner.digest != src.digest
                        || !src.document.config.enabled
                    {
                        return Err("PRESENTATION_EXPIRED: 专用窗来源已改变".into());
                    }
                    Some(s)
                } else {
                    None
                };
                let blocked = src.hidden || !src.visible;
                if blocked
                    && !matches!(
                        method,
                        "pomodoro.window.status"
                            | "pomodoro.window.hide"
                            | "pomodoro.window.ready"
                            | "pomodoro.window.configure"
                    )
                {
                    return Err("PRESENTATION_EXPIRED: 隐藏的番茄组件不执行新窗口动作".into());
                }
                let mut drag = None;
                let mut menu = None;
                match method {
                    "pomodoro.window.status" => empty(&args)?,
                    "pomodoro.window.configure" => {
                        #[derive(Deserialize)]
                        #[serde(rename_all = "camelCase", deny_unknown_fields)]
                        struct Input {
                            request_id: String,
                            expected_revision: u64,
                            config: Preferences,
                        }
                        let input: Input = serde_json::from_value(args)
                            .map_err(|_| "INVALID_ARGUMENT: 专用窗设置参数无效")?;
                        if blocked && input.config.enabled {
                            return Err(
                                "PRESENTATION_EXPIRED: 请先显示番茄组件，再启用专用窗".into()
                            );
                        }
                        let (d, replayed) = state::configure(
                            &mut *store.db.lock().map_err(err)?,
                            instance,
                            &input.request_id,
                            input.expected_revision,
                            input.config,
                        )?;
                        if !d.config.enabled {
                            self.revoke(instance)?;
                        }
                        self.clear_error(instance)?;
                        let mut value = self.snapshot(store, instance)?;
                        value["replayed"] = json!(replayed);
                        return Ok(Reply {
                            value,
                            drag: None,
                            menu: None,
                        });
                    }
                    "pomodoro.window.open" => {
                        empty(&args)?;
                        if !src.document.config.enabled {
                            return Err("CONFIG_REQUIRED: 请先显式启用番茄专用窗".into());
                        }
                        let old = self.book.lock().map_err(err)?.get(instance);
                        let area = displays.area(old.as_ref().map(|s| &s.owner))?;
                        self.book.lock().map_err(err)?.open(
                            instance,
                            &src.digest,
                            area,
                            true,
                            now,
                        )?;
                        self.clear_error(instance)?;
                    }
                    "pomodoro.window.hide" => {
                        empty(&args)?;
                        let current = { self.book.lock().map_err(err)?.get(instance) };
                        if let Some(s) = current {
                            self.book.lock().map_err(err)?.hide(&s.owner)?;
                        }
                    }
                    "pomodoro.window.reset" => {
                        empty(&args)?;
                        let s = self
                            .book
                            .lock()
                            .map_err(err)?
                            .get(instance)
                            .ok_or("PRESENTATION_EXPIRED: 专用窗尚未打开")?;
                        let area = displays.area(Some(&s.owner))?;
                        self.book
                            .lock()
                            .map_err(err)?
                            .reposition(&s.owner, area, now)?;
                    }
                    "pomodoro.window.hover" => {
                        #[derive(Deserialize)]
                        #[serde(deny_unknown_fields)]
                        struct Input {
                            inside: bool,
                        }
                        let input: Input = serde_json::from_value(args)
                            .map_err(|_| "INVALID_ARGUMENT: 悬停仅接受inside")?;
                        let s = surface.ok_or("CAPABILITY_DENIED: 只有当前专用窗可回报悬停")?;
                        self.book
                            .lock()
                            .map_err(err)?
                            .hover(&s.owner, false, input.inside, now)?;
                    }
                    "pomodoro.window.ready" => {
                        empty(&args)?;
                        surface.ok_or("CAPABILITY_DENIED: 只有当前专用窗可确认就绪")?;
                        self.book.lock().map_err(err)?.ready(window, token, now)?;
                    }
                    "pomodoro.window.menu" => {
                        empty(&args)?;
                        let s = surface.ok_or("CAPABILITY_DENIED: 只有当前专用窗可打开场景菜单")?;
                        if !s.target_visible() {
                            return Err("PRESENTATION_EXPIRED: 专用窗未显示".into());
                        }
                        let selected = selected_source(store, instance)?;
                        if s.data_source.as_deref() != Some(selected.as_str()) {
                            return Err("PRESENTATION_EXPIRED: 计时来源已改变".into());
                        }
                        let value =
                            crate::pomodoro::read(store, &selected, crate::store::now_ms())?;
                        let can_select =
                            matches!(value["phase"]["status"].as_str(), Some("stop" | "resting"));
                        let scenes = value["scenes"]
                            .as_array()
                            .ok_or("DATA_UNAVAILABLE: 番茄场景无效")?
                            .iter()
                            .map(|scene| {
                                Ok((
                                    scene["id"]
                                        .as_str()
                                        .ok_or("DATA_UNAVAILABLE: 场景标识缺失")?
                                        .to_owned(),
                                    format!(
                                        "{} {}",
                                        scene["icon"].as_str().unwrap_or(""),
                                        scene["name"].as_str().unwrap_or("")
                                    ),
                                    scene["id"] == value["currentSceneId"],
                                ))
                            })
                            .collect::<AppResult<Vec<_>>>()?;
                        menu = Some(SceneMenu {
                            owner: s.owner,
                            token: token.into(),
                            source: selected,
                            revision: value["revision"]
                                .as_u64()
                                .ok_or("DATA_UNAVAILABLE: 番茄版本缺失")?,
                            scenes,
                            can_select,
                        });
                    }
                    "pomodoro.window.drag" => {
                        empty(&args)?;
                        let s = surface.ok_or("CAPABILITY_DENIED: 只有当前专用窗可发起拖动")?;
                        let mut book = self.book.lock().map_err(err)?;
                        book.drag(&s.owner, now)?;
                        let current = book.get(instance).unwrap();
                        drag = Some(DragPermit {
                            owner: current.owner,
                            serial: current.serial,
                        });
                    }
                    _ => {
                        return Err(
                            "CAPABILITY_DENIED: 未知番茄专用窗方法，不接受网址、坐标或原生句柄"
                                .into(),
                        )
                    }
                }
                Ok(Reply {
                    value: self.snapshot(store, instance)?,
                    drag,
                    menu,
                })
            },
        )
    }
    /// 独立计时仍在pomodoro；这里仅读取当前选中源的阶段，不复制或启动计时。
    pub(crate) fn reconcile(
        &self,
        store: &Store,
        displays: &impl Displays,
        wall_now: u64,
        mono_now: u64,
    ) -> AppResult<()> {
        if self.stopped() {
            return Ok(());
        }
        let _native = store.native_actions.lock().map_err(err)?;
        let mut ids = candidates(store)?.into_iter().collect::<HashSet<_>>();
        ids.extend(
            self.book
                .lock()
                .map_err(err)?
                .list()
                .into_iter()
                .map(|s| s.owner.instance),
        );
        for instance in ids {
            let result = (|| -> AppResult<()> {
                let src = source(store, &instance)?;
                if !src.document.config.enabled {
                    self.revoke(&instance)?;
                    return Ok(());
                }
                let old = self.book.lock().map_err(err)?.get(&instance);
                let data_source = selected_source(store, &instance)?;
                if old.as_ref().is_some_and(|s| {
                    s.owner.digest != src.digest
                        || s.data_source.as_ref().is_some_and(|v| v != &data_source)
                }) {
                    self.revoke(&instance)?;
                }
                let suspended = src.hidden || !src.visible;
                let missing = self.book.lock().map_err(err)?.get(&instance).is_none();
                if missing && !suspended {
                    if self
                        .book
                        .lock()
                        .map_err(err)?
                        .retired
                        .iter()
                        .any(|o| o.instance == instance)
                    {
                        return Ok(());
                    }
                    // 失败保持可解释，等待用户重新打开，不按后台时钟连续重建窗口。
                    if self.errors.lock().map_err(err)?.contains_key(&instance) {
                        return Ok(());
                    }
                    let area = displays.area(None)?;
                    self.book.lock().map_err(err)?.open(
                        &instance,
                        &src.digest,
                        area,
                        false,
                        mono_now,
                    )?;
                }
                let value = crate::pomodoro::read(store, &data_source, wall_now)?;
                let phase = value["phase"]["status"]
                    .as_str()
                    .ok_or("DATA_UNAVAILABLE: 计时阶段缺失")?;
                let mut book = self.book.lock().map_err(err)?;
                book.suspend(&instance, suspended, mono_now)?;
                book.phase(&instance, phase, mono_now)?;
                Ok(())
            })();
            if let Err(error) = result {
                self.revoke(&instance)?;
                if !error.starts_with("CAPABILITY_DENIED:")
                    && !error.starts_with("SESSION_EXPIRED:")
                {
                    self.error(&instance, &error)?;
                }
            }
        }
        self.book.lock().map_err(err)?.tick(mono_now)?;
        Ok(())
    }
    pub(crate) fn drag_current(&self, store: &Store, permit: &DragPermit) -> AppResult<bool> {
        if self.stopped() {
            return Ok(false);
        }
        let src = source(store, &permit.owner.instance)?;
        Ok(src.document.config.enabled
            && src.visible
            && !src.hidden
            && src.digest == permit.owner.digest
            && self
                .book
                .lock()
                .map_err(err)?
                .get(&permit.owner.instance)
                .is_some_and(|s| {
                    s.owner == permit.owner
                        && s.serial == permit.serial
                        && s.dragging
                        && s.target_visible()
                }))
    }
}
pub(crate) fn select_scene(
    service: &Service,
    store: &Store,
    sessions: &widgets::Sessions,
    menu: &SceneMenu,
    scene: &str,
    request: &str,
    now: u64,
) -> AppResult<Value> {
    let _native = store.native_actions.lock().map_err(err)?;
    if service.stopped() {
        return Err("SESSION_EXPIRED: 番茄专用窗正在退出".into());
    }
    widgets::with_capability(
        store,
        sessions,
        &menu.owner.label(),
        &menu.token,
        state::CAPABILITY,
        |session| {
            let src = source(store, &session.instance)?;
            let s = service
                .book
                .lock()
                .map_err(err)?
                .authorized(&menu.owner.label(), &menu.token)?;
            if s.owner != menu.owner
                || s.owner.digest != src.digest
                || !src.document.config.enabled
                || src.hidden
                || !src.visible
                || !s.target_visible()
                || selected_source(store, &session.instance)? != menu.source
            {
                return Err("PRESENTATION_EXPIRED: 菜单已不属于当前窗口或数据源".into());
            }
            if !menu.can_select || !menu.scenes.iter().any(|v| v.0 == scene) {
                return Err("INVALID_ARGUMENT: 当前菜单不能选择此场景".into());
            }
            let value = crate::pomodoro::change(
                store,
                &menu.source,
                json!({"requestId":request,"expectedRevision":menu.revision,"operation":"select","id":scene}),
                now,
            )?;
            crate::shared_library::native_cursor(session, &menu.source, &value);
            Ok(value)
        },
    )
}
fn empty(value: &Value) -> AppResult<()> {
    if value.is_null() || value.as_object().is_some_and(|v| v.is_empty()) {
        Ok(())
    } else {
        Err("INVALID_ARGUMENT: 此窗口动作不接受附加参数".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::sync::atomic::AtomicUsize;
    fn id() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    struct FakeDisplays {
        calls: AtomicUsize,
        fail: bool,
    }
    impl Default for FakeDisplays {
        fn default() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                fail: false,
            }
        }
    }
    impl Displays for FakeDisplays {
        fn area(&self, _: Option<&Owner>) -> AppResult<crate::desktop_geometry::WorkArea> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.fail {
                return Err("DISPLAY_UNAVAILABLE: 离线显示器失败".into());
            }
            Ok(crate::desktop_geometry::WorkArea {
                left: 0,
                top: 0,
                width: 1920,
                height: 1040,
                scale_factor: 1.,
            })
        }
    }
    struct Fixture {
        store: Store,
        service: Service,
        sessions: widgets::Sessions,
        instance: String,
        window: String,
        token: String,
        display: FakeDisplays,
    }
    fn fixture() -> Fixture {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("artifacts")
            .join(format!("desktop-integration-pomodoro-window-host-{}", id()));
        let store = Store::open(root).unwrap();
        let manifest = json!({"schemaVersion":1,"id":state::PACKAGE,"name":"番茄专用窗离线检查","version":"0.3.0","hostApiMajor":1,"entry":"dist/index.html","defaultSize":{"width":150,"height":150},"minSize":{"width":150,"height":150},"capabilities":["storage.instance","storage.shared","focus.pomodoro",state::CAPABILITY],"settingsSchema":{"type":"object","additionalProperties":false,"properties":{}}});
        store.db.lock().unwrap().execute("INSERT INTO widget_packages(id,manifest,directory,digest)VALUES(?1,?2,'pomodoro-window-fixture',?3)",params![state::PACKAGE,manifest.to_string(),"a".repeat(64)]).unwrap();
        let instance = widgets::create_instance(&store, state::PACKAGE).unwrap();
        let window = format!("float-{instance}");
        let sessions = widgets::Sessions::default();
        let token = widgets::open_display_session(&store, &sessions, &window, &instance, None)
            .unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        Fixture {
            store,
            service: Service::default(),
            sessions,
            instance,
            window,
            token,
            display: FakeDisplays::default(),
        }
    }
    impl Fixture {
        fn call(&self, method: &str, args: Value) -> AppResult<Reply> {
            self.service.access(
                &self.store,
                &self.sessions,
                &self.window,
                &self.token,
                method,
                args,
                0,
                &self.display,
            )
        }
        fn enable(&self) {
            self.call(
                "pomodoro.window.configure",
                json!({"requestId":id(),"expectedRevision":0,"config":{"enabled":true}}),
            )
            .unwrap();
        }
        fn open(&self) -> state::Surface {
            self.call("pomodoro.window.open", Value::Null).unwrap();
            self.service
                .book
                .lock()
                .unwrap()
                .get(&self.instance)
                .unwrap()
        }
    }
    #[test]
    fn default_permission_never_reads_a_display_or_opens_a_window() {
        let f = fixture();
        f.call("pomodoro.window.status", Value::Null).unwrap();
        f.service.reconcile(&f.store, &f.display, 100, 0).unwrap();
        assert_eq!(f.display.calls.load(Ordering::Relaxed), 0);
        assert!(f.service.book.lock().unwrap().list().is_empty());
        assert!(f.call("pomodoro.window.open", Value::Null).is_err());
        assert_eq!(f.display.calls.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn enabling_saves_before_native_open_and_a_failed_save_keeps_the_old_surface() {
        let f = fixture();
        f.enable();
        assert_eq!(f.display.calls.load(Ordering::Relaxed), 0);
        let s = f.open();
        f.store.db.lock().unwrap().execute_batch("CREATE TRIGGER refuse_window_update BEFORE UPDATE ON app_settings WHEN new.key LIKE 'pomodoro.window/%' BEGIN SELECT RAISE(FAIL,'denied');END;").unwrap();
        assert!(f
            .call(
                "pomodoro.window.configure",
                json!({"requestId":id(),"expectedRevision":1,"config":{"enabled":false}})
            )
            .is_err());
        assert!(f.service.book.lock().unwrap().current(&s.owner));
        assert!(
            source(&f.store, &f.instance)
                .unwrap()
                .document
                .config
                .enabled
        );
    }
    #[test]
    fn disabled_hidden_or_invalid_sources_cannot_start_any_native_operation() {
        let f = fixture();
        f.enable();
        f.store
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE widget_instances SET visible=0 WHERE id=?1",
                [&f.instance],
            )
            .unwrap();
        assert!(f.call("pomodoro.window.open", Value::Null).is_err());
        f.service.reconcile(&f.store, &f.display, 100, 0).unwrap();
        assert_eq!(f.display.calls.load(Ordering::Relaxed), 0);
        f.store
            .db
            .lock()
            .unwrap()
            .execute("UPDATE widget_packages SET digest='broken'", [])
            .unwrap();
        assert!(f.call("pomodoro.window.status", Value::Null).is_err());
        assert_eq!(f.display.calls.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn cross_window_or_arbitrary_code_and_bounds_cannot_reach_a_display() {
        let f = fixture();
        f.enable();
        assert!(f
            .service
            .access(
                &f.store,
                &f.sessions,
                "other",
                &f.token,
                "pomodoro.window.open",
                Value::Null,
                0,
                &f.display
            )
            .is_err());
        for (method, args) in [
            ("pomodoro.window.open", json!({"url":"https://example.com"})),
            ("pomodoro.window.setBounds", json!({"x":1,"y":1})),
            (
                "pomodoro.window.executeJavaScript",
                json!({"script":"不允许"}),
            ),
            ("pomodoro.window.drag", Value::Null),
        ] {
            assert!(f.call(method, args).is_err());
        }
        assert_eq!(f.display.calls.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn only_the_current_bound_window_can_hover_ready_or_drag_and_old_tokens_are_revoked() {
        let f = fixture();
        f.enable();
        let s = f.open();
        let token = id();
        f.service
            .bind_session(&f.store, &s.owner.label(), &f.instance, &token)
            .unwrap();
        f.sessions.0.lock().unwrap().insert(
            token.clone(),
            widgets::Session {
                instance: f.instance.clone(),
                package_digest: None,
                window: s.owner.label(),
                preview: None,
                shared_cursor: None,
            },
        );
        let ready = f
            .service
            .access(
                &f.store,
                &f.sessions,
                &s.owner.label(),
                &token,
                "pomodoro.window.ready",
                Value::Null,
                100,
                &f.display,
            )
            .unwrap();
        assert_eq!(ready.value["surface"]["ready"], true);
        let reply = f
            .service
            .access(
                &f.store,
                &f.sessions,
                &s.owner.label(),
                &token,
                "pomodoro.window.drag",
                Value::Null,
                100,
                &f.display,
            )
            .unwrap();
        let permit = reply.drag.unwrap();
        assert!(f.service.drag_current(&f.store, &permit).unwrap());
        let newer = id();
        f.service
            .bind_session(&f.store, &s.owner.label(), &f.instance, &newer)
            .unwrap();
        f.service.session_closed(&s.owner.label(), &token).unwrap();
        assert!(f
            .service
            .access(
                &f.store,
                &f.sessions,
                &s.owner.label(),
                &token,
                "pomodoro.window.hover",
                json!({"inside":true}),
                200,
                &f.display
            )
            .is_err());
        assert!(!f.service.drag_current(&f.store, &permit).unwrap());
    }
    #[test]
    fn disabling_retires_only_its_window_and_does_not_change_the_pomodoro_document() {
        let f = fixture();
        f.enable();
        let s = f.open();
        let before = crate::pomodoro::read(&f.store, &f.instance, 100).unwrap();
        f.call(
            "pomodoro.window.configure",
            json!({"requestId":id(),"expectedRevision":1,"config":{"enabled":false}}),
        )
        .unwrap();
        assert!(!f.service.book.lock().unwrap().current(&s.owner));
        assert_eq!(
            f.call("pomodoro.window.status", Value::Null).unwrap().value["nativeCleanupPending"],
            true
        );
        assert_eq!(
            crate::pomodoro::read(&f.store, &f.instance, 100).unwrap(),
            before
        );
    }
    #[test]
    fn display_failures_keep_permission_but_never_claim_a_native_window_exists() {
        let f = fixture();
        f.enable();
        let bad = FakeDisplays {
            calls: AtomicUsize::new(0),
            fail: true,
        };
        f.service.reconcile(&f.store, &bad, 100, 0).unwrap();
        f.service.reconcile(&f.store, &bad, 100, 1).unwrap();
        assert_eq!(bad.calls.load(Ordering::Relaxed), 1);
        let value = f.call("pomodoro.window.status", Value::Null).unwrap().value;
        assert_eq!(value["config"]["enabled"], true);
        assert!(value["surface"].is_null());
        assert!(value["nativeError"]
            .as_str()
            .unwrap()
            .contains("DISPLAY_UNAVAILABLE"));
    }
    #[test]
    fn corrupt_permissions_and_stopping_revoke_memory_even_when_persistence_is_unavailable() {
        let f = fixture();
        f.enable();
        let s = f.open();
        f.store
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE app_settings SET value='broken' WHERE key=?1",
                [format!("pomodoro.window/{}", f.instance)],
            )
            .unwrap();
        f.service.reconcile(&f.store, &f.display, 100, 0).unwrap();
        assert!(!f.service.book.lock().unwrap().current(&s.owner));
        f.service.stop().unwrap();
        assert!(f.call("pomodoro.window.status", Value::Null).is_err());
        assert!(f.service.book.lock().unwrap().list().is_empty());
    }
    #[test]
    fn an_explicit_hide_returns_without_nested_book_locks_and_keeps_the_timer() {
        let f = fixture();
        f.enable();
        f.open();
        let result = f.call("pomodoro.window.hide", Value::Null).unwrap().value;
        assert_eq!(result["surface"]["hidden"], true);
        assert_eq!(result["config"]["enabled"], true);
    }
    #[test]
    fn selected_shared_timer_is_read_without_restarting_or_writing_the_private_timer() {
        let f = fixture();
        f.enable();
        let view = f.open();
        let at = crate::store::now_ms();
        let initial = crate::pomodoro::authorized(&f.store, &f.sessions, &f.window, &f.token, "pomodoro.change", json!({"requestId":id(),"expectedRevision":0,"operation":"initialize","templates":["reading"]}), at).unwrap();
        crate::pomodoro::authorized(
            &f.store,
            &f.sessions,
            &f.window,
            &f.token,
            "pomodoro.change",
            json!({"requestId":id(),"expectedRevision":initial["revision"],"operation":"start"}),
            at,
        )
        .unwrap();
        widgets::gateway(
            &f.store,
            &f.sessions,
            &f.window,
            &f.token,
            "library.create",
            json!({"name":"专用窗共用计时"}),
        )
        .unwrap();
        let selected = selected_source(&f.store, &f.instance).unwrap();
        assert_ne!(selected, f.instance);
        // 私有源此时已按确认规则暂停，业务接口必须拒绝读取；只在测试中核对原始行未变。
        use base64::Engine;
        let private_key = format!(
            "pomodoro.document:{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&f.instance)
        );
        let private_value = || {
            f.store
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT value FROM app_settings WHERE key=?1",
                    [&private_key],
                    |r| r.get::<_, String>(0),
                )
                .unwrap()
        };
        let private_before = private_value();
        assert!(crate::pomodoro::read(&f.store, &f.instance, at).is_err());
        let shared_before = crate::pomodoro::read(&f.store, &selected, at).unwrap();
        assert_eq!(shared_before["phase"]["status"], "stop");
        f.service.reconcile(&f.store, &f.display, at, 100).unwrap();
        assert_eq!(private_value(), private_before);
        assert_eq!(
            crate::pomodoro::read(&f.store, &selected, at).unwrap(),
            shared_before
        );
        assert!(f.service.book.lock().unwrap().current(&view.owner));
        assert!(
            source(&f.store, &f.instance)
                .unwrap()
                .document
                .config
                .enabled
        );
    }
    #[test]
    fn broken_shared_data_cannot_prevent_disabling_the_separate_window_permission() {
        let f = fixture();
        f.enable();
        f.open();
        widgets::gateway(
            &f.store,
            &f.sessions,
            &f.window,
            &f.token,
            "library.create",
            json!({"name":"故障隔离库"}),
        )
        .unwrap();
        f.store
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE app_settings SET value='broken' WHERE key LIKE 'shared-library:%'",
                [],
            )
            .unwrap();
        assert!(selected_source(&f.store, &f.instance).is_err());
        let token =
            widgets::open_display_session(&f.store, &f.sessions, &f.window, &f.instance, None)
                .unwrap()["token"]
                .as_str()
                .unwrap()
                .to_owned();
        let result = f
            .service
            .access(
                &f.store,
                &f.sessions,
                &f.window,
                &token,
                "pomodoro.window.configure",
                json!({"requestId":id(),"expectedRevision":1,"config":{"enabled":false}}),
                0,
                &f.display,
            )
            .unwrap();
        assert_eq!(result.value["config"]["enabled"], false);
        assert!(f.service.book.lock().unwrap().get(&f.instance).is_none());
    }
    fn native_session(f: &Fixture) -> (state::Surface, String) {
        f.enable();
        let s = f.open();
        let value = widgets::open_display_session(
            &f.store,
            &f.sessions,
            &s.owner.label(),
            &f.instance,
            None,
        )
        .unwrap();
        assert_eq!(value["role"], "pomodoro-window");
        let token = value["token"].as_str().unwrap().to_owned();
        f.service
            .bind_session(&f.store, &s.owner.label(), &f.instance, &token)
            .unwrap();
        f.service
            .book
            .lock()
            .unwrap()
            .ready(&s.owner.label(), &token, 0)
            .unwrap();
        (s, token)
    }
    fn native_change(
        f: &Fixture,
        s: &state::Surface,
        token: &str,
        operation: &str,
        extra: Value,
    ) -> Value {
        let d = f
            .service
            .business(
                &f.store,
                &f.sessions,
                &s.owner.label(),
                token,
                "pomodoro.get",
                Value::Null,
                crate::store::now_ms(),
            )
            .unwrap();
        let mut args =
            json!({"requestId":id(),"expectedRevision":d["revision"],"operation":operation});
        for (k, v) in extra.as_object().unwrap() {
            args[k] = v.clone();
        }
        f.service
            .business(
                &f.store,
                &f.sessions,
                &s.owner.label(),
                token,
                "pomodoro.change",
                args,
                crate::store::now_ms(),
            )
            .unwrap()
    }
    fn native_menu(f: &Fixture, s: &state::Surface, token: &str) -> SceneMenu {
        f.service
            .access(
                &f.store,
                &f.sessions,
                &s.owner.label(),
                token,
                "pomodoro.window.menu",
                Value::Null,
                0,
                &f.display,
            )
            .unwrap()
            .menu
            .unwrap()
    }
    #[test]
    fn scene_menu_binds_source_revision_and_uuid_replay_without_restarting_timer() {
        let f = fixture();
        let (s, token) = native_session(&f);
        let initial = native_change(
            &f,
            &s,
            &token,
            "initialize",
            json!({"templates":["reading","coding"]}),
        );
        let menu = native_menu(&f, &s, &token);
        let target = initial["scenes"][1]["id"].as_str().unwrap();
        let request = id();
        let a = select_scene(
            &f.service,
            &f.store,
            &f.sessions,
            &menu,
            target,
            &request,
            crate::store::now_ms(),
        )
        .unwrap();
        let b = select_scene(
            &f.service,
            &f.store,
            &f.sessions,
            &menu,
            target,
            &request,
            crate::store::now_ms(),
        )
        .unwrap();
        assert_eq!(a["currentSceneId"], target);
        assert_eq!(a["phase"]["status"], "stop");
        assert_eq!(a["revision"], b["revision"]);
        assert_eq!(a["history"], b["history"]);
        assert_eq!(a["phase"], b["phase"]);
        assert!(select_scene(
            &f.service,
            &f.store,
            &f.sessions,
            &menu,
            target,
            &id(),
            crate::store::now_ms()
        )
        .unwrap_err()
        .starts_with("REVISION_CONFLICT:"));
        assert!(f.sessions.0.lock().unwrap().contains_key(&f.token));
    }
    #[test]
    fn menu_and_clock_from_old_source_are_rejected_before_native_reconciliation() {
        let f = fixture();
        let (s, token) = native_session(&f);
        let initial = native_change(
            &f,
            &s,
            &token,
            "initialize",
            json!({"templates":["reading","coding"]}),
        );
        let menu = native_menu(&f, &s, &token);
        widgets::gateway(
            &f.store,
            &f.sessions,
            &f.window,
            &f.token,
            "library.create",
            json!({"name":"切源菜单"}),
        )
        .unwrap();
        let error = f
            .service
            .business(
                &f.store,
                &f.sessions,
                &s.owner.label(),
                &token,
                "pomodoro.get",
                Value::Null,
                crate::store::now_ms(),
            )
            .unwrap_err();
        assert!(
            error.starts_with("PRESENTATION_EXPIRED:") || error.starts_with("SESSION_EXPIRED:")
        );
        assert!(select_scene(
            &f.service,
            &f.store,
            &f.sessions,
            &menu,
            initial["scenes"][1]["id"].as_str().unwrap(),
            &id(),
            crate::store::now_ms()
        )
        .is_err());
        f.service
            .reconcile(&f.store, &f.display, crate::store::now_ms(), 100)
            .unwrap();
        assert!(!f.service.book.lock().unwrap().current(&s.owner));
        assert_eq!(
            f.service.snapshot(&f.store, &f.instance).unwrap()["nativeError"],
            Value::Null
        );
    }
    #[test]
    fn native_reload_revokes_old_menu_and_hidden_window_cannot_change_timer() {
        let f = fixture();
        let (s, token) = native_session(&f);
        let initial = native_change(
            &f,
            &s,
            &token,
            "initialize",
            json!({"templates":["reading"]}),
        );
        let menu = native_menu(&f, &s, &token);
        let next = widgets::open_display_session(
            &f.store,
            &f.sessions,
            &s.owner.label(),
            &f.instance,
            None,
        )
        .unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        f.service
            .bind_session(&f.store, &s.owner.label(), &f.instance, &next)
            .unwrap();
        f.service.session_closed(&s.owner.label(), &token).unwrap();
        assert!(select_scene(
            &f.service,
            &f.store,
            &f.sessions,
            &menu,
            initial["scenes"][0]["id"].as_str().unwrap(),
            &id(),
            crate::store::now_ms()
        )
        .is_err());
        f.service
            .book
            .lock()
            .unwrap()
            .ready(&s.owner.label(), &next, 0)
            .unwrap();
        f.service.book.lock().unwrap().hide(&s.owner).unwrap();
        assert!(f.service.business(&f.store,&f.sessions,&s.owner.label(),&next,"pomodoro.change",json!({"requestId":id(),"expectedRevision":initial["revision"],"operation":"start"}),crate::store::now_ms()).is_err());
        assert_eq!(
            f.service
                .business(
                    &f.store,
                    &f.sessions,
                    &s.owner.label(),
                    &next,
                    "pomodoro.get",
                    Value::Null,
                    crate::store::now_ms()
                )
                .unwrap()["phase"]["status"],
            "stop"
        );
    }
    #[test]
    fn active_native_scene_menu_cannot_switch_scene() {
        let f = fixture();
        let (s, token) = native_session(&f);
        native_change(
            &f,
            &s,
            &token,
            "initialize",
            json!({"templates":["reading","coding"]}),
        );
        let running = native_change(&f, &s, &token, "start", json!({}));
        let menu = native_menu(&f, &s, &token);
        assert!(!menu.can_select);
        assert!(select_scene(
            &f.service,
            &f.store,
            &f.sessions,
            &menu,
            running["scenes"][1]["id"].as_str().unwrap(),
            &id(),
            crate::store::now_ms()
        )
        .is_err());
        assert_eq!(
            crate::pomodoro::read(&f.store, &f.instance, crate::store::now_ms()).unwrap()
                ["currentSceneId"],
            running["currentSceneId"]
        );
    }
}
