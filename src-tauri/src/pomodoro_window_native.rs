//! 番茄钟专用窗、托盘与系统事件。页面只能发固定动作，计时仍归原后台。
use crate::{
    desktop_geometry::WorkArea,
    pomodoro_window::{self as state, Bounds, Owner, Surface},
    pomodoro_window_host::{self as host, Displays, SceneMenu, Service},
    store::{err, now_ms, AppResult},
    windows, Core,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    webview::NewWindowResponse,
    AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use uuid::Uuid;

struct Tray {
    _icon: TrayIcon,
    hidden: CheckMenuItem<tauri::Wry>,
}
struct Ticket {
    menu: SceneMenu,
    at: u64,
}
#[derive(Default)]
struct Runtime {
    trays: HashMap<String, Tray>,
    menus: HashMap<String, (Menu<tauri::Wry>, Ticket)>,
    destroying: HashSet<String>,
    focus_attempts: HashMap<String, u64>,
    drag_started: HashMap<String, u64>,
    moved: HashMap<String, u64>,
    checked: HashMap<String, u64>,
    emitted: HashMap<String, Value>,
}
pub(crate) struct Coordinator {
    pub service: Service,
    runtime: Mutex<Runtime>,
    driving: AtomicBool,
    reconcile: AtomicBool,
    wake: tokio::sync::Notify,
    started: Instant,
}
impl Default for Coordinator {
    fn default() -> Self {
        Self {
            service: Service::default(),
            runtime: Mutex::new(Runtime::default()),
            driving: AtomicBool::new(false),
            reconcile: AtomicBool::new(true),
            wake: tokio::sync::Notify::new(),
            started: Instant::now(),
        }
    }
}

/// 实际显示回执只从驱动读取；发出show/setPosition不等于系统已经应用。
#[derive(Clone, Copy)]
struct Observed {
    bounds: Bounds,
    visible: bool,
}
trait Driver {
    fn exists(&self, owner: &Owner) -> bool;
    fn create(&mut self, s: &Surface) -> AppResult<()>;
    fn place(&mut self, s: &Surface, at: Bounds) -> AppResult<Observed>;
    fn conceal(&mut self, owner: &Owner) -> AppResult<()>;
    fn destroy(&mut self, owner: &Owner) -> AppResult<bool>;
}
fn retire(service: &Service, owner: &Owner) -> AppResult<()> {
    let mut book = service.book.lock().map_err(err)?;
    if book.current(owner) {
        book.retire(&owner.instance);
    }
    Ok(())
}
fn release(service: &Service, driver: &mut impl Driver) -> AppResult<()> {
    let retired = { service.book.lock().map_err(err)?.retired.clone() };
    for owner in retired {
        match driver.destroy(&owner) {
            Ok(true) => service.book.lock().map_err(err)?.retired_confirmed(&owner),
            Ok(false) => {}
            Err(error) => {
                service.error(&owner.instance, &error)?;
            }
        }
    }
    Ok(())
}
fn render_surface(
    service: &Service,
    driver: &mut impl Driver,
    s: &Surface,
    now: u64,
    current: impl Fn() -> bool,
) -> AppResult<()> {
    let live = || {
        current() && !service.stopped() && service.book.lock().is_ok_and(|b| b.current(&s.owner))
    };
    if !live() {
        let result = driver.conceal(&s.owner);
        retire(service, &s.owner)?;
        return result;
    }
    if !driver.exists(&s.owner) && (s.manual_hidden || s.suspended || s.fault.is_some()) {
        return Ok(());
    }
    let result: AppResult<()> = (|| {
        driver.create(s)?;
        if !live() {
            driver.conceal(&s.owner)?;
            retire(service, &s.owner)?;
            return Ok(());
        }
        let observed = driver.place(s, s.frame(now))?;
        // 系统回调可在原生调用中撤销许可；旧show返回后立即隐藏，不能把它记成成功。
        let same = service.book.lock().is_ok_and(|b| {
            b.get(&s.owner.instance)
                .is_some_and(|v| v.owner == s.owner && v.serial == s.serial)
        });
        if !live() || !same {
            driver.conceal(&s.owner)?;
            if !live() {
                retire(service, &s.owner)?;
            }
            return Ok(());
        }
        service.book.lock().map_err(err)?.confirmed(
            &s.owner,
            s.serial,
            observed.bounds,
            observed.visible,
        );
        Ok(())
    })();
    if let Err(error) = &result {
        let _ = driver.conceal(&s.owner);
        retire(service, &s.owner)?;
        service.error(&s.owner.instance, error)?;
    }
    result
}
fn monitor_area(m: &tauri::Monitor) -> WorkArea {
    let a = m.work_area();
    WorkArea {
        left: a.position.x,
        top: a.position.y,
        width: a.size.width,
        height: a.size.height,
        scale_factor: m.scale_factor(),
    }
}
fn choose_area(areas: &[WorkArea], at: Bounds) -> AppResult<WorkArea> {
    let x = f64::from(at.x) + f64::from(at.width) / 2.;
    let y = f64::from(at.y) + f64::from(at.height) / 2.;
    areas
        .iter()
        .copied()
        .min_by(|a, b| {
            let distance = |a: &WorkArea| {
                let dx = (f64::from(a.left) - x)
                    .max(0.)
                    .max(x - (f64::from(a.left) + f64::from(a.width)));
                let dy = (f64::from(a.top) - y)
                    .max(0.)
                    .max(y - (f64::from(a.top) + f64::from(a.height)));
                dx * dx + dy * dy
            };
            distance(a).total_cmp(&distance(b))
        })
        .ok_or_else(|| "DISPLAY_UNAVAILABLE: 没有可用工作区".into())
}
struct SystemDisplays<'a> {
    app: &'a AppHandle,
    service: &'a Service,
}
impl Displays for SystemDisplays<'_> {
    fn area(&self, owner: Option<&Owner>) -> AppResult<WorkArea> {
        if let Some(owner) = owner {
            let anchor = self
                .service
                .book
                .lock()
                .map_err(err)?
                .get(&owner.instance)
                .filter(|s| s.owner == *owner)
                .ok_or("PRESENTATION_EXPIRED: 专用窗已替换")?
                .anchor();
            choose_area(
                &self
                    .app
                    .available_monitors()
                    .map_err(err)?
                    .iter()
                    .map(monitor_area)
                    .collect::<Vec<_>>(),
                anchor,
            )
        } else {
            self.app
                .primary_monitor()
                .map_err(err)?
                .map(|m| monitor_area(&m))
                .ok_or_else(|| "DISPLAY_UNAVAILABLE: 主工作区不可用".into())
        }
    }
}
fn observe(w: &WebviewWindow) -> AppResult<Observed> {
    let p = w.outer_position().map_err(err)?;
    let size = w.inner_size().map_err(err)?;
    Ok(Observed {
        bounds: Bounds {
            x: p.x,
            y: p.y,
            width: size.width,
            height: size.height,
        },
        visible: w.is_visible().map_err(err)?,
    })
}
fn visibility(w: &WebviewWindow, visible: bool) -> AppResult<()> {
    // Tao初始focused=false标记在创建后被移除；后续普通show会使用SW_SHOW并激活窗口。
    #[cfg(windows)]
    {
        if visible {
            crate::desktop_native::show(w)
        } else {
            crate::desktop_native::hide(w)
        }
    }
    #[cfg(not(windows))]
    {
        if visible {
            w.show().map_err(err)
        } else {
            w.hide().map_err(err)
        }
    }
}
fn geometry(w: &WebviewWindow, at: Bounds) -> AppResult<()> {
    // 与不激活显隐使用同一原生适配，避免Tao的旧VISIBLE标记在移动时重新隐藏窗口。
    #[cfg(windows)]
    {
        crate::desktop_native::place(w, at.x, at.y, at.width, at.height, false)
    }
    #[cfg(not(windows))]
    {
        w.set_size(tauri::PhysicalSize::new(at.width, at.height))
            .map_err(err)?;
        w.set_position(tauri::PhysicalPosition::new(at.x, at.y))
            .map_err(err)
    }
}
fn local(app: &AppHandle, url: &url::Url) -> bool {
    matches!(
        url.origin().ascii_serialization().as_str(),
        "http://tauri.localhost" | "https://tauri.localhost"
    ) || url.scheme() == "tauri" && url.host_str() == Some("localhost")
        || app
            .config()
            .build
            .dev_url
            .as_ref()
            .is_some_and(|d| d.origin() == url.origin())
}
fn id(owner: &Owner, action: &str) -> String {
    format!(
        "pomodoro.{}.{}.{}",
        owner.instance, owner.presentation, action
    )
}
fn tray_id(owner: &Owner) -> String {
    id(owner, "tray")
}
struct Native<'a> {
    app: &'a AppHandle,
    coordinator: &'a Coordinator,
}
impl Native<'_> {
    fn tray(&self, s: &Surface) -> AppResult<()> {
        let label = s.owner.label();
        if self
            .coordinator
            .runtime
            .lock()
            .map_err(err)?
            .trays
            .contains_key(&label)
        {
            return Ok(());
        }
        let app = self.app;
        let home = MenuItem::with_id(app, id(&s.owner, "home"), "主页", true, None::<&str>)
            .map_err(err)?;
        let hidden = CheckMenuItem::with_id(
            app,
            id(&s.owner, "hide"),
            "隐藏",
            true,
            s.manual_hidden,
            None::<&str>,
        )
        .map_err(err)?;
        let add = MenuItem::with_id(app, id(&s.owner, "add"), "添加场景", true, None::<&str>)
            .map_err(err)?;
        let reset = MenuItem::with_id(app, id(&s.owner, "reset"), "重置位置", true, None::<&str>)
            .map_err(err)?;
        let close = MenuItem::with_id(app, id(&s.owner, "close"), "关闭专用窗", true, None::<&str>)
            .map_err(err)?;
        let menu = Menu::with_items(app, &[&home, &hidden, &add, &reset, &close]).map_err(err)?;
        let owner = s.owner.clone();
        let icon = TrayIconBuilder::with_id(tray_id(&owner))
            .icon(tauri::image::Image::new_owned(
                include_bytes!("../../components/widgetjs/pomodoro-source/pomodoro-tray.rgba")
                    .to_vec(),
                32,
                32,
            ))
            .tooltip("SoulDesk · 番茄钟")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_tray_icon_event(move |tray, event| {
                let app = tray.app_handle();
                let c = &app.state::<Core>().pomodoro_windows;
                match event {
                    TrayIconEvent::Enter { .. } | TrayIconEvent::Leave { .. } => {
                        let inside = matches!(event, TrayIconEvent::Enter { .. });
                        let _ = c
                            .service
                            .book
                            .lock()
                            .map(|mut b| b.hover(&owner, true, inside, c.now()));
                        c.kick(app);
                    }
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } => c.queue_action(app, owner.clone(), "open".into()),
                    _ => {}
                }
            })
            .build(app)
            .map_err(|_| "NATIVE_ERROR: 番茄托盘创建失败")?;
        self.coordinator.runtime.lock().map_err(err)?.trays.insert(
            label,
            Tray {
                _icon: icon,
                hidden,
            },
        );
        Ok(())
    }
}
impl Driver for Native<'_> {
    fn exists(&self, owner: &Owner) -> bool {
        self.app.get_webview_window(&owner.label()).is_some()
    }
    fn create(&mut self, s: &Surface) -> AppResult<()> {
        if !self.exists(&s.owner) {
            let directory = self
                .app
                .state::<Core>()
                .store
                .root
                .join("webview")
                .join(s.owner.label());
            std::fs::create_dir_all(&directory).map_err(err)?;
            let app = self.app.clone();
            WebviewWindowBuilder::new(
                self.app,
                s.owner.label(),
                WebviewUrl::App(format!("index.html?pomodoroWindow={}", s.owner.instance).into()),
            )
            .data_directory(directory)
            .title("SoulDesk · 番茄钟")
            .inner_size(state::SIZE, state::SIZE)
            .min_inner_size(state::SIZE, state::SIZE)
            .max_inner_size(state::SIZE, state::SIZE)
            .visible(false)
            .focused(false)
            .resizable(false)
            .decorations(false)
            .shadow(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .transparent(true)
            .background_color(tauri::window::Color(0, 0, 0, 0))
            .initialization_script(windows::host_script(self.app))
            .on_navigation(move |url| local(&app, url))
            .on_new_window(|_, _| NewWindowResponse::Deny)
            .build()
            .map_err(|_| "NATIVE_ERROR: 番茄专用窗创建失败")?;
        }
        self.tray(s)
    }
    fn place(&mut self, s: &Surface, at: Bounds) -> AppResult<Observed> {
        let w = self
            .app
            .get_webview_window(&s.owner.label())
            .ok_or("NATIVE_ERROR: 番茄专用窗未创建")?;
        let before = observe(&w)?;
        if !s.dragging && before.bounds != at {
            geometry(&w, at)?;
        }
        if s.target_visible() != before.visible {
            visibility(&w, s.target_visible())?;
        }
        let hidden = {
            self.coordinator
                .runtime
                .lock()
                .map_err(err)?
                .trays
                .get(&s.owner.label())
                .map(|t| t.hidden.clone())
        };
        if let Some(hidden) = hidden {
            if hidden.is_checked().map_err(err)? != s.manual_hidden {
                hidden.set_checked(s.manual_hidden).map_err(err)?;
            }
        }
        let focus = s.focus && s.target_visible() && {
            let mut r = self.coordinator.runtime.lock().map_err(err)?;
            if r.focus_attempts.get(&s.owner.label()) == Some(&s.focus_request) {
                false
            } else {
                r.focus_attempts.insert(s.owner.label(), s.focus_request);
                true
            }
        };
        if focus {
            w.set_focus()
                .map_err(|_| "NATIVE_ERROR: 专用窗焦点未确认，请重新打开")?;
        }
        if s.focus && w.is_focused().map_err(err)? {
            self.coordinator
                .service
                .book
                .lock()
                .map_err(err)?
                .focused(&s.owner, s.focus_request);
        }
        observe(&w)
    }
    fn conceal(&mut self, owner: &Owner) -> AppResult<()> {
        if let Some(w) = self.app.get_webview_window(&owner.label()) {
            visibility(&w, false)?;
        }
        Ok(())
    }
    fn destroy(&mut self, owner: &Owner) -> AppResult<bool> {
        let label = owner.label();
        // manager仍持有托盘资源，必须显式remove，不能只drop本地clone。
        self.app.remove_tray_by_id(&tray_id(owner));
        self.coordinator
            .runtime
            .lock()
            .map_err(err)?
            .trays
            .remove(&label);
        self.app
            .state::<Core>()
            .sessions
            .0
            .lock()
            .map_err(err)?
            .retain(|_, s| s.window != label);
        let first = self
            .coordinator
            .runtime
            .lock()
            .map_err(err)?
            .destroying
            .insert(label.clone());
        if let Some(w) = self.app.get_webview_window(&label) {
            let hidden = visibility(&w, false);
            if first {
                w.destroy()
                    .map_err(|_| "NATIVE_ERROR: 番茄专用窗释放失败，已保持隐藏")?;
            }
            hidden?;
        }
        let done = self.app.get_webview_window(&label).is_none()
            && self.app.tray_by_id(&tray_id(owner)).is_none();
        if done {
            let mut r = self.coordinator.runtime.lock().map_err(err)?;
            r.destroying.remove(&label);
            r.focus_attempts.remove(&label);
            r.drag_started.remove(&label);
            r.moved.remove(&label);
            r.checked.remove(&label);
            r.emitted.remove(&label);
            r.menus.retain(|_, (_, t)| t.menu.owner != *owner);
        }
        Ok(done)
    }
}
impl Coordinator {
    pub(crate) fn now(&self) -> u64 {
        self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }
    pub(crate) fn kick(&self, app: &AppHandle) {
        self.wake.notify_one();
        if self.driving.swap(true, Ordering::AcqRel) {
            return;
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let handle = app.clone();
                let result = tauri::async_runtime::spawn_blocking(move || {
                    let core = handle.state::<Core>();
                    let _actions = core.window_actions.lock().map_err(err)?;
                    core.pomodoro_windows.step(&handle)
                })
                .await;
                let active = match result {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => {
                        app.state::<Core>().report(e);
                        false
                    }
                    Err(e) => {
                        app.state::<Core>().report(err(e));
                        false
                    }
                };
                let c = &app.state::<Core>().pomodoro_windows;
                if !active {
                    c.driving.store(false, Ordering::Release);
                    // 未消费的通知会让下一次kick接续；每秒后台也会检查已保存许可。
                    break;
                }
                let moving = c
                    .service
                    .book
                    .lock()
                    .is_ok_and(|b| b.list().iter().any(|s| s.animating() || s.dragging));
                tokio::select! {_=c.wake.notified()=>{},_=tokio::time::sleep(Duration::from_millis(if moving{16}else{100}))=>{}}
            }
        });
    }
    fn current(&self, app: &AppHandle, owner: &Owner) -> bool {
        let core = app.state::<Core>();
        !self.service.stopped()
            && !core.quitting.load(Ordering::Acquire)
            && self.service.book.lock().is_ok_and(|b| b.current(owner))
            && host::source(&core.store, &owner.instance).is_ok_and(|s| {
                s.document.config.enabled && s.visible && !s.hidden && s.digest == owner.digest
            })
            && self
                .service
                .book
                .lock()
                .ok()
                .and_then(|b| b.get(&owner.instance))
                .is_some_and(|s| {
                    s.data_source.as_ref().is_none_or(|selected| {
                        host::selected_source(&core.store, &owner.instance)
                            .is_ok_and(|value| value == *selected)
                    })
                })
    }
    fn step(&self, app: &AppHandle) -> AppResult<bool> {
        let core = app.state::<Core>();
        let now = self.now();
        if self.reconcile.swap(false, Ordering::AcqRel) {
            self.service.reconcile(
                &core.store,
                &SystemDisplays {
                    app,
                    service: &self.service,
                },
                now_ms(),
                now,
            )?;
        }
        // 每秒调度负责业务阶段协调；动画不重复读取或推进业务计时。
        self.service.book.lock().map_err(err)?.tick(now)?;
        let mut native = Native {
            app,
            coordinator: self,
        };
        release(&self.service, &mut native)?;
        let surfaces = { self.service.book.lock().map_err(err)?.list() };
        for mut s in surfaces {
            let _native = core.store.native_actions.lock().map_err(err)?;
            let inspected = (|| -> AppResult<()> {
                if self.current(app, &s.owner) {
                    if let Some(w) = app.get_webview_window(&s.owner.label()) {
                        let due = {
                            let mut r = self.runtime.lock().map_err(err)?;
                            let last = r.checked.entry(s.owner.label()).or_default();
                            if now.saturating_sub(*last) >= 300 {
                                *last = now;
                                true
                            } else {
                                false
                            }
                        };
                        if s.dragging {
                            let last = {
                                let r = self.runtime.lock().map_err(err)?;
                                r.moved.get(&s.owner.label()).copied().unwrap_or_else(|| {
                                    r.drag_started.get(&s.owner.label()).copied().unwrap_or(now)
                                })
                            };
                            if !left_button_down() && now.saturating_sub(last) >= 100 {
                                let at = observe(&w)?.bounds;
                                let area = choose_area(
                                    &app.available_monitors()
                                        .map_err(err)?
                                        .iter()
                                        .map(monitor_area)
                                        .collect::<Vec<_>>(),
                                    at,
                                )?;
                                self.service
                                    .book
                                    .lock()
                                    .map_err(err)?
                                    .settle_drag(&s.owner, at, area, now)?;
                                s = self
                                    .service
                                    .book
                                    .lock()
                                    .map_err(err)?
                                    .get(&s.owner.instance)
                                    .ok_or("PRESENTATION_EXPIRED: 拖动窗口已关闭")?;
                            }
                        } else if due {
                            let area = SystemDisplays {
                                app,
                                service: &self.service,
                            }
                            .area(Some(&s.owner))?;
                            if (
                                area.left,
                                area.top,
                                area.width,
                                area.height,
                                area.scale_factor,
                            ) != (
                                s.area.left,
                                s.area.top,
                                s.area.width,
                                s.area.height,
                                s.area.scale_factor,
                            ) {
                                self.service.book.lock().map_err(err)?.reflow(
                                    &s.owner,
                                    observe(&w)?.bounds,
                                    area,
                                    now,
                                )?;
                                s = self
                                    .service
                                    .book
                                    .lock()
                                    .map_err(err)?
                                    .get(&s.owner.instance)
                                    .ok_or("PRESENTATION_EXPIRED: 工作区窗口已关闭")?;
                            }
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = inspected {
                let _ = native.conceal(&s.owner);
                retire(&self.service, &s.owner)?;
                self.service.error(&s.owner.instance, &error)?;
                continue;
            }
            let _ = render_surface(&self.service, &mut native, &s, now, || {
                self.current(app, &s.owner)
            });
            let latest = self
                .service
                .book
                .lock()
                .map_err(err)?
                .get(&s.owner.instance)
                .filter(|v| v.owner == s.owner);
            if let Some(latest) = latest {
                let value = latest.snapshot();
                let label = latest.owner.label();
                let changed = {
                    let mut r = self.runtime.lock().map_err(err)?;
                    if r.emitted.get(&label) == Some(&value) {
                        false
                    } else {
                        r.emitted.insert(label.clone(), value.clone());
                        true
                    }
                };
                if changed {
                    app.emit_to(&label, "host:pomodoro-window", &value)
                        .map_err(err)?;
                }
            }
        }
        self.runtime
            .lock()
            .map_err(err)?
            .menus
            .retain(|_, (_, t)| now.saturating_sub(t.at) < 30_000);
        let b = self.service.book.lock().map_err(err)?;
        Ok(!b.retired.is_empty() || !b.list().is_empty())
    }
    pub(crate) fn tick(&self, app: &AppHandle) {
        self.reconcile.store(true, Ordering::Release);
        self.kick(app);
    }
    pub(crate) fn cleanup(&self, app: &AppHandle) -> AppResult<()> {
        release(
            &self.service,
            &mut Native {
                app,
                coordinator: self,
            },
        )
    }
    pub(crate) fn suspend_all(&self) -> AppResult<()> {
        let mut book = self.service.book.lock().map_err(err)?;
        for s in book.list() {
            book.suspend(&s.owner.instance, true, self.now())?;
        }
        Ok(())
    }
    pub(crate) fn failed(&self, app: &AppHandle, label: &str, instance: &str) -> AppResult<()> {
        let current = { self.service.book.lock().map_err(err)?.by_label(label) };
        if let Some(s) = current.filter(|s| s.owner.instance == instance && !s.suspended) {
            retire(&self.service, &s.owner)?;
            self.service
                .error(instance, "LOAD_FAILED: 番茄专用窗加载失败，请重新打开")?;
            self.kick(app);
        }
        Ok(())
    }
    fn queue_action(&self, app: &AppHandle, owner: Owner, action: String) {
        windows::dispatch(app, move |app| {
            let c = &app.state::<Core>().pomodoro_windows;
            let result = c.action(app, &owner, &action);
            c.reconcile.store(true, Ordering::Release);
            c.kick(app);
            result
        });
    }
    fn action(&self, app: &AppHandle, owner: &Owner, action: &str) -> AppResult<()> {
        let core = app.state::<Core>();
        if matches!(action, "home" | "add") {
            {
                let _native = core.store.native_actions.lock().map_err(err)?;
                if !self.current(app, owner) {
                    return Ok(());
                }
            }
            return windows::detail_at(app, &owner.instance, "pomodoro-scenes");
        }
        let _native = core.store.native_actions.lock().map_err(err)?;
        if !self.current(app, owner) {
            return Ok(());
        }
        match action {
            "hide" => {
                let s = self
                    .service
                    .book
                    .lock()
                    .map_err(err)?
                    .get(&owner.instance)
                    .ok_or("PRESENTATION_EXPIRED: 专用窗已关闭")?;
                if s.manual_hidden {
                    let area = SystemDisplays {
                        app,
                        service: &self.service,
                    }
                    .area(Some(owner))?;
                    self.service.book.lock().map_err(err)?.open(
                        &owner.instance,
                        &owner.digest,
                        area,
                        true,
                        self.now(),
                    )?;
                } else {
                    self.service.book.lock().map_err(err)?.hide(owner)?;
                }
            }
            "open" => {
                let area = SystemDisplays {
                    app,
                    service: &self.service,
                }
                .area(Some(owner))?;
                self.service.book.lock().map_err(err)?.open(
                    &owner.instance,
                    &owner.digest,
                    area,
                    true,
                    self.now(),
                )?;
            }
            "reset" => {
                let area = SystemDisplays {
                    app,
                    service: &self.service,
                }
                .area(Some(owner))?;
                self.service
                    .book
                    .lock()
                    .map_err(err)?
                    .reposition(owner, area, self.now())?;
            }
            "close" => {
                let src = host::source(&core.store, &owner.instance)?;
                state::configure(
                    &mut *core.store.db.lock().map_err(err)?,
                    &owner.instance,
                    &Uuid::new_v4().to_string(),
                    src.document.revision,
                    state::Preferences { enabled: false },
                )?;
                self.service.revoke(&owner.instance)?;
            }
            _ => return Err("INVALID_ARGUMENT: 未知专用窗动作".into()),
        }
        Ok(())
    }
    pub(crate) fn menu_event(&self, app: &AppHandle, action: &str) -> bool {
        if !action.starts_with("pomodoro.") {
            return false;
        }
        let parts = action.split('.').collect::<Vec<_>>();
        if parts.len() != 4 {
            return true;
        }
        if parts[1] == "scene" {
            let ticket_id = parts[2].to_owned();
            let scene = parts[3].to_owned();
            let ticket = self
                .runtime
                .lock()
                .ok()
                .and_then(|mut r| r.menus.remove(&ticket_id));
            if let Some((_, ticket)) =
                ticket.filter(|(_, t)| self.now().saturating_sub(t.at) < 30_000)
            {
                let app = app.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let core = app.state::<Core>();
                    let c = &core.pomodoro_windows;
                    if let Err(e) = host::select_scene(
                        &c.service,
                        &core.store,
                        &core.sessions,
                        &ticket.menu,
                        &scene,
                        &Uuid::new_v4().to_string(),
                        now_ms(),
                    ) {
                        let _ = c.service.error(&ticket.menu.owner.instance, &e);
                    }
                    c.kick(&app);
                });
            }
            return true;
        }
        let current = self
            .service
            .book
            .lock()
            .ok()
            .and_then(|b| b.get(parts[1]))
            .filter(|s| s.owner.presentation == parts[2]);
        if let Some(s) = current {
            self.queue_action(app, s.owner, parts[3].into());
        }
        true
    }
    fn popup(&self, app: &AppHandle, menu: SceneMenu) -> AppResult<()> {
        if !self.current(app, &menu.owner) {
            return Err("PRESENTATION_EXPIRED: 菜单窗口已关闭".into());
        }
        let ticket = Uuid::new_v4().to_string();
        let native = Menu::new(app).map_err(err)?;
        for (scene, title, checked) in &menu.scenes {
            let item = CheckMenuItem::with_id(
                app,
                format!("pomodoro.scene.{ticket}.{scene}"),
                title,
                menu.can_select,
                *checked,
                None::<&str>,
            )
            .map_err(err)?;
            native.append(&item).map_err(err)?;
        }
        let w = app
            .get_webview_window(&menu.owner.label())
            .ok_or("PRESENTATION_EXPIRED: 菜单窗口已关闭")?;
        self.runtime.lock().map_err(err)?.menus.insert(
            ticket.clone(),
            (
                native.clone(),
                Ticket {
                    menu,
                    at: self.now(),
                },
            ),
        );
        if let Err(e) = w.popup_menu(&native) {
            self.runtime.lock().map_err(err)?.menus.remove(&ticket);
            return Err(err(e));
        }
        Ok(())
    }
    pub(crate) fn window_event(&self, app: &AppHandle, label: &str, event: &WindowEvent) -> bool {
        if !label.starts_with("pomodoro-window-") {
            return false;
        }
        let current = self
            .service
            .book
            .lock()
            .ok()
            .and_then(|b| b.by_label(label));
        if let Some(s) = current {
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = self.service.book.lock().map(|mut b| b.hide(&s.owner));
                }
                WindowEvent::Destroyed => {
                    let _ = retire(&self.service, &s.owner);
                    if !s.suspended && !self.service.stopped() {
                        let _ = self.service.error(
                            &s.owner.instance,
                            "NATIVE_ERROR: 番茄专用窗意外关闭，请重新打开",
                        );
                    }
                }
                WindowEvent::Moved(_) => {
                    if let Ok(mut r) = self.runtime.lock() {
                        r.moved.insert(label.into(), self.now());
                    }
                }
                WindowEvent::Focused(true) => {
                    let _ = self
                        .service
                        .book
                        .lock()
                        .map(|mut b| b.focused(&s.owner, s.focus_request));
                }
                _ => {}
            }
        }
        if matches!(event, WindowEvent::Destroyed) {
            if let Ok(mut sessions) = app.state::<Core>().sessions.0.lock() {
                sessions.retain(|_, s| s.window != label);
            }
        }
        self.kick(app);
        true
    }
}
#[cfg(windows)]
fn left_button_down() -> bool {
    // 只在用户已发起的本窗拖动中查询释放，不注册全局钩子或读取鼠标坐标。
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    unsafe { GetAsyncKeyState(VK_LBUTTON as i32) < 0 }
}
#[cfg(not(windows))]
fn left_button_down() -> bool {
    false
}
pub(crate) fn surface_method(label: &str, method: &str) -> AppResult<()> {
    if !label.starts_with("pomodoro-window-")
        || matches!(
            method,
            "pomodoro.get"
                | "pomodoro.change"
                | "pomodoro.window.status"
                | "pomodoro.window.hover"
                | "pomodoro.window.ready"
                | "pomodoro.window.drag"
                | "pomodoro.window.menu"
                | "pomodoro.window.hide"
                | "runtime.ready"
                | "view.openDetails"
        )
    {
        Ok(())
    } else {
        Err("CAPABILITY_DENIED: 专用窗只接受当前番茄钟及固定窗口动作".into())
    }
}
pub(crate) async fn gateway(
    app: &AppHandle,
    label: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    let core = app.state::<Core>();
    let c = &core.pomodoro_windows;
    let display = SystemDisplays {
        app,
        service: &c.service,
    };
    let reply = c.service.access(
        &core.store,
        &core.sessions,
        label,
        token,
        method,
        args,
        c.now(),
        &display,
    )?;
    if let Some(permit) = reply.drag {
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let result = (|| -> AppResult<()> {
                let app = &handle;
                let core = app.state::<Core>();
                let c = &core.pomodoro_windows;
                let _native = core.store.native_actions.lock().map_err(err)?;
                if !c.service.drag_current(&core.store, &permit)? {
                    return Err("PRESENTATION_EXPIRED: 拖动许可已失效".into());
                }
                let w = app
                    .get_webview_window(&permit.owner.label())
                    .ok_or("PRESENTATION_EXPIRED: 拖动窗口已关闭")?;
                c.runtime
                    .lock()
                    .map_err(err)?
                    .drag_started
                    .insert(permit.owner.label(), c.now());
                // Windows拖动可能等待用户释放鼠标；校验后释放数据动作锁，后台计时继续。
                drop(_native);
                c.kick(app);
                if let Err(e) = w.start_dragging() {
                    let area = SystemDisplays {
                        app,
                        service: &c.service,
                    }
                    .area(Some(&permit.owner))?;
                    c.service.book.lock().map_err(err)?.settle_drag(
                        &permit.owner,
                        observe(&w)?.bounds,
                        area,
                        c.now(),
                    )?;
                    return Err(err(e));
                }
                Ok(())
            })();
            let c = &handle.state::<Core>().pomodoro_windows;
            if let Err(e) = result {
                let _ = c.service.error(&permit.owner.instance, &e);
            }
            c.kick(&handle);
        });
    }
    if matches!(
        method,
        "pomodoro.window.configure" | "pomodoro.window.open" | "pomodoro.window.reset"
    ) {
        c.reconcile.store(true, Ordering::Release);
    }
    c.kick(app);
    if let Some(menu) = reply.menu {
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let c = &handle.state::<Core>().pomodoro_windows;
            let instance = menu.owner.instance.clone();
            if let Err(e) = c.popup(&handle, menu) {
                let _ = c.service.error(&instance, &e);
            }
            c.kick(&handle);
        });
    }
    Ok(reply.value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Fake<'a> {
        live: bool,
        events: Vec<&'static str>,
        placed: Option<Observed>,
        wrong_position: bool,
        fail_create: bool,
        closed: bool,
        after_create: Option<Box<dyn FnMut() + 'a>>,
        after_place: Option<Box<dyn FnMut() + 'a>>,
    }
    impl Driver for Fake<'_> {
        fn exists(&self, _: &Owner) -> bool {
            self.live
        }
        fn create(&mut self, _: &Surface) -> AppResult<()> {
            self.events.push("create");
            self.live = true;
            if let Some(f) = &mut self.after_create {
                f();
            }
            if self.fail_create {
                Err("NATIVE_ERROR: 受控创建失败".into())
            } else {
                Ok(())
            }
        }
        fn place(&mut self, s: &Surface, mut at: Bounds) -> AppResult<Observed> {
            self.events.push("place");
            if self.wrong_position {
                at.x += 1;
            }
            let value = Observed {
                bounds: at,
                visible: s.target_visible(),
            };
            self.placed = Some(value);
            if let Some(f) = &mut self.after_place {
                f();
            }
            Ok(value)
        }
        fn conceal(&mut self, _: &Owner) -> AppResult<()> {
            self.events.push("hide");
            if let Some(v) = &mut self.placed {
                v.visible = false;
            }
            Ok(())
        }
        fn destroy(&mut self, _: &Owner) -> AppResult<bool> {
            self.events.push("destroy");
            if self.closed {
                self.live = false;
            }
            Ok(self.closed)
        }
    }
    fn area() -> WorkArea {
        WorkArea {
            left: 0,
            top: 0,
            width: 1920,
            height: 1040,
            scale_factor: 1.,
        }
    }
    fn surface(service: &Service) -> Surface {
        service
            .book
            .lock()
            .unwrap()
            .open(
                &Uuid::new_v4().to_string(),
                &"a".repeat(64),
                area(),
                false,
                0,
            )
            .unwrap()
    }
    fn ready(service: &Service, s: &Surface) -> Surface {
        let token = Uuid::new_v4().to_string();
        let mut b = service.book.lock().unwrap();
        b.bind(&s.owner.label(), &token).unwrap();
        b.ready(&s.owner.label(), &token, 0).unwrap();
        b.get(&s.owner.instance).unwrap()
    }
    #[test]
    fn default_and_hidden_views_have_no_native_creation() {
        let service = Service::default();
        let mut d = Fake::default();
        release(&service, &mut d).unwrap();
        assert!(d.events.is_empty());
        let s = surface(&service);
        service.book.lock().unwrap().hide(&s.owner).unwrap();
        let s = service.book.lock().unwrap().get(&s.owner.instance).unwrap();
        render_surface(&service, &mut d, &s, 0, || true).unwrap();
        assert!(d.events.is_empty());
    }
    #[test]
    fn load_stays_hidden_until_ready_and_uses_actual_position_receipts() {
        let service = Service::default();
        let mut d = Fake::default();
        let s = surface(&service);
        render_surface(&service, &mut d, &s, 0, || true).unwrap();
        assert!(!d.placed.unwrap().visible);
        let s = ready(&service, &s);
        d.wrong_position = true;
        render_surface(&service, &mut d, &s, 1, || true).unwrap();
        assert_eq!(
            service
                .book
                .lock()
                .unwrap()
                .get(&s.owner.instance)
                .unwrap()
                .snapshot()["nativeSettled"],
            false
        );
        d.wrong_position = false;
        render_surface(&service, &mut d, &s, 2, || true).unwrap();
        assert_eq!(
            service
                .book
                .lock()
                .unwrap()
                .get(&s.owner.instance)
                .unwrap()
                .snapshot()["nativeSettled"],
            true
        );
        assert!(d.placed.unwrap().visible);
    }
    #[test]
    fn revocation_during_create_prevents_late_show() {
        let service = Service::default();
        let s = ready(&service, &surface(&service));
        let permitted = AtomicBool::new(true);
        let mut d = Fake {
            after_create: Some(Box::new(|| permitted.store(false, Ordering::Release))),
            ..Default::default()
        };
        render_surface(&service, &mut d, &s, 0, || {
            permitted.load(Ordering::Acquire)
        })
        .unwrap();
        assert_eq!(d.events, vec!["create", "hide"]);
        assert!(!service.book.lock().unwrap().current(&s.owner));
    }
    #[test]
    fn hide_during_place_conceals_the_late_frame_without_retiring_current_owner() {
        let service = Service::default();
        let s = ready(&service, &surface(&service));
        let mut d = Fake {
            after_place: Some(Box::new(|| {
                service.book.lock().unwrap().hide(&s.owner).unwrap();
            })),
            ..Default::default()
        };
        render_surface(&service, &mut d, &s, 0, || true).unwrap();
        assert_eq!(d.events, vec!["create", "place", "hide"]);
        assert!(!d.placed.unwrap().visible);
        let current = service.book.lock().unwrap().get(&s.owner.instance).unwrap();
        assert!(current.manual_hidden);
        assert_eq!(current.applied, 0);
    }
    #[test]
    fn failed_create_is_hidden_retired_and_not_retried_by_renderer() {
        let service = Service::default();
        let s = surface(&service);
        let mut d = Fake {
            fail_create: true,
            ..Default::default()
        };
        assert!(render_surface(&service, &mut d, &s, 0, || true).is_err());
        assert_eq!(d.events, vec!["create", "hide"]);
        render_surface(&service, &mut d, &s, 1, || true).unwrap();
        assert_eq!(d.events, vec!["create", "hide", "hide"]);
    }
    #[test]
    fn stop_and_cleanup_wait_for_actual_resource_release() {
        let service = Service::default();
        let s = surface(&service);
        let mut d = Fake {
            live: true,
            ..Default::default()
        };
        service.stop().unwrap();
        release(&service, &mut d).unwrap();
        assert_eq!(service.book.lock().unwrap().retired, vec![s.owner.clone()]);
        d.closed = true;
        release(&service, &mut d).unwrap();
        assert!(service.book.lock().unwrap().retired.is_empty());
        assert!(!d.live);
        render_surface(&service, &mut d, &s, 1, || true).unwrap();
        assert!(!d.events.iter().any(|e| *e == "create"));
    }
    #[test]
    fn monitor_selection_uses_expanded_anchor_including_negative_coordinates() {
        let left = WorkArea {
            left: -1920,
            ..area()
        };
        let right = area();
        let anchor = Bounds {
            x: -150,
            y: 800,
            width: 150,
            height: 150,
        };
        assert_eq!(choose_area(&[right, left], anchor).unwrap().left, -1920);
        assert!(choose_area(&[], anchor).is_err());
    }
    #[test]
    fn dedicated_method_allowlist_rejects_general_storage_network_and_input() {
        for method in [
            "storage.get",
            "library.bind",
            "market.snapshot",
            "keyboard.configure",
            "pomodoro.window.configure",
            "pomodoro.window.setBounds",
            "files.open",
            "view.openExternal",
        ] {
            assert!(
                surface_method("pomodoro-window-invalid", method).is_err(),
                "{method}"
            );
        }
        for method in [
            "pomodoro.get",
            "pomodoro.change",
            "runtime.ready",
            "view.openDetails",
            "pomodoro.window.drag",
            "pomodoro.window.menu",
        ] {
            surface_method("pomodoro-window-invalid", method).unwrap();
        }
        surface_method("panel", "library.bind").unwrap();
    }
}
