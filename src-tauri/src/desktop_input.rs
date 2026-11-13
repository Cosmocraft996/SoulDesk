//! 只观察桌面主键点击，不吞掉系统鼠标事件；可访问性查询在独立工作线程执行。
use crate::{store::err, windows as coordinator, Core};
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
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
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
    valid: bool,
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
    if code >= 0 && ENABLED.load(Ordering::Acquire) {
        let primary = if GetSystemMetrics(SM_SWAPBUTTON) != 0 {
            WM_RBUTTONDOWN
        } else {
            WM_LBUTTONDOWN
        };
        if message as u32 == primary {
            let e = &*(data as *const MSLLHOOKSTRUCT);
            let valid = e.flags & LLMHF_INJECTED == 0
                && [VK_CONTROL, VK_MENU, VK_SHIFT]
                    .iter()
                    .all(|key| GetAsyncKeyState(*key as i32) >= 0);
            if let Some(tx) = CLICKS.get() {
                let _ = tx.try_send(Click {
                    x: e.pt.x,
                    y: e.pt.y,
                    time: e.time,
                    window: WindowFromPoint(e.pt) as usize,
                    valid,
                });
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
        if !click.valid
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
pub fn start(app: &AppHandle, enabled: bool) -> Result<Listener, String> {
    set_enabled(enabled);
    let (tx, rx) = mpsc::sync_channel::<Click>(32);
    CLICKS.set(tx).map_err(|_| "桌面双击监听已经启动")?;
    let app = app.clone();
    std::thread::Builder::new()
        .name("desktop-hit-test".into())
        .spawn(move || {
            let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            if !initialized {
                app.state::<Core>()
                    .report("桌面双击可访问性初始化失败".into());
                return;
            }
            let mut previous: Option<(i32, i32, u32)> = None;
            while !STOPPED.load(Ordering::Acquire) {
                let click = match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                    Ok(click) => click,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                if !ENABLED.load(Ordering::Acquire) || !desktop_blank(click) {
                    previous = None;
                    continue;
                }
                let current = (click.x, click.y, click.time);
                let paired = previous.is_some_and(|last| unsafe {
                    same_pair(
                        last,
                        current,
                        GetDoubleClickTime(),
                        GetSystemMetrics(SM_CXDOUBLECLK),
                        GetSystemMetrics(SM_CYDOUBLECLK),
                    )
                });
                previous = if paired { None } else { Some(current) };
                if paired {
                    coordinator::dispatch(&app, |app| {
                        if ENABLED.load(Ordering::Acquire) && !coordinator::hidden(app)? {
                            coordinator::toggle(app)?;
                        }
                        Ok(())
                    });
                }
            }
            unsafe {
                CoUninitialize();
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
                    "桌面双击监听失败：{}",
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
}
