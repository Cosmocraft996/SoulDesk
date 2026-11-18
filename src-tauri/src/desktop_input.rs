//! 观察鼠标主键，保留原始事件的系统分发。命中测试在独立工作线程执行。
use crate::{desktop_native, store::err, windows as coordinator, Core};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, OnceLock,
};
use tauri::{AppHandle, Manager};
use windows::Win32::{
    Foundation::POINT as AccessiblePoint,
    System::{
        Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
        Variant::VARIANT,
    },
    UI::Accessibility::{AccessibleObjectFromPoint, ROLE_SYSTEM_CLIENT, ROLE_SYSTEM_LIST},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{PtInRect, ScreenToClient},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, GetDoubleClickTime, VK_CONTROL, VK_MENU, VK_SHIFT,
        },
        WindowsAndMessaging::*,
    },
};

static ENABLED: AtomicBool = AtomicBool::new(true);
static STOPPED: AtomicBool = AtomicBool::new(false);
static CLICKS: OnceLock<mpsc::SyncSender<Click>> = OnceLock::new();
#[derive(Clone, Copy)]
struct Click {
    x: i32,
    y: i32,
    time: u32,
    window: usize,
    scene: u64,
    unmodified: bool,
}

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Release);
}
pub struct Listener(u32);
impl Drop for Listener {
    fn drop(&mut self) {
        STOPPED.store(true, Ordering::Release);
        unsafe {
            PostThreadMessageW(self.0, WM_QUIT, 0, 0);
        }
    }
}
unsafe extern "system" fn mouse(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code >= 0 {
        let primary = if GetSystemMetrics(SM_SWAPBUTTON) != 0 {
            WM_RBUTTONDOWN
        } else {
            WM_LBUTTONDOWN
        };
        if message as u32 == primary {
            let e = &*(data as *const MSLLHOOKSTRUCT);
            let scene = coordinator::panel_scene();
            if e.flags & LLMHF_INJECTED == 0
                && (coordinator::scene_is_foreground(scene) || ENABLED.load(Ordering::Acquire))
            {
                if let Some(tx) = CLICKS.get() {
                    let _ = tx.try_send(Click {
                        x: e.pt.x,
                        y: e.pt.y,
                        time: e.time,
                        window: WindowFromPoint(e.pt) as usize,
                        scene,
                        unmodified: [VK_CONTROL, VK_MENU, VK_SHIFT]
                            .iter()
                            .all(|key| GetAsyncKeyState(*key as i32) >= 0),
                    });
                }
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, message, data)
}
fn class(window: HWND) -> String {
    let mut value = [0u16; 128];
    let len = unsafe { GetClassNameW(window, value.as_mut_ptr(), value.len() as i32) };
    String::from_utf16_lossy(&value[..len.max(0) as usize])
}
fn process(window: HWND) -> u32 {
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, &mut pid);
    }
    pid
}
fn desktop_blank(click: Click) -> bool {
    let window = click.window as HWND;
    unsafe {
        if !click.unmodified
            || IsWindow(window) == 0
            || process(window) != process(GetShellWindow())
            || WindowFromPoint(windows_sys::Win32::Foundation::POINT {
                x: click.x,
                y: click.y,
            }) != window
        {
            return false;
        }
        let root = GetAncestor(window, GA_ROOT);
        if !["Progman", "WorkerW"].contains(&class(root).as_str()) {
            return false;
        }
        match class(window).as_str() {
            "Progman" | "WorkerW" | "SHELLDLL_DefView" => true,
            "SysListView32" => {
                if class(GetParent(window)) != "SHELLDLL_DefView" {
                    return false;
                }
                let mut accessible = None;
                let mut child = VARIANT::default();
                let point = AccessiblePoint {
                    x: click.x,
                    y: click.y,
                };
                if AccessibleObjectFromPoint(point, &mut accessible, &mut child).is_err() {
                    return false;
                }
                let Some(accessible) = accessible else {
                    return false;
                };
                let Ok(role) = accessible.get_accRole(&child) else {
                    return false;
                };
                // 图标可能是父列表的子编号，也可能有独立对象；按实际角色排除两种情况。
                i32::try_from(&child).ok() == Some(0)
                    && WindowFromPoint(windows_sys::Win32::Foundation::POINT {
                        x: click.x,
                        y: click.y,
                    }) == window
                    && i32::try_from(&role).is_ok_and(|role| {
                        role == ROLE_SYSTEM_LIST as i32 || role == ROLE_SYSTEM_CLIENT as i32
                    })
            }
            _ => false,
        }
    }
}

fn outside_panel(app: &AppHandle, click: Click) -> bool {
    let hit = click.window as HWND;
    if let Some(panel) = app.get_webview_window("panel") {
        if let Ok(raw) = desktop_native::handle(&panel) {
            let panel = raw as HWND;
            // 面板内的 WebView 和由面板拥有的系统弹窗都属于面板。
            if hit == panel
                || unsafe { IsChild(panel, hit) != 0 || GetAncestor(hit, GA_ROOTOWNER) == panel }
            {
                return false;
            }
        }
    }
    if let Some(main) = app.get_webview_window("main") {
        if let Ok(raw) = desktop_native::handle(&main) {
            let main = raw as HWND;
            let mut point = POINT {
                x: click.x,
                y: click.y,
            };
            let mut area = RECT::default();
            // 管理页内容区由页面区分面板开关和普通外部点击。
            if unsafe {
                GetAncestor(hit, GA_ROOT) == main
                    && ScreenToClient(main, &mut point) != 0
                    && GetClientRect(main, &mut area) != 0
                    && PtInRect(&area, point) != 0
            } {
                return false;
            }
        }
    }
    let pid = process(hit);
    if pid == std::process::id() && class(hit) == "#32768" {
        return false;
    }
    // 托盘图标和它的菜单交给托盘事件，以免开菜单时先收起、选面板时又展开。
    if pid == process(unsafe { GetShellWindow() }) {
        if let Some(rect) = app
            .tray_by_id(crate::MAIN_TRAY)
            .and_then(|tray| tray.rect().ok().flatten())
        {
            let (tauri::Position::Physical(origin), tauri::Size::Physical(size)) =
                (rect.position, rect.size)
            else {
                return true;
            };
            return (i64::from(click.x) - i64::from(origin.x)) as u64 >= u64::from(size.width)
                || (i64::from(click.y) - i64::from(origin.y)) as u64 >= u64::from(size.height);
        }
    }
    true
}

fn same_pair(
    previous: (i32, i32, u32),
    current: (i32, i32, u32),
    max_ms: u32,
    width: i32,
    height: i32,
) -> bool {
    current.2.wrapping_sub(previous.2) <= max_ms
        && current.0.abs_diff(previous.0) <= (width.max(0) / 2) as u32
        && current.1.abs_diff(previous.1) <= (height.max(0) / 2) as u32
}

#[derive(Debug, Eq, PartialEq)]
enum ClickAction {
    Dismiss,
    Open,
    Ignore,
}

fn click_action(scene: u64, first_blank_scene: Option<u64>) -> ClickAction {
    if coordinator::scene_is_foreground(scene) {
        ClickAction::Dismiss
    // 前台面板已由第一下点击收起或置底，第二下不重新打开。
    } else if first_blank_scene == Some(scene) {
        ClickAction::Open
    } else {
        ClickAction::Ignore
    }
}

pub fn start(app: &AppHandle, enabled: bool) -> Result<Listener, String> {
    set_enabled(enabled);
    let (tx, rx) = mpsc::sync_channel::<Click>(32);
    CLICKS.set(tx).map_err(|_| "桌面双击监听已经启动")?;
    let app = app.clone();
    std::thread::Builder::new()
        .name("desktop-hit-test".into())
        .spawn(move || {
            let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            if !initialized && ENABLED.load(Ordering::Acquire) {
                app.state::<Core>()
                    .report("桌面双击可访问性初始化失败".into());
            }
            let mut previous: Option<Click> = None;
            while !STOPPED.load(Ordering::Acquire) {
                let click = match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                    Ok(click) => click,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                let blank = initialized && ENABLED.load(Ordering::Acquire) && desktop_blank(click);
                let first = previous.take().filter(|last| {
                    blank
                        && last.window == click.window
                        && unsafe {
                            same_pair(
                                (last.x, last.y, last.time),
                                (click.x, click.y, click.time),
                                GetDoubleClickTime(),
                                GetSystemMetrics(SM_CXDOUBLECLK),
                                GetSystemMetrics(SM_CYDOUBLECLK),
                            )
                        }
                });
                if blank && first.is_none() {
                    previous = Some(click);
                }
                let result = match click_action(click.scene, first.map(|last| last.scene)) {
                    ClickAction::Dismiss
                        if click.scene == coordinator::panel_scene()
                            && (blank || outside_panel(&app, click)) =>
                    {
                        coordinator::serialized(&app, |app| {
                            coordinator::dismiss_from_outside(app, click.scene)
                        })
                    }
                    ClickAction::Open => coordinator::serialized(&app, |app| {
                        if click.scene == coordinator::panel_scene()
                            && ENABLED.load(Ordering::Acquire)
                            && !coordinator::hidden(app)?
                        {
                            coordinator::toggle(app)?;
                        }
                        Ok(())
                    }),
                    _ => Ok(()),
                };
                if let Err(error) = result {
                    app.state::<Core>().report(error);
                }
            }
            if initialized {
                unsafe {
                    CoUninitialize();
                }
            }
        })
        .map_err(err)?;
    let (ready, result) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("desktop-mouse-listener".into())
        .spawn(move || unsafe {
            let hook = SetWindowsHookExW(
                WH_MOUSE_LL,
                Some(mouse),
                GetModuleHandleW(std::ptr::null()),
                0,
            );
            if hook.is_null() {
                let _ = ready.send(Err(format!(
                    "面板鼠标监听失败：{}",
                    std::io::Error::last_os_error()
                )));
                return;
            }
            let mut message = MSG::default();
            PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
            let _ = ready.send(Ok(GetCurrentThreadId()));
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            UnhookWindowsHookEx(hook);
        })
        .map_err(err)?;
    result.recv().map_err(err)?.map(Listener)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn double_click_uses_system_bounds_and_handles_tick_wraparound() {
        assert!(same_pair((10, 10, u32::MAX - 5), (11, 11, 4), 500, 4, 4));
        assert!(!same_pair((10, 10, 1), (10, 10, 502), 500, 4, 4));
        assert!(!same_pair((10, 10, 1), (13, 10, 2), 500, 4, 4));
    }

    #[test]
    fn only_double_click_from_the_same_background_scene_opens_panel() {
        assert_eq!(click_action(3, None), ClickAction::Dismiss);
        assert_eq!(click_action(3, Some(3)), ClickAction::Dismiss);
        assert_eq!(click_action(4, Some(3)), ClickAction::Ignore);
        assert_eq!(click_action(4, None), ClickAction::Ignore);
        assert_eq!(click_action(4, Some(4)), ClickAction::Open);
        assert_eq!(click_action(7, Some(4)), ClickAction::Dismiss);
    }
}
