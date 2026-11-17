//! 只在显式启用后启动的Windows键盘事件线程；不截断或改写系统输入。
use crate::{
    keyboard::{KeyboardState, Snapshot},
    store::AppResult,
};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc,
};
use tokio::sync::watch;

#[derive(Clone, Default)]
pub(crate) struct Event {
    pub snapshot: Snapshot,
    pub ready: bool,
    pub error: Option<String>,
}
pub(crate) struct Capture {
    stopped: Arc<AtomicBool>,
    thread_id: Arc<AtomicU32>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub receiver: watch::Receiver<Event>,
}
impl Capture {
    pub fn start() -> AppResult<Self> {
        let stopped = Arc::new(AtomicBool::new(false));
        let thread_id = Arc::new(AtomicU32::new(0));
        let (sender, receiver) = watch::channel(Event::default());
        let stop = stopped.clone();
        let id = thread_id.clone();
        let thread = std::thread::Builder::new()
            .name("keyboard-display".into())
            .spawn(move || {
                if let Err(error) = platform::run(&stop, &id, &sender) {
                    sender.send_replace(Event {
                        error: Some(error),
                        ..Default::default()
                    });
                }
            })
            .map_err(crate::store::err)?;
        Ok(Self {
            stopped,
            thread_id,
            thread: Some(thread),
            receiver,
        })
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        platform::wake(self.thread_id.load(Ordering::Acquire));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        cell::RefCell,
        ptr::{null, null_mut},
        time::Instant,
    };
    use windows_sys::Win32::{
        Foundation::{LPARAM, LRESULT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyNameTextW, GetKeyState},
            WindowsAndMessaging::*,
        },
    };
    struct Context {
        state: KeyboardState,
        sender: watch::Sender<Event>,
        start: Instant,
        demo_pending: bool,
    }
    thread_local! { static CONTEXT: RefCell<Option<Context>> = const {RefCell::new(None)}; }
    fn name(event: &KBDLLHOOKSTRUCT) -> String {
        match event.vkCode {
            8 => "⌫".into(),
            20 => "Caps".into(),
            27 => "Esc".into(),
            32 => "Space".into(),
            33 => "⇧".into(),
            34 => "⇩".into(),
            36 => "Home".into(),
            37 => "←".into(),
            38 => "↑".into(),
            39 => "→".into(),
            40 => "↓".into(),
            44 => "📸".into(),
            93 => "☰".into(),
            _ => {
                let mut buffer = [0u16; 64];
                let bits = ((event.scanCode & 255) << 16)
                    | if event.flags & LLKHF_EXTENDED != 0 {
                        1 << 24
                    } else {
                        0
                    };
                let count = unsafe {
                    GetKeyNameTextW(bits as i32, buffer.as_mut_ptr(), buffer.len() as i32)
                };
                if count > 0 {
                    String::from_utf16_lossy(&buffer[..count as usize])
                } else {
                    format!("Key {}", event.vkCode)
                }
            }
        }
    }
    unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0
            && lparam != 0
            && matches!(
                wparam as u32,
                WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP
            )
        {
            let event = &*(lparam as *const KBDLLHOOKSTRUCT);
            let up = matches!(wparam as u32, WM_KEYUP | WM_SYSKEYUP);
            CONTEXT.with(|slot| {
                // 回调不等待锁、查询SQLite、操作窗口或写日志。普通输入连名称也不生成。
                if let Ok(mut slot) = slot.try_borrow_mut() {
                    if let Some(context) = slot.as_mut() {
                        if context.state.accepts(event.vkCode as u16, up) {
                            context.demo_pending = false;
                            let name = if up { String::new() } else { name(event) };
                            if context.state.input(
                                event.vkCode as u16,
                                up,
                                &name,
                                context.start.elapsed().as_millis() as u64,
                            ) {
                                context.sender.send_replace(Event {
                                    snapshot: context.state.snapshot(),
                                    ready: true,
                                    error: None,
                                });
                            }
                        }
                    }
                }
            });
        }
        CallNextHookEx(null_mut(), code, wparam, lparam)
    }
    pub fn wake(id: u32) {
        if id != 0 {
            unsafe {
                PostThreadMessageW(id, WM_QUIT, 0, 0);
            }
        }
    }
    pub fn run(
        stopped: &AtomicBool,
        id: &AtomicU32,
        sender: &watch::Sender<Event>,
    ) -> AppResult<()> {
        unsafe {
            let mut message: MSG = std::mem::zeroed();
            PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
            id.store(GetCurrentThreadId(), Ordering::Release);
            if stopped.load(Ordering::Acquire) {
                return Ok(());
            }
            CONTEXT.with(|slot| {
                *slot.borrow_mut() = Some(Context {
                    state: KeyboardState::new(GetKeyState(20) & 1 != 0),
                    sender: sender.clone(),
                    start: Instant::now(),
                    demo_pending: true,
                });
            });
            let handle = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), GetModuleHandleW(null()), 0);
            if handle.is_null() {
                CONTEXT.with(|slot| *slot.borrow_mut() = None);
                return Err(format!(
                    "键盘演示监听初始化失败：{}",
                    std::io::Error::last_os_error()
                ));
            }
            let timer = SetTimer(null_mut(), 0, 30, None);
            if timer == 0 {
                UnhookWindowsHookEx(handle);
                CONTEXT.with(|slot| *slot.borrow_mut() = None);
                return Err("键盘演示定时器初始化失败".into());
            }
            sender.send_modify(|event| event.ready = true);
            let mut result = Ok(());
            while !stopped.load(Ordering::Acquire) {
                let received = GetMessageW(&mut message, null_mut(), 0, 0);
                if received <= 0 {
                    if received < 0 {
                        result = Err("键盘演示消息循环失败".into());
                    } else if !stopped.load(Ordering::Acquire) {
                        result = Err("键盘演示消息循环意外结束".into());
                    }
                    break;
                }
                if message.message == WM_TIMER && message.wParam == timer {
                    CONTEXT.with(|slot| {
                        if let Some(context) = slot.borrow_mut().as_mut() {
                            if context.demo_pending
                                && context.start.elapsed() >= std::time::Duration::from_secs(1)
                            {
                                context.demo_pending = false;
                                context.state.demo(1000);
                                context.sender.send_replace(Event {
                                    snapshot: context.state.snapshot(),
                                    ready: true,
                                    error: None,
                                });
                            }
                            let now = context.start.elapsed().as_millis() as u64;
                            // 只核对本轮已按下的键，放在回调之外。安全桌面/UIPI返回0时按松开处理。
                            let released = context
                                .state
                                .reconcile_released(|code| GetAsyncKeyState(code as i32) < 0, now);
                            let expired = context.state.expire(now);
                            if released || expired {
                                context.sender.send_replace(Event {
                                    snapshot: context.state.snapshot(),
                                    ready: true,
                                    error: None,
                                });
                            }
                        }
                    });
                } else {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            KillTimer(null_mut(), timer);
            UnhookWindowsHookEx(handle);
            CONTEXT.with(|slot| *slot.borrow_mut() = None);
            id.store(0, Ordering::Release);
            sender.send_replace(Event::default());
            result
        }
    }
}
#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn wake(_: u32) {}
    pub fn run(_: &AtomicBool, _: &AtomicU32, _: &watch::Sender<Event>) -> AppResult<()> {
        Err("键盘演示仅支持Windows".into())
    }
}
