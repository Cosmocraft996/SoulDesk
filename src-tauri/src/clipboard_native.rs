//! 固定窗口的主动复制/粘贴；系统调用与目标核对分开以便离线验证。
use crate::{
    clipboard_actions::{Desktop, Paste, Write},
    store::AppResult,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Target {
    pub window: usize,
    pub process: u32,
    pub thread: u32,
    pub created: u64,
    pub focus: usize,
}
pub(crate) trait Platform {
    fn own_window(&mut self, window: usize) -> bool;
    fn inspect(&mut self, window: usize) -> AppResult<Target>;
    fn foreground(&mut self) -> usize;
    fn modifiers_clear(&mut self) -> bool;
    fn clipboard_sequence(&mut self) -> u32;
    fn copy(&mut self, owner: usize, text: &str) -> AppResult<Write>;
    fn hide(&mut self) -> AppResult<()>;
    fn activate(&mut self, window: usize) -> AppResult<()>;
    fn input(&mut self) -> AppResult<Paste>;
}
pub(crate) struct Controlled<P> {
    pub owner: usize,
    pub target: Option<Target>,
    pub platform: P,
    sequence: Option<u32>,
}
impl<P: Platform> Controlled<P> {
    pub(crate) fn new(owner: usize, target: Option<Target>, platform: P) -> Self {
        Self {
            owner,
            target,
            platform,
            sequence: None,
        }
    }
    fn identity_current(&mut self) -> bool {
        self.target.is_some_and(|target| {
            self.platform
                .inspect(target.window)
                .is_ok_and(|actual| actual == target)
        })
    }
}
impl<P: Platform> Desktop for Controlled<P> {
    fn ready(&mut self) -> AppResult<()> {
        let foreground = self.platform.foreground();
        if !self.platform.own_window(self.owner)
            || (foreground != self.owner && !self.target.is_some_and(|t| t.window == foreground))
            || (self.target.is_some() && !self.identity_current())
        {
            return Err("TARGET_CHANGED: 原窗口或前台已变化，未执行".into());
        }
        Ok(())
    }
    fn write(&mut self, text: &str) -> AppResult<Write> {
        if text.is_empty() || text.contains('\0') || text.chars().take(50001).count() > 50000 {
            return Err("INVALID_ARGUMENT: 复制文本须为1～50000字符且不含零字符".into());
        }
        self.ready()?;
        self.sequence = None;
        let result = self.platform.copy(self.owner, text)?;
        if let Write::Copied(sequence) = result {
            if sequence != 0 {
                self.sequence = Some(sequence);
                return Ok(Write::Copied(sequence));
            }
        }
        Ok(Write::Uncertain)
    }
    fn prepare_paste(&mut self) -> AppResult<()> {
        if self.sequence.is_none() || self.target.is_none() {
            return Err("TARGET_CHANGED: 未确认复制或没有原应用目标".into());
        }
        self.ready()?;
        self.platform.hide()?;
        let foreground = self.platform.foreground();
        let target = self.target.unwrap();
        if (foreground != 0 && foreground != self.owner && foreground != target.window)
            || !self.identity_current()
        {
            return Err("TARGET_CHANGED: 隐藏后前台已变化，未抢占其他窗口".into());
        }
        self.platform.activate(target.window)?;
        if !self.target_current() {
            return Err("TARGET_CHANGED: 无法恢复原目标、焦点或剪贴板，未发送输入".into());
        }
        Ok(())
    }
    fn target_current(&mut self) -> bool {
        self.sequence
            .is_some_and(|seq| seq != 0 && self.platform.clipboard_sequence() == seq)
            && self
                .target
                .is_some_and(|target| self.platform.foreground() == target.window)
            && self.identity_current()
            && self.platform.modifiers_clear()
    }
    fn send_paste(&mut self) -> AppResult<Paste> {
        if !self.target_current() {
            return Err("TARGET_CHANGED: 输入前目标、按键或剪贴板已变化".into());
        }
        self.platform.input()
    }
}

#[cfg(windows)]
pub(crate) mod system {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GlobalFree, FILETIME, HGLOBAL, HWND},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardOwner, GetClipboardSequenceNumber,
                OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
            Threading::{
                GetCurrentProcessId, GetProcessTimes, OpenProcess,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
                KEYEVENTF_KEYUP, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_V,
            },
            WindowsAndMessaging::{
                GetAncestor, GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId,
                IsIconic, IsWindow, IsWindowVisible, SetForegroundWindow, GA_ROOT, GUITHREADINFO,
            },
        },
    };
    pub(crate) const SELF_FORMAT: &str = "SoulDesk.clipboard.owner.v1";
    const TARGET_ERROR: &str = "TARGET_CHANGED: 原应用身份、可见性或输入焦点已失效";
    pub(crate) struct Native<F: FnMut() -> AppResult<()>> {
        hide: F,
    }
    impl<F: FnMut() -> AppResult<()>> Native<F> {
        pub(crate) fn new(hide: F) -> Self {
            Self { hide }
        }
    }
    fn process_created(pid: u32) -> AppResult<u64> {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return Err(TARGET_ERROR.into());
        }
        let (mut created, mut exit, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let ok =
            unsafe { GetProcessTimes(process, &mut created, &mut exit, &mut kernel, &mut user) };
        unsafe {
            CloseHandle(process);
        }
        if ok == 0 {
            return Err(TARGET_ERROR.into());
        }
        Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }
    fn inspect(window: usize) -> AppResult<Target> {
        let hwnd = window as HWND;
        if hwnd.is_null()
            || unsafe { IsWindow(hwnd) } == 0
            || unsafe { IsWindowVisible(hwnd) } == 0
            || unsafe { IsIconic(hwnd) } != 0
            || unsafe { GetAncestor(hwnd, GA_ROOT) } != hwnd
        {
            return Err(TARGET_ERROR.into());
        }
        let mut pid = 0;
        let thread = unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if thread == 0 || pid == 0 || pid == unsafe { GetCurrentProcessId() } {
            return Err(TARGET_ERROR.into());
        }
        let mut gui = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetGUIThreadInfo(thread, &mut gui) } == 0
            || gui.hwndFocus.is_null()
            || unsafe { GetAncestor(gui.hwndFocus, GA_ROOT) } != hwnd
        {
            return Err(TARGET_ERROR.into());
        }
        Ok(Target {
            window,
            process: pid,
            thread,
            created: process_created(pid)?,
            focus: gui.hwndFocus as usize,
        })
    }
    pub(crate) fn capture_target() -> AppResult<Target> {
        let window = unsafe { GetAncestor(GetForegroundWindow(), GA_ROOT) };
        inspect(window as usize)
    }
    struct Memory(HGLOBAL);
    impl Drop for Memory {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    GlobalFree(self.0);
                }
            }
        }
    }
    impl Memory {
        fn copy(bytes: &[u8]) -> AppResult<Self> {
            let value = Self(unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) });
            if value.0.is_null() {
                return Err("CLIPBOARD_ERROR: 复制内存分配失败，原剪贴板未改写".into());
            }
            let pointer = unsafe { GlobalLock(value.0) };
            if pointer.is_null() {
                return Err("CLIPBOARD_ERROR: 复制内存不可写，原剪贴板未改写".into());
            }
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
                GlobalUnlock(value.0);
            }
            Ok(value)
        }
        fn give(&mut self, format: u32) -> bool {
            if unsafe { SetClipboardData(format, self.0) }.is_null() {
                return false;
            }
            self.0 = std::ptr::null_mut();
            true
        }
    }
    struct ClipboardLock(bool);
    impl Drop for ClipboardLock {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    CloseClipboard();
                }
            }
        }
    }
    fn copy(owner: usize, text: &str) -> AppResult<Write> {
        let bytes = text
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let mut content = Memory::copy(&bytes)?;
        let mut marker = Memory::copy(&unsafe { GetCurrentProcessId() }.to_le_bytes())?;
        let name = SELF_FORMAT.encode_utf16().chain([0]).collect::<Vec<_>>();
        let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
        if format == 0 {
            return Err("CLIPBOARD_ERROR: 自身复制标识注册失败，原剪贴板未改写".into());
        }
        if unsafe { OpenClipboard(owner as HWND) } == 0 {
            return Err("CLIPBOARD_BUSY: 剪贴板正在使用，请重新选择记录".into());
        }
        let mut lock = ClipboardLock(true);
        if unsafe { EmptyClipboard() } == 0 {
            return Ok(Write::Uncertain);
        }
        if !marker.give(format)
            || !content.give(windows::Win32::System::Ole::CF_UNICODETEXT.0 as u32)
        {
            return Ok(Write::Uncertain);
        }
        let sequence = unsafe { GetClipboardSequenceNumber() };
        if unsafe { CloseClipboard() } == 0 {
            return Ok(Write::Uncertain);
        }
        lock.0 = false;
        if sequence == 0
            || unsafe { GetClipboardSequenceNumber() } != sequence
            || unsafe { GetClipboardOwner() } != owner as HWND
        {
            return Ok(Write::Uncertain);
        }
        Ok(Write::Copied(sequence))
    }
    fn inputs() -> [INPUT; 4] {
        [
            (VK_CONTROL, 0),
            (VK_V, 0),
            (VK_V, KEYEVENTF_KEYUP),
            (VK_CONTROL, KEYEVENTF_KEYUP),
        ]
        .map(|(key, flags)| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        })
    }
    impl<F: FnMut() -> AppResult<()>> Platform for Native<F> {
        fn own_window(&mut self, window: usize) -> bool {
            let hwnd = window as HWND;
            let mut pid = 0;
            unsafe {
                IsWindow(hwnd) != 0
                    && GetWindowThreadProcessId(hwnd, &mut pid) != 0
                    && pid == GetCurrentProcessId()
                    && GetAncestor(hwnd, GA_ROOT) == hwnd
                    && IsWindowVisible(hwnd) != 0
            }
        }
        fn inspect(&mut self, window: usize) -> AppResult<Target> {
            inspect(window)
        }
        fn foreground(&mut self) -> usize {
            unsafe { GetAncestor(GetForegroundWindow(), GA_ROOT) as usize }
        }
        fn modifiers_clear(&mut self) -> bool {
            [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN, VK_V]
                .iter()
                .all(|&key| unsafe { GetAsyncKeyState(i32::from(key)) } >= 0)
        }
        fn clipboard_sequence(&mut self) -> u32 {
            unsafe { GetClipboardSequenceNumber() }
        }
        fn copy(&mut self, owner: usize, text: &str) -> AppResult<Write> {
            copy(owner, text)
        }
        fn hide(&mut self) -> AppResult<()> {
            (self.hide)()
        }
        fn activate(&mut self, window: usize) -> AppResult<()> {
            inspect(window)?;
            if unsafe { GetForegroundWindow() } == window as HWND
                || unsafe { SetForegroundWindow(window as HWND) } != 0
            {
                Ok(())
            } else {
                Err("TARGET_CHANGED: 系统拒绝恢复原应用焦点".into())
            }
        }
        fn input(&mut self) -> AppResult<Paste> {
            let values = inputs();
            let count = unsafe {
                SendInput(
                    values.len() as u32,
                    values.as_ptr(),
                    std::mem::size_of::<INPUT>() as i32,
                )
            };
            if count == 4 {
                return Ok(Paste::Submitted);
            }
            if count == 0 {
                return Err("INPUT_FAILED: 系统未接受粘贴按键".into());
            }
            // 部分输入只释放本次可能按下的V/Ctrl，不重发按下或粘贴。
            unsafe {
                SendInput(2, values[2..].as_ptr(), std::mem::size_of::<INPUT>() as i32);
            }
            Ok(Paste::Uncertain)
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn clipboard_native_input_batch_contains_only_ctrl_v_and_matching_keyups() {
            let batch = inputs();
            assert_eq!(batch.len(), 4);
            for (input, (key, flags)) in batch.iter().zip([
                (VK_CONTROL, 0),
                (VK_V, 0),
                (VK_V, KEYEVENTF_KEYUP),
                (VK_CONTROL, KEYEVENTF_KEYUP),
            ]) {
                assert_eq!(input.r#type, INPUT_KEYBOARD);
                let value = unsafe { input.Anonymous.ki };
                assert_eq!(value.wVk, key);
                assert_eq!(value.dwFlags, flags);
                assert_eq!(value.wScan, 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> Target {
        Target {
            window: 21,
            process: 42,
            thread: 43,
            created: 44,
            focus: 22,
        }
    }
    struct Fake {
        actual: Target,
        foreground: usize,
        own: bool,
        modifiers: bool,
        seq: u32,
        trace: Vec<&'static str>,
        deny_focus: bool,
        external_after_hide: bool,
    }
    impl Default for Fake {
        fn default() -> Self {
            Self {
                actual: target(),
                foreground: 11,
                own: true,
                modifiers: false,
                seq: 80,
                trace: Vec::new(),
                deny_focus: false,
                external_after_hide: false,
            }
        }
    }
    impl Platform for Fake {
        fn own_window(&mut self, w: usize) -> bool {
            w == 11 && self.own
        }
        fn inspect(&mut self, w: usize) -> AppResult<Target> {
            if w == self.actual.window {
                Ok(self.actual)
            } else {
                Err("window gone".into())
            }
        }
        fn foreground(&mut self) -> usize {
            self.foreground
        }
        fn modifiers_clear(&mut self) -> bool {
            !self.modifiers
        }
        fn clipboard_sequence(&mut self) -> u32 {
            self.seq
        }
        fn copy(&mut self, owner: usize, text: &str) -> AppResult<Write> {
            assert_eq!(owner, 11);
            assert_eq!(text, "选中文字🙂");
            self.trace.push("copy");
            self.seq = 81;
            Ok(Write::Copied(81))
        }
        fn hide(&mut self) -> AppResult<()> {
            self.trace.push("hide");
            self.foreground = if self.external_after_hide {
                99
            } else {
                self.actual.window
            };
            Ok(())
        }
        fn activate(&mut self, w: usize) -> AppResult<()> {
            self.trace.push("activate");
            if self.deny_focus {
                return Err("focus denied".into());
            }
            self.foreground = w;
            Ok(())
        }
        fn input(&mut self) -> AppResult<Paste> {
            self.trace.push("input");
            Ok(Paste::Submitted)
        }
    }
    fn desktop() -> Controlled<Fake> {
        Controlled::new(11, Some(target()), Fake::default())
    }
    #[test]
    fn clipboard_native_copy_requires_real_owned_active_window_and_never_activates_target() {
        let mut io = desktop();
        io.ready().unwrap();
        assert!(matches!(io.write("选中文字🙂").unwrap(), Write::Copied(81)));
        assert_eq!(io.platform.trace, vec!["copy"]);
        let mut io = desktop();
        io.platform.own = false;
        assert!(io.ready().is_err());
        assert!(io.write("选中文字🙂").is_err());
        assert!(io.platform.trace.is_empty());
        let mut io = desktop();
        io.platform.foreground = 99;
        assert!(io.ready().is_err());
        assert!(io.write("选中文字🙂").is_err());
        assert!(io.platform.trace.is_empty());
    }
    #[test]
    fn clipboard_native_paste_hides_owner_restores_exact_target_and_checks_again_before_input() {
        let mut io = desktop();
        io.write("选中文字🙂").unwrap();
        io.prepare_paste().unwrap();
        assert!(io.target_current());
        assert!(matches!(io.send_paste().unwrap(), Paste::Submitted));
        assert_eq!(io.platform.trace, vec!["copy", "hide", "activate", "input"]);
    }
    #[test]
    fn clipboard_native_reused_hwnd_pid_thread_or_changed_focus_never_receive_input() {
        for field in 0..4 {
            let mut io = desktop();
            io.write("选中文字🙂").unwrap();
            match field {
                0 => io.platform.actual.process += 1,
                1 => io.platform.actual.thread += 1,
                2 => io.platform.actual.created += 1,
                _ => io.platform.actual.focus += 1,
            };
            assert!(io.prepare_paste().is_err());
            assert!(io.send_paste().is_err());
            assert_eq!(io.platform.trace, vec!["copy"]);
        }
    }
    #[test]
    fn clipboard_native_external_foreground_after_hide_cancels_without_stealing_focus() {
        let mut io = desktop();
        io.write("选中文字🙂").unwrap();
        io.platform.external_after_hide = true;
        assert!(io.prepare_paste().is_err());
        assert!(!io.target_current());
        assert!(io.send_paste().is_err());
        assert_eq!(io.platform.trace, vec!["copy", "hide"]);
    }
    #[test]
    fn clipboard_native_target_or_clipboard_changes_after_prepare_cancel_input() {
        for changed in [false, true] {
            let mut io = desktop();
            io.write("选中文字🙂").unwrap();
            io.prepare_paste().unwrap();
            if changed {
                io.platform.foreground = 99
            } else {
                io.platform.seq += 1
            };
            assert!(!io.target_current());
            assert!(io.send_paste().is_err());
            assert!(!io.platform.trace.contains(&"input"));
        }
    }
    #[test]
    fn clipboard_native_pressed_modifiers_focus_denial_or_missing_target_never_send_input() {
        let mut io = desktop();
        io.write("选中文字🙂").unwrap();
        io.platform.deny_focus = true;
        assert!(io.prepare_paste().is_err());
        assert!(!io.platform.trace.contains(&"input"));
        let mut io = desktop();
        io.write("选中文字🙂").unwrap();
        io.prepare_paste().unwrap();
        io.platform.modifiers = true;
        assert!(io.send_paste().is_err());
        assert!(!io.platform.trace.contains(&"input"));
        let mut io = Controlled::new(11, None, Fake::default());
        io.ready().unwrap();
        io.write("选中文字🙂").unwrap();
        assert!(io.prepare_paste().is_err());
        assert_eq!(io.platform.trace, vec!["copy"]);
    }
    #[test]
    fn clipboard_native_never_pastes_before_confirmed_copy_and_checks_text_before_effect() {
        let mut io = desktop();
        assert!(io.prepare_paste().is_err());
        assert!(io.send_paste().is_err());
        assert!(io.platform.trace.is_empty());
        for invalid in ["".into(), "zero\0text".into(), "界".repeat(50001)] {
            assert!(io.write(&invalid).is_err());
        }
        assert!(io.platform.trace.is_empty());
    }
}
