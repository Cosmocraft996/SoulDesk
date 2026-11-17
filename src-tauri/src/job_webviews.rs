//! AI直聘固定网页的身份、导航和采集凭据。网页没有宿主令牌或任意脚本接口。
use crate::{
    job_protocol,
    store::{err, AppResult},
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

const EXPIRED: &str = "PRESENTATION_EXPIRED: AI直聘网页已关闭、跳转或替换";
fn canonical(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Owner {
    pub instance: String,
    pub presentation: String,
    pub digest: String,
}
impl Owner {
    pub(crate) fn validate(&self) -> AppResult<()> {
        if !canonical(&self.instance)
            || !canonical(&self.presentation)
            || self.digest.len() != 64
            || !self.digest.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("INVALID_ARGUMENT: AI直聘网页归属无效".into());
        }
        Ok(())
    }
    pub(crate) fn parent(&self) -> String {
        format!("job-panel-{}-{}", self.instance, self.presentation)
    }
    pub(crate) fn shell(&self) -> String {
        format!("job-shell-{}-{}", self.instance, self.presentation)
    }
    pub(crate) fn boss(&self) -> String {
        format!("job-boss-{}-{}", self.instance, self.presentation)
    }
    pub(crate) fn profile(&self, root: &Path) -> AppResult<PathBuf> {
        self.validate()?;
        Ok(root.join("job-webviews").join(&self.instance))
    }
    pub(crate) fn shell_profile(&self, root: &Path) -> AppResult<PathBuf> {
        self.validate()?;
        Ok(root.join("webview").join(self.shell()))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Script {
    Login,
    Capture,
}
impl Script {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Login => job_protocol::LOGIN_SCRIPT,
            Self::Capture => job_protocol::CAPTURE_SCRIPT,
        }
    }
}
#[derive(Clone, Debug)]
struct Navigation {
    id: u64,
    serial: u64,
    url: url::Url,
    success: Option<bool>,
}
#[derive(Clone)]
struct View {
    owner: Owner,
    tag: String,
    serial: u64,
    request: String,
    navigation: Option<Navigation>,
    fault: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct Ticket {
    owner: Owner,
    tag: String,
    request: String,
    serial: u64,
    navigation: u64,
    pub script: Script,
}
#[derive(Default)]
pub(crate) struct Book {
    views: HashMap<String, View>,
}
impl Book {
    pub(crate) fn attach(&mut self, owner: Owner, tag: String) -> AppResult<()> {
        owner.validate()?;
        if !canonical(&tag) {
            return Err("INVALID_ARGUMENT: 网页身份无效".into());
        }
        if self.views.contains_key(&owner.instance) {
            return Err("BUSY: 旧AI直聘网页尚未撤销".into());
        }
        self.views.insert(
            owner.instance.clone(),
            View {
                owner,
                tag,
                serial: 0,
                request: uuid::Uuid::new_v4().to_string(),
                navigation: None,
                fault: false,
            },
        );
        Ok(())
    }
    pub(crate) fn owns(&self, owner: &Owner, tag: &str) -> bool {
        self.views
            .get(&owner.instance)
            .is_some_and(|v| v.owner == *owner && v.tag == tag && !v.fault)
    }
    pub(crate) fn owner(&self, instance: &str) -> Option<Owner> {
        self.views
            .get(instance)
            .filter(|v| !v.fault)
            .map(|v| v.owner.clone())
    }
    pub(crate) fn detach(&mut self, owner: &Owner) {
        if self
            .views
            .get(&owner.instance)
            .is_some_and(|v| v.owner == *owner)
        {
            self.views.remove(&owner.instance);
        }
    }
    pub(crate) fn fail(&mut self, owner: &Owner) {
        if let Some(v) = self
            .views
            .get_mut(&owner.instance)
            .filter(|v| v.owner == *owner)
        {
            v.fault = true;
        }
    }
    fn view(&self, owner: &Owner) -> AppResult<&View> {
        self.views
            .get(&owner.instance)
            .filter(|v| v.owner == *owner && !v.fault)
            .ok_or_else(|| EXPIRED.into())
    }
    pub(crate) fn requested(&mut self, owner: &Owner) -> AppResult<String> {
        let v = self
            .views
            .get_mut(&owner.instance)
            .filter(|v| v.owner == *owner && !v.fault)
            .ok_or(EXPIRED)?;
        v.request = uuid::Uuid::new_v4().to_string();
        v.navigation = None;
        v.serial = v
            .serial
            .checked_add(1)
            .ok_or("LIMIT: 网页导航版本达到上限")?;
        Ok(v.request.clone())
    }
    pub(crate) fn starting(&mut self, owner: &Owner, tag: &str, id: u64, url: url::Url) -> bool {
        if !job_protocol::site_navigation(&url) || !self.owns(owner, tag) {
            return false;
        }
        let v = self.views.get_mut(&owner.instance).expect("网页归属已核对");
        let Some(serial) = v.serial.checked_add(1) else {
            v.fault = true;
            return false;
        };
        v.serial = serial;
        v.navigation = Some(Navigation {
            id,
            serial,
            url,
            success: None,
        });
        true
    }
    pub(crate) fn completed(&mut self, owner: &Owner, tag: &str, id: u64, success: bool) {
        if !self.owns(owner, tag) {
            return;
        }
        let v = self.views.get_mut(&owner.instance).expect("网页归属已核对");
        if let Some(n) = v.navigation.as_mut().filter(|n| n.id == id) {
            n.success = Some(success);
        }
    }
    pub(crate) fn loaded(&self, owner: &Owner, request: &str) -> AppResult<Option<url::Url>> {
        let v = self.view(owner)?;
        if v.request != request {
            return Err(EXPIRED.into());
        }
        match &v.navigation {
            Some(n) if n.success == Some(true) => Ok(Some(n.url.clone())),
            Some(n) if n.success == Some(false) => {
                Err("NAVIGATION_FAILED: BOSS页面载入失败，请手动检查网络或登录".into())
            }
            _ => Ok(None),
        }
    }
    pub(crate) fn request(&self, owner: &Owner) -> AppResult<String> {
        Ok(self.view(owner)?.request.clone())
    }
    pub(crate) fn ticket(
        &self,
        owner: &Owner,
        script: Script,
        expected: Option<(&str, u32)>,
    ) -> AppResult<Ticket> {
        let v = self.view(owner)?;
        let n = v
            .navigation
            .as_ref()
            .filter(|n| n.success == Some(true))
            .ok_or("DATA_UNAVAILABLE: 请等待BOSS页面载入完成")?;
        if n.url.origin().ascii_serialization() != "https://www.zhipin.com" {
            return Err(EXPIRED.into());
        }
        if script == Script::Capture {
            let (keyword, page) = expected.ok_or("INVALID_ARGUMENT: 固定采集缺少当前搜索身份")?;
            if !search_identity(&n.url, keyword, page) {
                return Err(EXPIRED.into());
            }
        }
        Ok(Ticket {
            owner: owner.clone(),
            tag: v.tag.clone(),
            request: v.request.clone(),
            serial: n.serial,
            navigation: n.id,
            script,
        })
    }
    pub(crate) fn validate(&self, ticket: &Ticket) -> AppResult<()> {
        let v = self.view(&ticket.owner)?;
        if v.tag != ticket.tag
            || v.request != ticket.request
            || !v.navigation.as_ref().is_some_and(|n| {
                n.id == ticket.navigation && n.serial == ticket.serial && n.success == Some(true)
            })
        {
            return Err(EXPIRED.into());
        }
        Ok(())
    }
    pub(crate) fn status(&self, instance: &str) -> Value {
        match self.views.get(instance) {
            None => json!({"attached":false}),
            Some(v) => {
                json!({"attached":true,"fault":v.fault,"phase":match &v.navigation{None=>"pending",Some(n)=>match n.success{None=>"loading",Some(true)=>"loaded",Some(false)=>"failed"}}})
            }
        }
    }
}
pub(crate) fn search_identity(url: &url::Url, keyword: &str, page: u32) -> bool {
    if url.origin().ascii_serialization() != "https://www.zhipin.com"
        || url.path() != "/web/geek/job"
        || url.fragment().is_some()
    {
        return false;
    }
    let pairs = url.query_pairs().collect::<Vec<_>>();
    let keys = pairs
        .iter()
        .filter(|(k, _)| k == "query")
        .collect::<Vec<_>>();
    let pages = pairs
        .iter()
        .filter(|(k, _)| k == "page")
        .collect::<Vec<_>>();
    keys.len() == 1 && keys[0].1 == keyword && pages.len() == 1 && pages[0].1 == page.to_string()
}
pub(crate) fn valid_rect(rect: (f64, f64, f64, f64), area: (f64, f64)) -> bool {
    let (x, y, width, height) = rect;
    [x, y, width, height, area.0, area.1]
        .iter()
        .all(|n| n.is_finite())
        && x >= 0.
        && y >= 0.
        && width >= 160.
        && height >= 160.
        && area.0 <= 16384.
        && area.1 <= 16384.
        && x + width <= area.0
        && y + height <= area.1
}
pub(crate) type Shared = Arc<Mutex<Book>>;
pub(crate) fn commit<T>(
    book: &Shared,
    ticket: &Ticket,
    action: impl FnOnce() -> AppResult<T>,
) -> AppResult<T> {
    let current = book.lock().map_err(err)?;
    current.validate(ticket)?;
    action()
}

#[cfg(windows)]
pub(crate) mod native {
    use super::*;
    use std::time::Duration;
    use tauri::{
        webview::{NewWindowResponse, WebviewBuilder},
        Manager, WebviewUrl,
    };
    use webview2_com::{
        ExecuteScriptCompletedHandler,
        Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
        NavigationCompletedEventHandler, NavigationStartingEventHandler,
    };
    #[derive(Clone)]
    pub(crate) struct Native {
        pub app: tauri::AppHandle,
        pub book: Shared,
    }
    impl Native {
        /// 调用方先建立本机父窗口。远程视图不注入宿主脚本、令牌或IPC。
        pub(crate) fn create(&self, owner: &Owner, root: &Path) -> AppResult<()> {
            owner.validate()?;
            let parent = self.app.get_window(&owner.parent()).ok_or(EXPIRED)?;
            if self.app.get_webview(&owner.boss()).is_some() {
                return Err("BUSY: 旧BOSS网页释放尚未确认".into());
            }
            let profile = owner.profile(root)?;
            std::fs::create_dir_all(&profile).map_err(err)?;
            let tag = uuid::Uuid::new_v4().to_string();
            self.book
                .lock()
                .map_err(err)?
                .attach(owner.clone(), tag.clone())?;
            let builder = WebviewBuilder::new(
                owner.boss(),
                WebviewUrl::External("about:blank".parse().map_err(err)?),
            )
            .data_directory(profile)
            .focused(false)
            .devtools(false)
            .on_navigation(|url| {
                url.as_str() == "about:blank" || job_protocol::site_navigation(url)
            })
            // 职位详情由受限宿主动作打开；远程页不能自行创建有权限的窗口。
            .on_new_window(|_, _| NewWindowResponse::Deny);
            let result = (|| {
                let view = parent
                    .add_child(
                        builder,
                        tauri::LogicalPosition::new(-10000., -10000.),
                        tauri::LogicalSize::new(640., 480.),
                    )
                    .map_err(err)?;
                view.hide().map_err(err)?;
                self.events(&view, owner.clone(), tag)?;
                Ok(())
            })();
            if result.is_err() {
                self.book.lock().map_err(err)?.fail(owner);
                if let Some(v) = self.app.get_webview(&owner.boss()) {
                    let _ = v.hide();
                    let _ = v.close();
                }
            }
            result
        }
        fn events(&self, view: &tauri::Webview, owner: Owner, tag: String) -> AppResult<()> {
            let book = self.book.clone();
            let (send, receive) = std::sync::mpsc::sync_channel(1);
            view.with_webview(move |platform| {
                let result: windows::core::Result<()> = (|| unsafe {
                    let webview = platform.controller().CoreWebView2()?;
                    let start_book = book.clone();
                    let start_owner = owner.clone();
                    let start_tag = tag.clone();
                    let mut starting = 0;
                    webview.add_NavigationStarting(
                        &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                            let Some(args) = args else {
                                return Ok(());
                            };
                            let mut id = 0;
                            args.NavigationId(&mut id)?;
                            let mut uri = windows::core::PWSTR::null();
                            let result = args.Uri(&mut uri);
                            let owned = webview2_com::CoTaskMemPWSTR::from(uri);
                            result?;
                            let allow = url::Url::parse(&owned.to_string()).is_ok_and(|url| {
                                start_book.lock().is_ok_and(|mut b| {
                                    b.starting(&start_owner, &start_tag, id, url)
                                })
                            });
                            args.SetCancel(!allow)?;
                            Ok(())
                        })),
                        &mut starting,
                    )?;
                    let mut completed = 0;
                    let result = webview.add_NavigationCompleted(
                        &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                            let Some(args) = args else {
                                return Ok(());
                            };
                            let mut id = 0;
                            let mut success = windows::core::BOOL::default();
                            let mut code = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
                            args.NavigationId(&mut id)?;
                            args.IsSuccess(&mut success)?;
                            args.WebErrorStatus(&mut code)?;
                            if let Ok(mut b) = book.lock() {
                                b.completed(&owner, &tag, id, success.as_bool());
                            }
                            Ok(())
                        })),
                        &mut completed,
                    );
                    if result.is_err() {
                        let _ = webview.remove_NavigationStarting(starting);
                    }
                    result
                })();
                let _ =
                    send.send(result.map_err(|_| "NATIVE_ERROR: BOSS导航监听未建立".to_owned()));
            })
            .map_err(err)?;
            receive
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| "NATIVE_ERROR: BOSS导航监听状态未确定")?
        }
        pub(crate) fn navigate(&self, owner: &Owner, url: url::Url) -> AppResult<String> {
            if !job_protocol::site_navigation(&url) {
                return Err("CAPABILITY_DENIED: BOSS仅允许固定站点HTTPS导航".into());
            }
            let request = self.book.lock().map_err(err)?.requested(owner)?;
            self.app
                .get_webview(&owner.boss())
                .ok_or(EXPIRED)?
                .navigate(url)
                .map_err(|_| "NAVIGATION_FAILED: BOSS导航未开始")?;
            Ok(request)
        }
        pub(crate) async fn wait(
            &self,
            owner: &Owner,
            request: &str,
            cancel: &CancellationToken,
        ) -> AppResult<url::Url> {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            loop {
                if cancel.is_cancelled() {
                    return Err("CANCELLED".into());
                }
                if let Some(url) = self.book.lock().map_err(err)?.loaded(owner, request)? {
                    return Ok(url);
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err("TIMEOUT: BOSS页面载入超时，未继续采集或评分".into());
                }
                tokio::select! {biased;_=cancel.cancelled()=>return Err("CANCELLED".into()),_=tokio::time::sleep(Duration::from_millis(100))=>()}
            }
        }
        pub(crate) async fn evaluate(
            &self,
            ticket: Ticket,
            cancel: &CancellationToken,
        ) -> AppResult<Value> {
            self.book.lock().map_err(err)?.validate(&ticket)?;
            let view = self.app.get_webview(&ticket.owner.boss()).ok_or(EXPIRED)?;
            let (send, receive) = tokio::sync::oneshot::channel::<AppResult<Value>>();
            let send = Arc::new(Mutex::new(Some(send)));
            let fail = send.clone();
            let book = self.book.clone();
            let check = ticket.clone();
            view.with_webview(move |platform| {
                let result: windows::core::Result<()> = (|| unsafe {
                    let webview = platform.controller().CoreWebView2()?;
                    let code = webview2_com::CoTaskMemPWSTR::from(ticket.script.code());
                    webview.ExecuteScript(
                        *code.as_ref().as_pcwstr(),
                        &ExecuteScriptCompletedHandler::create(Box::new(move |status, text| {
                            let value = (|| -> AppResult<Value> {
                                status.map_err(|_| "NATIVE_ERROR: BOSS固定采集执行失败")?;
                                book.lock().map_err(err)?.validate(&check)?;
                                if text.len() > 2 * 1024 * 1024 {
                                    return Err(
                                        "CAPACITY_EXCEEDED: BOSS固定采集结果超过2MiB".into()
                                    );
                                }
                                serde_json::from_str(&text)
                                    .map_err(|_| "DATA_UNAVAILABLE: BOSS固定采集未返回JSON".into())
                            })();
                            if let Ok(mut s) = send.lock() {
                                if let Some(s) = s.take() {
                                    let _ = s.send(value);
                                }
                            }
                            Ok(())
                        })),
                    )
                })();
                if result.is_err() {
                    if let Ok(mut s) = fail.lock() {
                        if let Some(s) = s.take() {
                            let _ = s.send(Err("NATIVE_ERROR: BOSS固定采集未开始".into()));
                        }
                    }
                }
            })
            .map_err(|_| "NATIVE_ERROR: BOSS固定采集调度失败")?;
            tokio::select! {biased;_=cancel.cancelled()=>Err("CANCELLED".into()),result=tokio::time::timeout(Duration::from_secs(10),receive)=>result.map_err(|_|"TIMEOUT: BOSS固定采集结果未确定")?.map_err(|_|"NATIVE_ERROR: BOSS固定采集回调已关闭")?}
        }
        pub(crate) fn place(
            &self,
            owner: &Owner,
            rect: (f64, f64, f64, f64),
            visible: bool,
        ) -> AppResult<()> {
            self.book.lock().map_err(err)?.view(owner)?;
            let (x, y, width, height) = rect;
            let parent = self.app.get_window(&owner.parent()).ok_or(EXPIRED)?;
            let size = parent
                .inner_size()
                .map_err(err)?
                .to_logical::<f64>(parent.scale_factor().map_err(err)?);
            if !valid_rect(rect, (size.width, size.height)) {
                return Err("INVALID_ARGUMENT: BOSS视图位置或尺寸无效".into());
            }
            let view = self.app.get_webview(&owner.boss()).ok_or(EXPIRED)?;
            view.set_bounds(tauri::Rect {
                position: tauri::LogicalPosition::new(x, y).into(),
                size: tauri::LogicalSize::new(width, height).into(),
            })
            .map_err(err)?;
            if visible { view.show() } else { view.hide() }.map_err(err)
        }
        pub(crate) fn close(&self, owner: &Owner) -> AppResult<()> {
            self.book.lock().map_err(err)?.fail(owner);
            if let Some(view) = self.app.get_webview(&owner.boss()) {
                view.hide().map_err(err)?;
                view.close().map_err(err)?;
            }
            if self.app.get_webview(&owner.boss()).is_some() {
                return Err("NATIVE_ERROR: BOSS网页释放尚未确认".into());
            }
            self.book.lock().map_err(err)?.detach(owner);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner() -> Owner {
        Owner {
            instance: uuid::Uuid::new_v4().to_string(),
            presentation: uuid::Uuid::new_v4().to_string(),
            digest: "a".repeat(64),
        }
    }
    fn ready() -> (Book, Owner, String) {
        let mut b = Book::default();
        let o = owner();
        let tag = uuid::Uuid::new_v4().to_string();
        b.attach(o.clone(), tag.clone()).unwrap();
        assert!(b.starting(&o, &tag, 1, job_protocol::search_url("Vue", 1).unwrap()));
        b.completed(&o, &tag, 1, true);
        (b, o, tag)
    }
    #[test]
    fn fixed_profiles_and_labels_cannot_escape_the_instance() {
        let o = owner();
        let root = Path::new("test-root");
        assert_eq!(
            o.profile(root).unwrap(),
            root.join("job-webviews").join(&o.instance)
        );
        assert_eq!(
            o.shell_profile(root).unwrap(),
            root.join("webview").join(o.shell())
        );
        assert_ne!(o.boss(), o.shell());
        let mut bad = o.clone();
        bad.instance = "../x".into();
        assert!(bad.validate().is_err());
        bad = o;
        bad.digest = "bad".into();
        assert!(bad.validate().is_err());
    }
    #[test]
    fn search_identity_checks_decoded_keyword_page_and_repeated_query_fields() {
        let u = job_protocol::search_url("C++ & 前端", 2).unwrap();
        assert!(search_identity(&u, "C++ & 前端", 2));
        for (url, key, page) in [
            (u.clone(), "C++", 2),
            (u.clone(), "C++ & 前端", 1),
            (
                "https://www.zhipin.com/web/geek/job?query=Vue&query=Rust&page=1"
                    .parse()
                    .unwrap(),
                "Vue",
                1,
            ),
            (
                "https://www.zhipin.com/web/geek/job?query=Vue&page=1&page=2"
                    .parse()
                    .unwrap(),
                "Vue",
                1,
            ),
            (
                "https://login.zhipin.com/web/geek/job?query=Vue&page=1"
                    .parse()
                    .unwrap(),
                "Vue",
                1,
            ),
        ] {
            assert!(!search_identity(&url, key, page));
        }
    }
    #[test]
    fn late_navigation_and_script_results_cannot_cross_a_new_navigation() {
        let (mut b, o, tag) = ready();
        let ticket = b.ticket(&o, Script::Capture, Some(("Vue", 1))).unwrap();
        assert!(b.validate(&ticket).is_ok());
        assert!(b.starting(&o, &tag, 2, job_protocol::search_url("Rust", 1).unwrap()));
        b.completed(&o, &tag, 1, true);
        assert_eq!(b.status(&o.instance)["phase"], "loading");
        assert!(b.validate(&ticket).is_err());
        b.completed(&o, &tag, 2, true);
        assert!(b.ticket(&o, Script::Capture, Some(("Vue", 1))).is_err());
        assert!(b.ticket(&o, Script::Capture, Some(("Rust", 1))).is_ok());
    }
    #[test]
    fn removed_replaced_or_failed_views_reject_old_tags_and_tickets() {
        let (mut b, o, tag) = ready();
        let t = b.ticket(&o, Script::Login, None).unwrap();
        b.detach(&o);
        let mut next = o.clone();
        next.presentation = uuid::Uuid::new_v4().to_string();
        let next_tag = uuid::Uuid::new_v4().to_string();
        b.attach(next.clone(), next_tag.clone()).unwrap();
        assert!(!b.starting(&o, &tag, 2, job_protocol::search_url("Vue", 1).unwrap()));
        assert!(b.validate(&t).is_err());
        b.completed(&o, &tag, 1, true);
        assert_eq!(b.status(&o.instance)["phase"], "pending");
        b.fail(&next);
        assert!(!b.owns(&next, &next_tag));
    }
    #[test]
    fn new_requests_invalidate_the_old_completion_even_before_navigation_starts() {
        let (mut b, o, _) = ready();
        let t = b.ticket(&o, Script::Login, None).unwrap();
        let old = b.request(&o).unwrap();
        let current = b.requested(&o).unwrap();
        assert_ne!(old, current);
        assert!(b.loaded(&o, &old).is_err());
        assert!(b.loaded(&o, &current).unwrap().is_none());
        assert!(b.validate(&t).is_err());
    }
    #[test]
    fn failed_or_offsite_navigation_is_not_a_ready_or_logged_in_page() {
        let (mut b, o, tag) = ready();
        assert!(!b.starting(&o, &tag, 2, "https://evil.example/".parse().unwrap()));
        assert!(!b.starting(&o, &tag, 2, "https://www.zhipin.com:444/".parse().unwrap()));
        assert!(!b.starting(&o, &tag, 2, "about:blank".parse().unwrap()));
        assert!(b.starting(&o, &tag, 3, job_protocol::search_url("Vue", 1).unwrap()));
        b.completed(&o, &tag, 3, false);
        assert!(b.loaded(&o, &b.request(&o).unwrap()).is_err());
        assert!(b.ticket(&o, Script::Login, None).is_err());
        assert!(b.status(&o.instance).get("loggedIn").is_none());
    }
    #[test]
    fn capture_identity_is_held_through_the_entire_database_commit() {
        let (b, o, tag) = ready();
        let ticket = b.ticket(&o, Script::Capture, Some(("Vue", 1))).unwrap();
        let shared = Arc::new(Mutex::new(b));
        assert_eq!(
            commit(&shared, &ticket, || {
                assert!(shared.try_lock().is_err());
                Ok(2)
            })
            .unwrap(),
            2
        );
        shared
            .lock()
            .unwrap()
            .starting(&o, &tag, 2, job_protocol::search_url("Rust", 1).unwrap());
        let mut called = false;
        assert!(commit(&shared, &ticket, || {
            called = true;
            Ok(())
        })
        .is_err());
        assert!(!called);
    }
    #[test]
    fn boss_bounds_stay_inside_the_current_client_area() {
        assert!(valid_rect((20., 80., 600., 400.), (900., 700.)));
        for rect in [
            (20., 80., 900., 400.),
            (-1., 0., 600., 400.),
            (20., 80., 100., 400.),
            (f64::NAN, 0., 600., 400.),
            (20., 80., 600., 1000.),
        ] {
            assert!(!valid_rect(rect, (900., 700.)));
        }
        assert!(!valid_rect((0., 0., 160., 160.), (f64::INFINITY, 700.)));
    }
}
