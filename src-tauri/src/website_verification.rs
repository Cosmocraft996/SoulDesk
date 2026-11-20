//! 网站收藏夹的临时浏览器。网页没有宿主消息桥，窗口菜单由宿主处理。
use crate::store::AppResult;
use serde_json::Value;
use tauri::AppHandle;

#[cfg(not(windows))]
#[derive(Default)]
pub(crate) struct State;

#[cfg(not(windows))]
pub(crate) async fn request(_: &AppHandle, _: &str, _: &str, _: Value) -> AppResult<Value> {
    Err("NOT_SUPPORTED: 当前平台不支持验证窗口".into())
}

#[cfg(windows)]
pub(crate) use native::{request, State};

#[cfg(windows)]
mod native {
    use super::*;
    use crate::{
        content::web_url,
        store::err,
        website_metadata::{clean_title, public_addresses, public_url},
        Core,
    };
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde_json::json;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tauri::{
        menu::{Menu, MenuItem},
        webview::NewWindowResponse,
        Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
    };
    use url::Url;
    use uuid::Uuid;
    use webview2_com::{
        CoTaskMemPWSTR, GetFaviconCompletedHandler, LaunchingExternalUriSchemeEventHandler,
        Microsoft::Web::WebView2::Win32::{
            ICoreWebView2NavigationCompletedEventArgs2, ICoreWebView2_15, ICoreWebView2_18,
            ICoreWebView2_2, COREWEBVIEW2_FAVICON_IMAGE_FORMAT_PNG,
            COREWEBVIEW2_PERMISSION_STATE_DENY,
        },
        NavigationCompletedEventHandler, NavigationStartingEventHandler,
        PermissionRequestedEventHandler, WebResourceResponseReceivedEventHandler,
    };
    use windows::core::{Interface, BOOL, PWSTR};

    const CAPTURE: &str = "website-verification-capture";
    const NAME: &str = "网页验证 · SoulDesk";

    #[derive(Clone, Default)]
    pub(crate) struct State(Arc<Mutex<Option<Visit>>>);

    struct Visit {
        id: Uuid,
        instance: String,
        navigation: u64,
        navigation_url: String,
        challenged: bool,
        page_ready: bool,
        reading: bool,
        result: Value,
    }

    impl State {
        fn pending(&self, id: Uuid) -> bool {
            self.update(id, |visit| visit.result["state"] == "pending") == Some(true)
        }

        fn update<T>(&self, id: Uuid, f: impl FnOnce(&mut Visit) -> T) -> Option<T> {
            let mut visits = self.0.lock().ok()?;
            let visit = visits.as_mut().filter(|visit| visit.id == id)?;
            Some(f(visit))
        }
    }

    fn label(id: Uuid) -> String {
        format!("website-verify-{id}")
    }

    pub(crate) async fn request(
        app: &AppHandle,
        instance: &str,
        method: &str,
        args: Value,
    ) -> AppResult<Value> {
        let fields = args
            .as_object()
            .ok_or("INVALID_ARGUMENT: 验证窗口参数无效")?;
        let id: Uuid = fields
            .get("id")
            .and_then(Value::as_str)
            .and_then(|value| value.parse().ok())
            .ok_or("INVALID_ARGUMENT: 验证编号无效")?;
        let core = app.state::<Core>();
        let state = core.website_verification.clone();
        if method == "links.verify.start" {
            if fields.len() != 2 {
                return Err("INVALID_ARGUMENT: 验证窗口参数无效".into());
            }
            let address = fields
                .get("url")
                .and_then(Value::as_str)
                .ok_or("INVALID_ARGUMENT: 缺少网站地址")?;
            let url = public_url(web_url(address)?)?;
            {
                let mut visits = state.0.lock().map_err(err)?;
                if visits
                    .as_ref()
                    .is_some_and(|visit| visit.result["state"] == "pending")
                {
                    return Err("BUSY: 请先完成或关闭当前验证窗口".into());
                }
                *visits = Some(Visit {
                    id,
                    instance: instance.into(),
                    navigation: 0,
                    navigation_url: String::new(),
                    challenged: false,
                    page_ready: false,
                    reading: false,
                    result: json!({"state":"pending"}),
                });
            }
            let opening = async {
                tokio::time::timeout(Duration::from_secs(2), public_addresses(&url))
                    .await
                    .map_err(|_| "DATA_UNAVAILABLE: 无法确认网站的公网地址")??;
                open(app, &state, id, url).await
            }
            .await;
            if let Err(error) = opening {
                close(app, &state, id);
                return Err(error);
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(600)).await;
                close(&app, &state, id);
            });
            return Ok(json!({"state":"pending"}));
        }
        if fields.len() != 1 {
            return Err("INVALID_ARGUMENT: 验证窗口参数无效".into());
        }
        let result = state
            .update(id, |visit| {
                (visit.instance == instance).then(|| visit.result.clone())
            })
            .flatten()
            .unwrap_or_else(|| json!({"state":"closed"}));
        if method == "links.verify.close" && result["state"] != "closed" {
            // 关闭请求同时丢弃未保存的结果。
            state.update(id, |visit| visit.result = json!({"state":"closed"}));
            close(app, &state, id);
            return Ok(json!({"state":"closed"}));
        }
        Ok(result)
    }

    fn close(app: &AppHandle, state: &State, id: Uuid) {
        state.update(id, |visit| {
            if visit.result["state"] == "pending" {
                visit.result = json!({"state":"closed"});
            }
        });
        if let Some(window) = app.get_webview_window(&label(id)) {
            let _ = window.close();
        }
    }

    async fn open(app: &AppHandle, state: &State, id: Uuid, url: Url) -> AppResult<()> {
        if !state.pending(id) {
            return Err("SESSION_EXPIRED: 验证已结束".into());
        }
        let profile = app
            .state::<Core>()
            .store
            .root
            .join("webview")
            .join("website-verification");
        let capture = MenuItem::with_id(app, CAPTURE, "完成验证并读取网站信息", true, None::<&str>)
            .map_err(err)?;
        let menu = Menu::with_items(app, &[&capture]).map_err(err)?;
        let target = url.origin();
        let source = url.origin();
        let capture_origin = url.origin();
        let event_state = state.clone();
        let window = WebviewWindowBuilder::new(
            app,
            label(id),
            WebviewUrl::External("about:blank".parse().map_err(err)?),
        )
        .title(NAME)
        .inner_size(840., 650.)
        .min_inner_size(480., 360.)
        .visible(false)
        .incognito(true)
        .data_directory(profile)
        .disable_drag_drop_handler()
        .devtools(false)
        .menu(menu)
        .on_navigation(move |address| {
            address.as_str() == "about:blank"
                || (address.origin() == target && public_url(address.clone()).is_ok())
        })
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_download(|_, _| false)
        .on_menu_event(move |owner, event| {
            if event.id().as_ref() == CAPTURE && owner.label() == label(id) {
                if let Some(window) = owner.app_handle().get_webview_window(owner.label()) {
                    read_current(&window, &event_state, id, capture_origin.clone());
                }
            }
        })
        .build()
        .map_err(err)?;
        let closed = state.clone();
        window.on_window_event(move |event| {
            if matches!(event, WindowEvent::Destroyed) {
                closed.update(id, |visit| {
                    if visit.result["state"] == "pending" {
                        visit.result = json!({"state":"closed"});
                    }
                });
            }
        });

        let (send, done) = tokio::sync::oneshot::channel();
        let bound = state.clone();
        window
            .with_webview(move |platform| {
                let result: windows::core::Result<()> = (|| unsafe {
                    let web = platform.controller().CoreWebView2()?;
                    let start = bound.clone();
                    let response_state = bound.clone();
                    web.add_NavigationStarting(
                        &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                            let Some(args) = args else {
                                return Ok(());
                            };
                            let mut nav = 0;
                            args.NavigationId(&mut nav)?;
                            let mut uri = PWSTR::null();
                            args.Uri(&mut uri)?;
                            let uri = CoTaskMemPWSTR::from(uri).to_string();
                            let valid = Url::parse(&uri)
                                .is_ok_and(|url| url.origin() == source && public_url(url).is_ok());
                            if uri != "about:blank" {
                                args.SetCancel(!valid)?;
                            }
                            if valid || uri == "about:blank" {
                                start.update(id, |visit| {
                                    visit.navigation = nav;
                                    visit.navigation_url = uri;
                                    visit.challenged = false;
                                    visit.page_ready = false;
                                    visit.reading = false;
                                });
                            }
                            Ok(())
                        })),
                        &mut 0,
                    )?;
                    web.add_NavigationCompleted(
                        &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                            let Some(args) = args else {
                                return Ok(());
                            };
                            let mut nav = 0;
                            let mut status = 0;
                            let mut success = BOOL::default();
                            args.NavigationId(&mut nav)?;
                            args.IsSuccess(&mut success)?;
                            args.cast::<ICoreWebView2NavigationCompletedEventArgs2>()?
                                .HttpStatusCode(&mut status)?;
                            bound.update(id, |visit| {
                                if visit.navigation == nav {
                                    visit.page_ready =
                                        success.as_bool() && (200..300).contains(&status);
                                }
                            });
                            Ok(())
                        })),
                        &mut 0,
                    )?;
                    web.cast::<ICoreWebView2_2>()?
                        .add_WebResourceResponseReceived(
                            &WebResourceResponseReceivedEventHandler::create(Box::new(
                                move |_, args| {
                                    let Some(args) = args else {
                                        return Ok(());
                                    };
                                    let mut address = PWSTR::null();
                                    args.Request()?.Uri(&mut address)?;
                                    let address = CoTaskMemPWSTR::from(address).to_string();
                                    if response_state
                                        .update(id, |visit| visit.navigation_url == address)
                                        != Some(true)
                                    {
                                        return Ok(());
                                    }
                                    let mut value = PWSTR::null();
                                    let status = args
                                        .Response()?
                                        .Headers()?
                                        .GetHeader(windows::core::w!("cf-mitigated"), &mut value);
                                    let value = CoTaskMemPWSTR::from(value);
                                    if status.is_ok()
                                        && value.to_string().eq_ignore_ascii_case("challenge")
                                    {
                                        response_state.update(id, |visit| {
                                            if visit.navigation_url == address {
                                                visit.challenged = true;
                                            }
                                        });
                                    }
                                    Ok(())
                                },
                            )),
                            &mut 0,
                        )?;
                    web.cast::<ICoreWebView2_18>()?
                        .add_LaunchingExternalUriScheme(
                            &LaunchingExternalUriSchemeEventHandler::create(Box::new(|_, args| {
                                if let Some(args) = args {
                                    args.SetCancel(true)?;
                                }
                                Ok(())
                            })),
                            &mut 0,
                        )?;
                    web.add_PermissionRequested(
                        &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                            if let Some(args) = args {
                                args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                            }
                            Ok(())
                        })),
                        &mut 0,
                    )?;
                    Ok(())
                })();
                let _ = send.send(result.map_err(err));
            })
            .map_err(err)?;
        tokio::time::timeout(Duration::from_secs(4), done)
            .await
            .map_err(|_| "NATIVE_ERROR: 验证窗口初始化超时")?
            .map_err(err)??;
        if !state.pending(id) {
            return Err("SESSION_EXPIRED: 验证已结束".into());
        }
        window.navigate(url).map_err(err)?;
        window.show().map_err(err)?;
        window.set_focus().map_err(err)
    }

    fn read_current(window: &WebviewWindow, state: &State, id: Uuid, origin: url::Origin) {
        let Some(Some(navigation)) = state.update(id, |visit| {
            if visit.result["state"] == "pending"
                && visit.page_ready
                && !visit.challenged
                && !visit.reading
            {
                visit.reading = true;
                Some(visit.navigation)
            } else {
                None
            }
        }) else {
            let _ = window.set_title("网页验证 · 页面就绪后再读取");
            return;
        };

        let book = state.clone();
        let target = window.clone();
        let started = window.with_webview(move |platform| {
            let result: windows::core::Result<()> = (|| unsafe {
                let web = platform.controller().CoreWebView2()?;
                let mut address = PWSTR::null();
                web.Source(&mut address)?;
                let address = CoTaskMemPWSTR::from(address).to_string();
                let mut title = PWSTR::null();
                web.DocumentTitle(&mut title)?;
                let title = clean_title(&CoTaskMemPWSTR::from(title).to_string());
                if !Url::parse(&address).is_ok_and(|url| url.origin() == origin) {
                    retry_capture(&target, &book, id, navigation);
                    return Ok(());
                }
                let capture_book = book.clone();
                let capture_window = target.clone();
                web.cast::<ICoreWebView2_15>()?.GetFavicon(
                    COREWEBVIEW2_FAVICON_IMAGE_FORMAT_PNG,
                    &GetFaviconCompletedHandler::create(Box::new(move |error, stream| {
                        let icon = if error.is_ok() {
                            stream.and_then(|stream| {
                                let mut bytes = vec![0_u8; 128 * 1024 + 1];
                                let mut count = 0;
                                let status = stream.Read(
                                    bytes.as_mut_ptr().cast(),
                                    bytes.len() as u32,
                                    Some(&mut count),
                                );
                                (status.is_ok()
                                    && (8..bytes.len() as u32).contains(&count)
                                    && bytes.starts_with(b"\x89PNG\r\n\x1a\n"))
                                .then(|| {
                                    format!(
                                        "data:image/png;base64,{}",
                                        STANDARD.encode(&bytes[..count as usize])
                                    )
                                })
                            })
                        } else {
                            None
                        };
                        if title.is_none() && icon.is_none() {
                            retry_capture(&capture_window, &capture_book, id, navigation);
                            return Ok(());
                        }
                        let saved = capture_book.update(id, |visit| {
                            if visit.navigation != navigation
                                || visit.challenged
                                || visit.result["state"] != "pending"
                            {
                                return false;
                            }
                            visit.result = json!({"state":"ready", "title":title, "icon":icon});
                            true
                        });
                        if saved == Some(true) {
                            let _ = capture_window.close();
                        }
                        Ok(())
                    })),
                )
            })();
            if result.is_err() {
                retry_capture(&target, &book, id, navigation);
            }
        });
        if started.is_err() {
            retry_capture(window, state, id, navigation);
        }
    }

    fn retry_capture(window: &WebviewWindow, state: &State, id: Uuid, navigation: u64) {
        state.update(id, |visit| {
            if visit.navigation == navigation {
                visit.reading = false;
            }
        });
        let _ = window.set_title("网页验证 · 页面就绪后再读取");
    }
}
