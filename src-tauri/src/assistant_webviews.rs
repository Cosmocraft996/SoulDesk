//! 固定网页助手的子视图协调；所有系统操作由驱动负责，测试不创建网页。
use crate::{
    assistant_state::{self, Config, Platform, Proxy},
    store::AppResult,
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
pub(crate) fn layout(count: u8, width: f64, height: f64) -> AppResult<Vec<Rect>> {
    if ![1, 2, 4].contains(&count)
        || !width.is_finite()
        || !height.is_finite()
        || !(160.0..=16384.0).contains(&width)
        || !(200.0..=16384.0).contains(&height)
    {
        return Err("INVALID_ARGUMENT: 助手视口或分屏数无效".into());
    }
    let columns = if count == 4 { 2.0 } else { 1.0 };
    let rows = if count == 1 { 1.0 } else { 2.0 };
    let cell_width = (width - 24.0 - (columns - 1.0) * 12.0) / columns;
    let cell_height = (height - 56.0 - 12.0 - (rows - 1.0) * 12.0) / rows;
    Ok((0..count)
        .map(|i| Rect {
            x: 12.0 + (i as f64 % columns) * (cell_width + 12.0),
            y: 56.0 + (i as f64 / columns).floor() * (cell_height + 12.0),
            width: cell_width,
            height: cell_height - 40.0,
        })
        .collect())
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Owner {
    pub instance: String,
    pub presentation: String,
    pub parent: String,
    pub revision: u64,
}
impl Owner {
    pub(crate) fn validate(&self) -> AppResult<()> {
        assistant_state::key(&self.instance)?;
        if !uuid::Uuid::parse_str(&self.presentation)
            .is_ok_and(|id| id.to_string() == self.presentation)
            || self.parent != format!("assistant-panel-{}-{}", self.instance, self.presentation)
            || self.revision > 9_007_199_254_740_991
        {
            return Err("INVALID_ARGUMENT: 助手展示归属无效".into());
        }
        Ok(())
    }
    pub(crate) fn label(&self, slot: u8) -> String {
        format!(
            "assistant-page-{}-{}-{slot}",
            self.instance, self.presentation
        )
    }
    pub(crate) fn shell(&self) -> String {
        format!("assistant-shell-{}-{}", self.instance, self.presentation)
    }
    pub(crate) fn shell_profile(&self, root: &Path) -> AppResult<PathBuf> {
        self.validate()?;
        Ok(root.join("webview").join(self.shell()))
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Child {
    pub owner: Owner,
    pub slot: u8,
    pub platform: Platform,
    pub proxy: Proxy,
    pub profile: PathBuf,
    pub rect: Rect,
    pub shown: bool,
    pub uncertain: bool,
}
impl Child {
    pub(crate) fn label(&self) -> String {
        self.owner.label(self.slot)
    }
}
pub(crate) trait Driver {
    /// 新子视图必须在不可见位置、无焦点创建，重验归属后才能显示。
    fn create(&mut self, child: &Child) -> AppResult<()>;
    fn navigate(&mut self, child: &Child) -> AppResult<()>;
    fn place(&mut self, child: &Child) -> AppResult<()>;
    fn visible(&mut self, label: &str, visible: bool) -> AppResult<()>;
    fn close(&mut self, label: &str) -> AppResult<()>;
    fn close_auxiliary(&mut self, instance: &str) -> AppResult<()>;
}
#[derive(Default)]
pub(crate) struct Controller {
    children: HashMap<String, Child>,
    pending_profiles: HashSet<String>,
}
#[derive(Serialize)]
pub(crate) struct Attached {
    pub attached: usize,
    pub shown: usize,
}
impl Controller {
    fn close_one(&mut self, label: &str, driver: &mut impl Driver) -> AppResult<()> {
        match driver.close(label) {
            Ok(()) => {
                self.children.remove(label);
                Ok(())
            }
            Err(error) => {
                if let Some(child) = self.children.get_mut(label) {
                    child.uncertain = true;
                }
                Err(error)
            }
        }
    }
    pub(crate) fn release(&mut self, instance: &str, driver: &mut impl Driver) -> AppResult<()> {
        assistant_state::key(instance)?;
        let mut error = driver.close_auxiliary(instance).err();
        if error.is_some() {
            self.pending_profiles.insert(instance.to_owned());
        } else {
            self.pending_profiles.remove(instance);
        }
        let mut labels = self
            .children
            .iter()
            .filter(|(_, c)| c.owner.instance == instance)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        labels.sort();
        for label in labels {
            if let Err(e) = self.close_one(&label, driver) {
                if error.is_none() {
                    error = Some(e);
                }
            }
        }
        if let Some(error) = error {
            Err(error)
        } else {
            Ok(())
        }
    }
    pub(crate) fn reconcile(
        &mut self,
        root: &Path,
        owner: &Owner,
        config: &Config,
        size: (f64, f64),
        visible: bool,
        overlay: bool,
        driver: &mut impl Driver,
        mut current: impl FnMut() -> bool,
    ) -> AppResult<Attached> {
        owner.validate()?;
        config.validate()?;
        let rects = layout(config.page_count, size.0, size.1)?;
        let profile = assistant_state::profile_directory(root, &owner.instance)?;
        let mut check = || {
            if current() {
                Ok(())
            } else {
                Err("SESSION_EXPIRED: 助手网页展示或配置已变化".to_owned())
            }
        };
        check()?;
        let result = (|| {
            let reset = self.pending_profiles.contains(&owner.instance)
                || self.children.values().any(|c| {
                    c.owner.instance == owner.instance
                        && (c.owner.presentation != owner.presentation
                            || c.proxy != config.proxy
                            || c.uncertain)
                });
            if reset || !visible && !config.background_enabled {
                self.release(&owner.instance, driver)?;
                check()?;
            }
            if !visible {
                if let Err(error) = driver.close_auxiliary(&owner.instance) {
                    self.pending_profiles.insert(owner.instance.clone());
                    return Err(error);
                }
                self.pending_profiles.remove(&owner.instance);
                check()?;
            }
            let mut removed = self
                .children
                .iter()
                .filter(|(_, c)| c.owner.instance == owner.instance && c.slot >= config.page_count)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            removed.sort();
            for label in removed {
                check()?;
                self.close_one(&label, driver)?;
                check()?;
            }
            let draw = visible && !overlay;
            for (slot, rect) in rects.into_iter().enumerate() {
                let label = owner.label(slot as u8);
                let mut wanted = Child {
                    owner: owner.clone(),
                    slot: slot as u8,
                    platform: config.platforms[slot],
                    proxy: config.proxy.clone(),
                    profile: profile.clone(),
                    rect,
                    shown: false,
                    uncertain: false,
                };
                check()?;
                if !draw
                    && self
                        .children
                        .get(&label)
                        .is_some_and(|old| old.platform != wanted.platform)
                {
                    self.close_one(&label, driver)?;
                    check()?;
                }
                if !self.children.contains_key(&label) {
                    if !draw {
                        continue;
                    }
                    wanted.uncertain = true;
                    self.children.insert(label.clone(), wanted.clone());
                    driver.create(&wanted)?;
                    check()?;
                    wanted.uncertain = false;
                    self.children.insert(label.clone(), wanted.clone());
                    // 驱动先建空白视图；再次确认归属后才开始远程导航。
                    driver.navigate(&wanted)?;
                    check()?;
                } else {
                    let previous = self.children[&label].clone();
                    wanted.shown = previous.shown;
                    self.children.insert(label.clone(), wanted.clone());
                    if previous.platform != wanted.platform {
                        driver.navigate(&wanted)?;
                        check()?;
                    }
                }
                check()?;
                driver.place(&wanted)?;
                check()?;
                if wanted.shown != draw {
                    driver.visible(&label, draw)?;
                    check()?;
                    wanted.shown = draw;
                }
                self.children.insert(label, wanted);
            }
            let children = self
                .children
                .values()
                .filter(|c| c.owner.instance == owner.instance)
                .collect::<Vec<_>>();
            Ok(Attached {
                attached: children.len(),
                shown: children.iter().filter(|c| c.shown).count(),
            })
        })();
        if let Err(error) = result {
            let cleanup = if error.starts_with("SESSION_EXPIRED:") {
                // 新布局/覆盖层可能在摆放期间到达；隐藏已确认的旧页，交给下一计划复用。
                // 尚未确认创建归属的页面仍必须关闭，不能开始远程导航。
                let labels = self
                    .children
                    .iter()
                    .filter(|(_, c)| c.owner.instance == owner.instance)
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();
                let mut failed = None;
                for label in labels {
                    if self.children[&label].uncertain {
                        if let Err(e) = self.close_one(&label, driver) {
                            failed.get_or_insert(e);
                        }
                    } else if driver.visible(&label, false).is_ok() {
                        self.children.get_mut(&label).unwrap().shown = false;
                    } else if let Err(e) = self.close_one(&label, driver) {
                        failed.get_or_insert(e);
                    }
                }
                failed
            } else {
                self.release(&owner.instance, driver).err()
            };
            return Err(if let Some(cleanup) = cleanup {
                format!("{error}；旧网页释放未确认：{cleanup}")
            } else {
                error
            });
        }
        result
    }
}

pub(crate) fn web_navigation_allowed(url: &url::Url) -> bool {
    url.as_str() == "about:blank"
        || url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|host| host != "localhost" && !host.ends_with(".localhost"))
            && url.username().is_empty()
            && url.password().is_none()
}

/// 导航地址只用于当次核对，不写入状态快照或日志。
pub(crate) enum Navigation {
    Starting { id: u64, url: url::Url },
    Completed { id: u64, success: bool, error: i32 },
}
#[derive(Clone)]
pub(crate) struct PageState {
    pub epoch: u64,
    pub platform: Platform,
    bound: bool,
    active: Option<u64>,
    phase: &'static str,
    error: Option<String>,
}
impl PageState {
    pub(crate) fn new(platform: Platform, epoch: u64) -> Self {
        Self {
            epoch,
            platform,
            bound: false,
            active: None,
            phase: "pending",
            error: None,
        }
    }
    pub(crate) fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"platform":self.platform,"phase":self.phase,"error":self.error})
    }
    pub(crate) fn popup_ready(&self) -> bool {
        self.bound
    }
    pub(crate) fn event(&mut self, event: Navigation) -> bool {
        match event {
            Navigation::Starting { id, url } => {
                if !web_navigation_allowed(&url) {
                    return false;
                }
                if url.as_str() == "about:blank" {
                    return true;
                }
                if !self.bound
                    && url
                        != self
                            .platform
                            .url()
                            .parse::<url::Url>()
                            .expect("固定网页地址有效")
                {
                    return false;
                }
                self.bound = true;
                self.active = Some(id);
                self.phase = "loading";
                self.error = None;
                true
            }
            Navigation::Completed { id, success, error } => {
                if self.active != Some(id) {
                    return false;
                }
                self.phase = if success { "ended" } else { "failed" };
                self.error = if success {
                    None
                } else {
                    Some(format!("网页导航失败（状态{error}），请查看页面或检查连接"))
                };
                true
            }
        }
    }
}

#[cfg(windows)]
pub(crate) mod native {
    use super::*;
    use std::sync::Arc;
    use tauri::{
        webview::{NewWindowFeatures, NewWindowResponse, WebviewBuilder},
        AppHandle, Manager, WebviewUrl,
    };
    type Popup = Arc<
        dyn Fn(&Owner, u8, &str, url::Url, NewWindowFeatures) -> NewWindowResponse<tauri::Wry>
            + Send
            + Sync,
    >;
    type Page = Arc<dyn Fn(&Owner, u8, &str, Option<Navigation>) -> bool + Send + Sync>;
    pub(crate) struct Native {
        app: AppHandle,
        popup: Popup,
        page: Page,
    }
    impl Native {
        pub(crate) fn new(app: AppHandle, popup: Popup, page: Page) -> Self {
            Self { app, popup, page }
        }
        fn navigation_events(
            &self,
            view: &tauri::Webview,
            owner: Owner,
            slot: u8,
            tag: String,
        ) -> AppResult<()> {
            use webview2_com::{
                Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
                NavigationCompletedEventHandler, NavigationStartingEventHandler,
            };
            let page = self.page.clone();
            if !page(&owner, slot, &tag, None) {
                return Err("SESSION_EXPIRED: 助手网页创建归属已撤销".into());
            }
            let (send, receive) = std::sync::mpsc::sync_channel(1);
            view.with_webview(move |platform| {
                // 回调参数来自WebView2；只读取导航编号、地址和状态，不取页面内容。
                let result: windows::core::Result<()> = (|| unsafe {
                    let webview = platform.controller().CoreWebView2()?;
                    let starting_owner = owner.clone();
                    let starting_page = page.clone();
                    let starting_tag = tag.clone();
                    let mut starting_token = 0;
                    webview.add_NavigationStarting(
                        &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                            let Some(args) = args else {
                                return Ok(());
                            };
                            let mut id = 0;
                            args.NavigationId(&mut id)?;
                            let mut uri = windows::core::PWSTR::null();
                            let read = args.Uri(&mut uri);
                            let owned = webview2_com::CoTaskMemPWSTR::from(uri);
                            read?;
                            let allow = url::Url::parse(&owned.to_string()).is_ok_and(|url| {
                                starting_page(
                                    &starting_owner,
                                    slot,
                                    &starting_tag,
                                    Some(Navigation::Starting { id, url }),
                                )
                            });
                            args.SetCancel(!allow)?;
                            Ok(())
                        })),
                        &mut starting_token,
                    )?;
                    let mut finished_token = 0;
                    let completed = webview.add_NavigationCompleted(
                        &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                            let Some(args) = args else {
                                return Ok(());
                            };
                            let mut id = 0;
                            let mut success = windows::core::BOOL::default();
                            let mut error = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
                            args.NavigationId(&mut id)?;
                            args.IsSuccess(&mut success)?;
                            args.WebErrorStatus(&mut error)?;
                            page(
                                &owner,
                                slot,
                                &tag,
                                Some(Navigation::Completed {
                                    id,
                                    success: success.as_bool(),
                                    error: error.0,
                                }),
                            );
                            Ok(())
                        })),
                        &mut finished_token,
                    );
                    if completed.is_err() {
                        let _ = webview.remove_NavigationStarting(starting_token);
                    }
                    completed
                })();
                let _ = send
                    .send(result.map_err(|_| "NATIVE_ERROR: 网页导航结果监听未建立".to_owned()));
            })
            .map_err(crate::store::err)?;
            receive
                .recv_timeout(std::time::Duration::from_secs(5))
                .map_err(|_| "NATIVE_ERROR: 网页导航监听结果未确定")?
        }
    }
    impl Driver for Native {
        fn create(&mut self, child: &Child) -> AppResult<()> {
            child.owner.validate()?;
            let core = self.app.state::<crate::Core>();
            if child.profile
                != assistant_state::profile_directory(&core.store.root, &child.owner.instance)?
            {
                return Err("CAPABILITY_DENIED: 网页资料目录不属于当前实例".into());
            }
            let parent = self
                .app
                .get_window(&child.owner.parent)
                .ok_or("PRESENTATION_EXPIRED: 助手原生窗口已关闭")?;
            if self.app.get_webview(&child.label()).is_some() {
                return Err("NATIVE_ERROR: 旧网页尚未释放，未重复创建".into());
            }
            std::fs::create_dir_all(&child.profile).map_err(crate::store::err)?;
            let popup = self.popup.clone();
            let owner = child.owner.clone();
            let slot = child.slot;
            let tag = uuid::Uuid::new_v4().to_string();
            let popup_tag = tag.clone();
            let mut builder = WebviewBuilder::new(
                child.label(),
                WebviewUrl::External("about:blank".parse().map_err(crate::store::err)?),
            )
            .data_directory(child.profile.clone())
            .focused(false)
            .devtools(false)
            .on_navigation(web_navigation_allowed)
            .on_new_window(move |url, features| {
                if !web_navigation_allowed(&url) {
                    NewWindowResponse::Deny
                } else {
                    popup(&owner, slot, &popup_tag, url, features)
                }
            });
            if let Some(args) = child.proxy.browser_arguments()? {
                builder = builder.additional_browser_args(&args);
            }
            // 先在客户区外创建；完成再次鉴权后，协调器才摆放并显示。
            let view = parent
                .add_child(
                    builder,
                    tauri::LogicalPosition::new(
                        -child.rect.width - 20.0,
                        -child.rect.height - 20.0,
                    ),
                    tauri::LogicalSize::new(child.rect.width, child.rect.height),
                )
                .map_err(crate::store::err)?;
            view.hide().map_err(crate::store::err)?;
            self.navigation_events(&view, child.owner.clone(), child.slot, tag)
        }
        fn navigate(&mut self, child: &Child) -> AppResult<()> {
            self.app
                .get_webview(&child.label())
                .ok_or("NATIVE_ERROR: 助手网页不存在")?
                .navigate(child.platform.url().parse().map_err(crate::store::err)?)
                .map_err(crate::store::err)
        }
        fn place(&mut self, child: &Child) -> AppResult<()> {
            self.app
                .get_webview(&child.label())
                .ok_or("NATIVE_ERROR: 助手网页不存在")?
                .set_bounds(tauri::Rect {
                    position: tauri::LogicalPosition::new(child.rect.x, child.rect.y).into(),
                    size: tauri::LogicalSize::new(child.rect.width, child.rect.height).into(),
                })
                .map_err(crate::store::err)
        }
        fn visible(&mut self, label: &str, visible: bool) -> AppResult<()> {
            let view = self
                .app
                .get_webview(label)
                .ok_or("NATIVE_ERROR: 助手网页不存在")?;
            if visible { view.show() } else { view.hide() }.map_err(crate::store::err)
        }
        fn close(&mut self, label: &str) -> AppResult<()> {
            if let Some(view) = self.app.get_webview(label) {
                view.close().map_err(crate::store::err)?;
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while self.app.get_webview(label).is_some() {
                if std::time::Instant::now() >= deadline {
                    return Err("NATIVE_ERROR: 旧助手网页释放尚未确认".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Ok(())
        }
        fn close_auxiliary(&mut self, instance: &str) -> AppResult<()> {
            assistant_state::key(instance)?;
            let prefix = format!("assistant-auth-{instance}-");
            let windows = self.app.windows();
            let mut error = None;
            for (label, window) in windows {
                if label.starts_with(&prefix) {
                    if let Err(e) = window.destroy() {
                        if error.is_none() {
                            error = Some(crate::store::err(e));
                        }
                    } else {
                        let deadline =
                            std::time::Instant::now() + std::time::Duration::from_secs(3);
                        while self.app.get_window(&label).is_some() {
                            if std::time::Instant::now() >= deadline {
                                error.get_or_insert_with(|| {
                                    "NATIVE_ERROR: 旧登录窗口释放尚未确认".into()
                                });
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }
                }
            }
            if let Some(e) = error {
                Err(e)
            } else {
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashSet,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    };
    fn owner(instance: Option<&str>) -> Owner {
        let instance = instance
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let presentation = uuid::Uuid::new_v4().to_string();
        Owner {
            parent: format!("assistant-panel-{instance}-{presentation}"),
            instance,
            presentation,
            revision: 0,
        }
    }
    fn root() -> PathBuf {
        std::env::temp_dir().join("assistant-no-native-profile-tests")
    }
    #[test]
    fn assistant_webviews_navigation_checks_initial_target_and_never_serializes_private_url() {
        let mut p = PageState::new(Platform::Chatgpt, 1);
        assert!(!p.event(Navigation::Starting {
            id: 1,
            url: "https://grok.com/private?q=secret".parse().unwrap()
        }));
        assert!(p.event(Navigation::Starting {
            id: 2,
            url: Platform::Chatgpt.url().parse().unwrap()
        }));
        assert!(p.event(Navigation::Starting {
            id: 3,
            url: "https://chatgpt.com/c/private?q=secret".parse().unwrap()
        }));
        assert_eq!(p.snapshot()["phase"], "loading");
        assert!(!p.snapshot().to_string().contains("secret"));
        assert!(!p.event(Navigation::Starting {
            id: 4,
            url: "https://tauri.localhost/".parse().unwrap()
        }));
    }
    #[test]
    fn assistant_webviews_navigation_failure_is_distinct_from_end_and_not_login_success() {
        let mut p = PageState::new(Platform::Kimi, 0);
        p.event(Navigation::Starting {
            id: 4,
            url: Platform::Kimi.url().parse().unwrap(),
        });
        p.event(Navigation::Completed {
            id: 4,
            success: false,
            error: 12,
        });
        assert_eq!(p.snapshot()["phase"], "failed");
        assert!(p.snapshot()["error"].as_str().unwrap().contains("12"));
        p.event(Navigation::Starting {
            id: 5,
            url: "https://www.kimi.com/".parse().unwrap(),
        });
        p.event(Navigation::Completed {
            id: 5,
            success: true,
            error: 0,
        });
        assert_eq!(p.snapshot()["phase"], "ended");
        assert!(p.snapshot()["error"].is_null());
        assert!(p.snapshot().get("loggedIn").is_none());
    }
    #[test]
    fn assistant_webviews_navigation_late_completions_cannot_clear_new_page_loading() {
        let mut p = PageState::new(Platform::Chatgpt, 0);
        p.event(Navigation::Starting {
            id: 8,
            url: Platform::Chatgpt.url().parse().unwrap(),
        });
        p.event(Navigation::Starting {
            id: 9,
            url: "https://chatgpt.com/c/new".parse().unwrap(),
        });
        p.event(Navigation::Completed {
            id: 8,
            success: false,
            error: 5,
        });
        assert_eq!(p.snapshot()["phase"], "loading");
        let mut new = PageState::new(Platform::Gemini, 1);
        new.event(Navigation::Completed {
            id: 9,
            success: true,
            error: 0,
        });
        assert_eq!(new.snapshot()["phase"], "pending");
        assert!(new.event(Navigation::Starting {
            id: 10,
            url: Platform::Gemini.url().parse().unwrap()
        }));
        new.event(Navigation::Completed {
            id: 9,
            success: true,
            error: 0,
        });
        assert_eq!(new.snapshot()["phase"], "loading");
    }
    #[test]
    fn assistant_webviews_initial_blank_does_not_bind_or_complete_a_real_service() {
        let mut p = PageState::new(Platform::Deepseek, 0);
        assert!(p.event(Navigation::Starting {
            id: 0,
            url: "about:blank".parse().unwrap()
        }));
        p.event(Navigation::Completed {
            id: 0,
            success: true,
            error: 0,
        });
        assert_eq!(p.snapshot()["phase"], "pending");
        assert!(!p.event(Navigation::Starting {
            id: 2,
            url: "https://other.test/".parse().unwrap()
        }));
    }
    #[derive(Default)]
    struct Fake {
        events: Vec<String>,
        live: HashSet<String>,
        fail_create: bool,
        fail_close: bool,
        fail_auxiliary: bool,
        after_create: Option<Arc<AtomicBool>>,
        after_place: Option<Arc<AtomicBool>>,
    }
    impl Driver for Fake {
        fn create(&mut self, c: &Child) -> AppResult<()> {
            self.events
                .push(format!("create:{}:{}", c.label(), c.platform.url()));
            self.live.insert(c.label());
            if let Some(current) = &self.after_create {
                current.store(false, Ordering::Release);
            }
            if self.fail_create {
                self.fail_create = false;
                return Err("NATIVE_ERROR: 创建结果未确定".into());
            }
            Ok(())
        }
        fn navigate(&mut self, c: &Child) -> AppResult<()> {
            self.events
                .push(format!("navigate:{}:{}", c.label(), c.platform.url()));
            Ok(())
        }
        fn place(&mut self, c: &Child) -> AppResult<()> {
            self.events.push(format!("place:{}", c.label()));
            if let Some(current) = &self.after_place {
                current.store(false, Ordering::Release);
            }
            Ok(())
        }
        fn visible(&mut self, l: &str, v: bool) -> AppResult<()> {
            self.events.push(format!("visible:{l}:{v}"));
            Ok(())
        }
        fn close(&mut self, l: &str) -> AppResult<()> {
            self.events.push(format!("close:{l}"));
            if self.fail_close {
                return Err("NATIVE_ERROR: 关闭失败".into());
            }
            self.live.remove(l);
            Ok(())
        }
        fn close_auxiliary(&mut self, id: &str) -> AppResult<()> {
            self.events.push(format!("aux:{id}"));
            if self.fail_auxiliary {
                return Err("NATIVE_ERROR: 登录弹窗未确认关闭".into());
            }
            Ok(())
        }
    }
    fn reconcile(
        c: &mut Controller,
        o: &Owner,
        cfg: &Config,
        io: &mut Fake,
        visible: bool,
        overlay: bool,
    ) -> AppResult<Attached> {
        c.reconcile(
            &root(),
            o,
            cfg,
            (600.0, 900.0),
            visible,
            overlay,
            io,
            || true,
        )
    }
    #[test]
    fn assistant_webviews_original_vertical_two_and_four_grid_leave_selector_space() {
        for count in [1, 2, 4] {
            let rects = layout(count, 600.0, 900.0).unwrap();
            assert_eq!(rects.len(), count as usize);
            for r in &rects {
                assert!(r.x >= 12.0 && r.y >= 56.0 && r.width > 0.0 && r.height > 0.0);
                assert!(r.x + r.width <= 588.0 && r.y + r.height <= 848.0);
            }
            if count == 2 {
                assert_eq!(rects[0].x, rects[1].x);
                assert!(rects[1].y > rects[0].y + rects[0].height + 40.0);
            }
            if count == 4 {
                assert_eq!(rects[0].y, rects[1].y);
                assert_eq!(rects[2].x, rects[0].x);
            }
        }
        for size in [(f64::NAN, 900.0), (600.0, f64::INFINITY), (100.0, 900.0)] {
            assert!(layout(4, size.0, size.1).is_err());
        }
    }
    #[test]
    fn assistant_webviews_remote_navigation_never_enters_local_host_or_unsafe_schemes() {
        for uri in [
            "https://accounts.google.com/login",
            "https://www.kimi.com/",
            "about:blank",
        ] {
            assert!(web_navigation_allowed(&uri.parse().unwrap()));
        }
        for uri in [
            "http://tauri.localhost/",
            "https://tauri.localhost/",
            "https://widget.localhost/",
            "https://asset.localhost/private",
            "https://user:pass@example.com/",
            "file:///C:/private",
            "data:text/html,hello",
            "javascript:alert(1)",
            "tauri://localhost",
            "http://widget.localhost/",
            "ipc://localhost/",
        ] {
            assert!(!web_navigation_allowed(&uri.parse().unwrap()), "{uri}");
        }
    }
    #[test]
    fn assistant_webviews_hidden_defaults_or_never_opened_background_do_not_create_pages() {
        let mut c = Controller::default();
        let o = owner(None);
        let mut io = Fake::default();
        let mut cfg = Config::default();
        assert_eq!(
            reconcile(&mut c, &o, &cfg, &mut io, false, false)
                .unwrap()
                .attached,
            0
        );
        cfg.background_enabled = true;
        assert_eq!(
            reconcile(&mut c, &o, &cfg, &mut io, false, false)
                .unwrap()
                .attached,
            0
        );
        assert!(io.live.is_empty());
        assert!(!io.events.iter().any(|e| e.starts_with("create:")));
    }
    #[test]
    fn assistant_webviews_initial_pages_use_fixed_platforms_and_shared_instance_profile() {
        let mut c = Controller::default();
        let o = owner(None);
        let mut io = Fake::default();
        let mut cfg = Config::default();
        cfg.page_count = 4;
        let v = reconcile(&mut c, &o, &cfg, &mut io, true, false).unwrap();
        assert_eq!((v.attached, v.shown), (4, 4));
        let paths = c
            .children
            .values()
            .map(|v| v.profile.clone())
            .collect::<HashSet<_>>();
        assert_eq!(paths.len(), 1);
        assert!(paths.iter().next().unwrap().ends_with(&o.instance));
        assert!(io
            .events
            .iter()
            .any(|e| e.ends_with("https://chatgpt.com/")));
        assert!(io.events.iter().any(|e| e.ends_with("https://grok.com/")));
    }
    #[test]
    fn assistant_webviews_resize_or_layout_keeps_unchanged_pages_and_platform_switch_only_navigates_its_slot(
    ) {
        let mut c = Controller::default();
        let o = owner(None);
        let mut io = Fake::default();
        let mut cfg = Config::default();
        reconcile(&mut c, &o, &cfg, &mut io, true, false).unwrap();
        io.events.clear();
        cfg.page_count = 2;
        reconcile(&mut c, &o, &cfg, &mut io, true, false).unwrap();
        assert_eq!(
            io.events
                .iter()
                .filter(|e| e.starts_with("create:"))
                .count(),
            1
        );
        assert_eq!(
            io.events
                .iter()
                .filter(|e| e.starts_with("navigate:"))
                .count(),
            1
        );
        assert!(!io
            .events
            .iter()
            .any(|e| e.starts_with("navigate:") && e.contains(&o.label(0))));
        io.events.clear();
        cfg.platforms[0] = Platform::Kimi;
        reconcile(&mut c, &o, &cfg, &mut io, true, false).unwrap();
        assert_eq!(
            io.events
                .iter()
                .filter(|e| e.starts_with("navigate:"))
                .count(),
            1
        );
        assert!(io
            .events
            .iter()
            .any(|e| e.contains(&o.label(0)) && e.starts_with("navigate:")));
        assert!(!io.events.iter().any(|e| e.starts_with("create:")));
    }
    #[test]
    fn assistant_webviews_proxy_rebuild_closes_all_before_create_and_keeps_profile_path_and_other_instance(
    ) {
        let mut c = Controller::default();
        let a = owner(None);
        let b = owner(None);
        let mut io = Fake::default();
        let mut cfg = Config::default();
        cfg.page_count = 2;
        reconcile(&mut c, &a, &cfg, &mut io, true, false).unwrap();
        reconcile(&mut c, &b, &cfg, &mut io, true, false).unwrap();
        let profile = c.children[&a.label(0)].profile.clone();
        io.events.clear();
        cfg.proxy = Proxy::Manual {
            protocol: assistant_state::Protocol::Socks4,
            host: "127.0.0.1".into(),
            port: 1080,
        };
        reconcile(&mut c, &a, &cfg, &mut io, true, false).unwrap();
        let first_create = io
            .events
            .iter()
            .position(|e| e.starts_with("create:"))
            .unwrap();
        assert_eq!(
            io.events[..first_create]
                .iter()
                .filter(|e| e.starts_with("close:"))
                .count(),
            2
        );
        assert_eq!(c.children[&a.label(0)].profile, profile);
        assert!(io.live.contains(&b.label(0)));
        assert!(!io.events.iter().any(|e| e.contains(&b.instance)));
    }
    #[test]
    fn assistant_webviews_overlay_and_background_hide_reuse_but_default_hide_releases() {
        let mut c = Controller::default();
        let o = owner(None);
        let mut io = Fake::default();
        let mut cfg = Config::default();
        reconcile(&mut c, &o, &cfg, &mut io, true, false).unwrap();
        io.events.clear();
        assert_eq!(
            reconcile(&mut c, &o, &cfg, &mut io, true, true)
                .unwrap()
                .shown,
            0
        );
        assert_eq!(io.live.len(), 1);
        assert_eq!(
            reconcile(&mut c, &o, &cfg, &mut io, true, false)
                .unwrap()
                .shown,
            1
        );
        assert!(!io.events.iter().any(|e| e.starts_with("create:")));
        cfg.background_enabled = true;
        assert_eq!(
            reconcile(&mut c, &o, &cfg, &mut io, false, false)
                .unwrap()
                .attached,
            1
        );
        cfg.background_enabled = false;
        assert_eq!(
            reconcile(&mut c, &o, &cfg, &mut io, false, false)
                .unwrap()
                .attached,
            0
        );
        assert!(io.live.is_empty());
    }
    #[test]
    fn assistant_webviews_owner_change_during_create_cleans_hidden_view_without_showing_or_touching_other_instance(
    ) {
        let mut c = Controller::default();
        let a = owner(None);
        let b = owner(None);
        let mut io = Fake::default();
        reconcile(&mut c, &b, &Config::default(), &mut io, true, false).unwrap();
        io.events.clear();
        let current = Arc::new(AtomicBool::new(true));
        io.after_create = Some(current.clone());
        assert!(c
            .reconcile(
                &root(),
                &a,
                &Config::default(),
                (600.0, 900.0),
                true,
                false,
                &mut io,
                || current.load(Ordering::Acquire)
            )
            .is_err());
        assert!(!io.live.contains(&a.label(0)));
        assert!(io.live.contains(&b.label(0)));
        assert!(!io.events.iter().any(|e| e.ends_with(":true")));
    }
    #[test]
    fn assistant_webviews_uncertain_create_and_failed_cleanup_require_closing_before_retry() {
        let mut c = Controller::default();
        let o = owner(None);
        let mut io = Fake {
            fail_create: true,
            fail_close: true,
            ..Default::default()
        };
        assert!(reconcile(&mut c, &o, &Config::default(), &mut io, true, false).is_err());
        assert_eq!(io.live.len(), 1);
        assert!(c.children[&o.label(0)].uncertain);
        io.events.clear();
        io.fail_close = false;
        reconcile(&mut c, &o, &Config::default(), &mut io, true, false).unwrap();
        let close = io
            .events
            .iter()
            .position(|e| e.starts_with("close:"))
            .unwrap();
        let create = io
            .events
            .iter()
            .position(|e| e.starts_with("create:"))
            .unwrap();
        assert!(close < create);
        assert_eq!(io.live.len(), 1);
    }
    #[test]
    fn assistant_webviews_new_layout_during_placement_hides_but_preserves_established_pages() {
        let mut c = Controller::default();
        let o = owner(None);
        let mut io = Fake::default();
        reconcile(&mut c, &o, &Config::default(), &mut io, true, false).unwrap();
        io.events.clear();
        let current = Arc::new(AtomicBool::new(true));
        io.after_place = Some(current.clone());
        assert!(c
            .reconcile(
                &root(),
                &o,
                &Config::default(),
                (800., 900.),
                true,
                false,
                &mut io,
                || current.load(Ordering::Acquire)
            )
            .is_err());
        assert!(io.live.contains(&o.label(0)), "新布局不应销毁已载入的网页");
        assert!(io
            .events
            .iter()
            .any(|e| e == &format!("visible:{}:false", o.label(0))));
        io.after_place = None;
        io.events.clear();
        reconcile(&mut c, &o, &Config::default(), &mut io, true, false).unwrap();
        assert!(!io
            .events
            .iter()
            .any(|e| e.starts_with("create:") || e.starts_with("navigate:")));
        assert!(io
            .events
            .iter()
            .any(|e| e == &format!("visible:{}:true", o.label(0))));
    }
    #[test]
    fn assistant_webviews_stale_plan_invalid_bounds_or_forged_parent_have_no_native_effect() {
        let mut c = Controller::default();
        let mut o = owner(None);
        let mut io = Fake::default();
        let local = o.shell_profile(&root()).unwrap();
        assert_eq!(local, root().join("webview").join(o.shell()));
        assert_ne!(
            local,
            assistant_state::profile_directory(&root(), &o.instance).unwrap()
        );
        assert_ne!(
            local,
            o.shell_profile(&root().join("another-isolated-root"))
                .unwrap()
        );
        assert!(c
            .reconcile(
                &root(),
                &o,
                &Config::default(),
                (600.0, 900.0),
                true,
                false,
                &mut io,
                || false
            )
            .is_err());
        o.parent = "main".into();
        assert!(o.shell_profile(&root()).is_err());
        assert!(reconcile(&mut c, &o, &Config::default(), &mut io, true, false).is_err());
        assert!(io.events.is_empty());
    }
    #[test]
    fn assistant_webviews_failed_auxiliary_close_blocks_proxy_profile_reuse_until_retry_confirms_release(
    ) {
        let mut c = Controller::default();
        let o = owner(None);
        let mut io = Fake::default();
        let mut cfg = Config::default();
        reconcile(&mut c, &o, &cfg, &mut io, true, false).unwrap();
        cfg.proxy = Proxy::Manual {
            protocol: assistant_state::Protocol::Http,
            host: "127.0.0.1".into(),
            port: 7890,
        };
        io.fail_auxiliary = true;
        io.events.clear();
        assert!(reconcile(&mut c, &o, &cfg, &mut io, true, false).is_err());
        assert!(!io.events.iter().any(|e| e.starts_with("create:")));
        io.events.clear();
        assert!(reconcile(&mut c, &o, &cfg, &mut io, true, false).is_err());
        assert!(!io.events.iter().any(|e| e.starts_with("create:")));
        io.fail_auxiliary = false;
        io.events.clear();
        assert_eq!(
            reconcile(&mut c, &o, &cfg, &mut io, true, false)
                .unwrap()
                .shown,
            1
        );
        let create = io
            .events
            .iter()
            .position(|e| e.starts_with("create:"))
            .unwrap();
        assert!(io.events[..create].iter().any(|e| e.starts_with("aux:")));
    }
    #[test]
    fn assistant_webviews_release_only_its_instance_and_replacement_does_not_keep_old_page_labels()
    {
        let mut c = Controller::default();
        let a = owner(None);
        let b = owner(None);
        let mut io = Fake::default();
        reconcile(&mut c, &a, &Config::default(), &mut io, true, false).unwrap();
        reconcile(&mut c, &b, &Config::default(), &mut io, true, false).unwrap();
        let new = owner(Some(&a.instance));
        reconcile(&mut c, &new, &Config::default(), &mut io, true, false).unwrap();
        assert!(!io.live.contains(&a.label(0)));
        assert!(io.live.contains(&new.label(0)));
        c.release(&a.instance, &mut io).unwrap();
        assert!(io.live.contains(&b.label(0)));
        assert!(!io.live.contains(&new.label(0)));
    }
}
