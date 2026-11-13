//! 只观察Windows休眠/唤醒通知，不改变电源状态或阻止休眠。
use crate::store::AppResult;
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub sequence: u64,
    pub kind: u32,
    pub utc_ms: u64,
    #[serde(skip)]
    pub instant: std::time::Instant,
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{
            atomic::{AtomicU64, Ordering},
            Mutex, OnceLock,
        },
    };
    use windows_sys::Win32::{
        System::Power::{
            PowerRegisterSuspendResumeNotification, PowerUnregisterSuspendResumeNotification,
            DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
        },
        UI::WindowsAndMessaging::DEVICE_NOTIFY_CALLBACK,
    };
    static EVENTS: OnceLock<Mutex<VecDeque<Event>>> = OnceLock::new();
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    unsafe extern "system" fn callback(
        _context: *const core::ffi::c_void,
        kind: u32,
        _setting: *const core::ffi::c_void,
    ) -> u32 {
        if [4, 7, 18].contains(&kind) {
            if let Some(events) = EVENTS.get() {
                if let Ok(mut events) = events.lock() {
                    if events.len() == 64 {
                        events.pop_front();
                    }
                    events.push_back(Event {
                        sequence: SEQUENCE.fetch_add(1, Ordering::SeqCst) + 1,
                        kind,
                        utc_ms: crate::store::now_ms(),
                        instant: std::time::Instant::now(),
                    });
                }
            }
        }
        0
    }
    pub struct Observer {
        handle: *mut core::ffi::c_void,
        _parameters: Box<DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS>,
    }
    impl Observer {
        pub fn new() -> AppResult<Self> {
            EVENTS.get_or_init(|| Mutex::new(VecDeque::new()));
            // 回调只访问进程期静态队列；注销后即便有迟到回调也不引用已释放的实例。
            let parameters = Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
                Callback: Some(callback),
                Context: std::ptr::null_mut(),
            });
            let mut handle = std::ptr::null_mut();
            let status = unsafe {
                PowerRegisterSuspendResumeNotification(
                    DEVICE_NOTIFY_CALLBACK,
                    parameters.as_ref() as *const _ as *mut _,
                    &mut handle,
                )
            };
            if status != 0 {
                return Err(format!("注册Windows电源观察失败：{status}"));
            }
            Ok(Self {
                handle,
                _parameters: parameters,
            })
        }
        pub fn latest(&self) -> AppResult<Vec<Event>> {
            Ok(EVENTS
                .get()
                .ok_or("电源观察未初始化")?
                .lock()
                .map_err(crate::store::err)?
                .iter()
                .cloned()
                .collect())
        }
    }
    impl Drop for Observer {
        fn drop(&mut self) {
            let status = unsafe { PowerUnregisterSuspendResumeNotification(self.handle as isize) };
            if status != 0 {
                eprintln!("注销电源观察失败：{status}");
            }
        }
    }
}
#[cfg(windows)]
pub use platform::Observer;
#[cfg(not(windows))]
pub struct Observer;
#[cfg(not(windows))]
impl Observer {
    pub fn new() -> AppResult<Self> {
        Err("当前平台不支持Windows休眠观察".into())
    }
    pub fn latest(&self) -> AppResult<Vec<Event>> {
        Err("当前平台不支持Windows休眠观察".into())
    }
}
#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn registers_and_unregisters_without_suspending_the_computer() {
        let observer = super::Observer::new().unwrap();
        assert!(observer.latest().is_ok());
        drop(observer);
    }
}
