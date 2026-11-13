//! 正式应用与隔离探针共用的 Windows 桌面适配。材质来源及许可见 docs/09、docs/licenses/。
use crate::store::err;
use serde_json::{json, Value};
use tauri::WebviewWindow;
fn attachment_valid(
    alive: bool,
    child: bool,
    parent_matches: bool,
    topmost: bool,
    pinned: bool,
) -> Result<(), String> {
    if alive && parent_matches && child != pinned && topmost == pinned {
        Ok(())
    } else {
        Err("原生窗口存活、父窗口或层级与当前展示意图不一致".into())
    }
}

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, POINT, RECT},
    Graphics::Gdi::ScreenToClient,
    UI::{
        HiDpi::{GetDpiForWindow, GetWindowDpiAwarenessContext},
        WindowsAndMessaging::*,
    },
};
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
pub fn handle(w: &WebviewWindow) -> Result<usize, String> {
    Ok(w.hwnd().map_err(err)?.0 as usize)
}
pub fn foreground() -> usize {
    unsafe { GetForegroundWindow() as usize }
}
fn keyboard_state(h: HWND) -> Value {
    let query = |thread| {
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let ok = unsafe { GetGUIThreadInfo(thread, &mut info) };
        if ok == 0 {
            return json!({"thread":thread,"querySucceeded":false});
        }
        json!({"thread":thread,"querySucceeded":true,"active":info.hwndActive as usize,"focus":info.hwndFocus as usize,"focusClass":class(info.hwndFocus),"focusPid":pid(info.hwndFocus),"focusWithinHost":info.hwndFocus==h||unsafe{IsChild(h,info.hwndFocus)}!=0})
    };
    json!({"foreground":foreground(),"foregroundQueue":query(0),"hostQueue":query(unsafe{GetWindowThreadProcessId(h,std::ptr::null_mut())})})
}
fn activate_click(h: HWND) -> Result<Value, String> {
    use windows_sys::Win32::{
        System::Threading::GetCurrentThreadId,
        UI::Input::KeyboardAndMouse::{GetAsyncKeyState, SetFocus, VK_LBUTTON, VK_RBUTTON},
    };
    let skip = |reason| Ok(json!({"attempted":false,"reason":reason}));
    unsafe {
        if IsWindow(h) == 0
            || pid(h) != std::process::id()
            || IsWindowVisible(h) == 0
            || GetWindowLongPtrW(h, GWL_STYLE) as u32 & WS_CHILD == 0
        {
            return skip("not-visible-owned-desktop-child");
        }
        if GetWindowThreadProcessId(h, std::ptr::null_mut()) != GetCurrentThreadId() {
            return Err("点击激活必须在窗口所属线程执行".into());
        }
        let physical = if GetSystemMetrics(SM_SWAPBUTTON) != 0 {
            VK_RBUTTON
        } else {
            VK_LBUTTON
        };
        if GetAsyncKeyState(physical as i32) >= 0 {
            return skip("pointer-no-longer-down");
        }
        let mut point = POINT::default();
        if GetCursorPos(&mut point) == 0 {
            return Err(winerr("读取点击位置失败"));
        }
        let hit = WindowFromPoint(point);
        if hit != h && IsChild(h, hit) == 0 {
            return skip("pointer-outside-this-widget");
        }
        let root = GetAncestor(h, GA_ROOT);
        if root.is_null() || root != GetAncestor(desktop()?, GA_ROOT) {
            return skip("desktop-parent-mismatch");
        }
        let before = keyboard_state(h);
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(GetCurrentThreadId(), &mut info) == 0 {
            return Err(winerr("读取组件键盘焦点失败"));
        }
        let target = info.hwndFocus;
        if target != h && IsChild(h, target) == 0 {
            return skip("widget-has-not-received-native-focus");
        }
        let already_foreground = GetForegroundWindow() == root;
        let accepted = already_foreground || SetForegroundWindow(root) != 0;
        if accepted {
            SetFocus(target);
        }
        let after = keyboard_state(h);
        Ok(
            json!({"attempted":true,"accepted":accepted,"alreadyForeground":already_foreground,"root":root as usize,"before":before,"after":after,"keyboardInWidget":after["foregroundQueue"]["focusWithinHost"]==true}),
        )
    }
}
pub fn activate_clicked(w: &WebviewWindow) -> Result<Value, String> {
    let raw = handle(w)?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    w.run_on_main_thread(move || {
        let _ = sender.send(activate_click(raw as HWND));
    })
    .map_err(err)?;
    receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .map_err(err)?
}
#[cfg(test)]
#[test]
fn hidden_widget_cannot_request_foreground_activation() {
    struct Owned(HWND);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    let h = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            wide("STATIC").as_ptr(),
            wide("SoulDesk hidden input boundary").as_ptr(),
            WS_POPUP,
            0,
            0,
            120,
            80,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    assert!(!h.is_null());
    let owned = Owned(h);
    let before = foreground();
    assert_eq!(activate_click(owned.0).unwrap()["attempted"], false);
    assert_eq!(
        activate_click(std::ptr::null_mut()).unwrap()["attempted"],
        false
    );
    assert_eq!(foreground(), before);
    assert_eq!(unsafe { IsWindowVisible(owned.0) }, 0);
}
pub fn verify_dpi(child: usize, parent: usize) -> Result<(), String> {
    use windows_sys::Win32::UI::HiDpi::AreDpiAwarenessContextsEqual;
    if child == 0
        || parent == 0
        || unsafe { AreDpiAwarenessContextsEqual(child as _, parent as _) } == 0
    {
        Err(format!(
            "桌面与组件 DPI 模式不兼容：child={child},parent={parent}；拒绝跨进程附着"
        ))
    } else {
        Ok(())
    }
}
pub fn prepare(w: &WebviewWindow) -> Result<(), String> {
    let h = handle(w)? as HWND;
    unsafe {
        let style = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
        windows_sys::Win32::Foundation::SetLastError(0);
        let old = SetWindowLongPtrW(
            h,
            GWL_EXSTYLE,
            ((style & !WS_EX_APPWINDOW) | WS_EX_TOOLWINDOW) as isize,
        );
        if old == 0 && windows_sys::Win32::Foundation::GetLastError() != 0 {
            return Err(winerr("设置工具窗口身份失败"));
        }
    }
    Ok(())
}
fn class(h: HWND) -> String {
    let mut v = [0u16; 128];
    let n = unsafe { GetClassNameW(h, v.as_mut_ptr(), 128) };
    String::from_utf16_lossy(&v[..n.max(0) as usize])
}
fn pid(h: HWND) -> u32 {
    let mut p = 0;
    unsafe { GetWindowThreadProcessId(h, &mut p) };
    p
}
fn winerr(operation: &str) -> String {
    format!("{operation}：{}", std::io::Error::last_os_error())
}
fn desktop() -> Result<HWND, String> {
    unsafe extern "system" fn find(h: HWND, data: LPARAM) -> i32 {
        let shell = GetShellWindow();
        if pid(h) == pid(shell)
            && ["Progman", "WorkerW"].contains(&class(h).as_str())
            && !FindWindowExW(
                h,
                std::ptr::null_mut(),
                wide("SHELLDLL_DefView").as_ptr(),
                std::ptr::null(),
            )
            .is_null()
        {
            *(data as *mut usize) = h as usize;
            0
        } else {
            1
        }
    }
    let mut result = 0usize;
    unsafe { EnumWindows(Some(find), &mut result as *mut usize as LPARAM) };
    if result == 0 {
        Err("未找到 Explorer 桌面视图的宿主；未改用浮窗".into())
    } else {
        Ok(result as HWND)
    }
}
fn bind_child(h: HWND, parent: HWND) -> Result<(), String> {
    unsafe {
        let child_dpi = GetWindowDpiAwarenessContext(h);
        let parent_dpi = GetWindowDpiAwarenessContext(parent);
        // 不改变进程 DPI 模式；句柄与缩放记录供判断跨进程附着风险。
        verify_dpi(child_dpi as usize, parent_dpi as usize)?;
        // 必须在仍是顶层窗口时移除置顶；转成 WS_CHILD 后再调用不会清掉该标志。
        if GetWindowLongPtrW(h, GWL_STYLE) as u32 & WS_CHILD == 0
            && SetWindowPos(
                h,
                HWND_NOTOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            ) == 0
        {
            return Err(winerr("恢复桌面非置顶层级失败"));
        }
        let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
        windows_sys::Win32::Foundation::SetLastError(0);
        let old = SetWindowLongPtrW(h, GWL_STYLE, ((style & !WS_POPUP) | WS_CHILD) as isize);
        if old == 0 && windows_sys::Win32::Foundation::GetLastError() != 0 {
            return Err(winerr("设置子窗口样式失败"));
        }
        SetParent(h, parent);
        if GetParent(h) != parent {
            return Err(winerr("桌面附着失败"));
        }
    }
    Ok(())
}
pub fn attach(w: &WebviewWindow, x: i32, y: i32) -> Result<(), String> {
    let h = handle(w)? as HWND;
    bind_child(h, desktop()?)?;
    let size = w.inner_size().map_err(err)?;
    place(w, x, y, size.width, size.height, true)?;
    check_attachment(w, false)
}
pub fn place(
    w: &WebviewWindow,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    show: bool,
) -> Result<(), String> {
    let h = handle(w)? as HWND;
    let mut point = POINT { x, y };
    unsafe {
        let child = GetWindowLongPtrW(h, GWL_STYLE) as u32 & WS_CHILD != 0;
        if child && ScreenToClient(GetParent(h), &mut point) == 0 {
            return Err(winerr("转换桌面坐标失败"));
        }
        let insert = if child {
            HWND_TOP
        } else if ["panel", "desktop-probe-panel"].contains(&w.label())
            || GetWindowLongPtrW(h, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0
        {
            HWND_TOPMOST
        } else {
            HWND_NOTOPMOST
        };
        if SetWindowPos(
            h,
            insert,
            point.x,
            point.y,
            width as i32,
            height as i32,
            SWP_NOACTIVATE | SWP_FRAMECHANGED | if show { SWP_SHOWWINDOW } else { 0 },
        ) == 0
        {
            return Err(winerr("设置窗口几何失败"));
        }
    }
    Ok(())
}
pub fn show(w: &WebviewWindow) -> Result<(), String> {
    unsafe {
        ShowWindow(handle(w)? as HWND, SW_SHOWNOACTIVATE);
    };
    Ok(())
}
pub fn hide(w: &WebviewWindow) -> Result<(), String> {
    // 原生附着后的样式由本模块拥有，不让 Tao 的旧窗口标志覆盖 WS_CHILD。
    unsafe {
        ShowWindow(handle(w)? as HWND, SW_HIDE);
    }
    Ok(())
}
pub fn pointer_inside(w: &WebviewWindow) -> Result<bool, String> {
    let mut point = POINT::default();
    let mut rect = RECT::default();
    unsafe {
        if GetCursorPos(&mut point) == 0 || GetWindowRect(handle(w)? as HWND, &mut rect) == 0 {
            return Err(winerr("读取面板鼠标位置失败"));
        }
    }
    Ok(
        point.x >= rect.left
            && point.x < rect.right
            && point.y >= rect.top
            && point.y < rect.bottom,
    )
}

/// 在协调工作线程逐帧移动真实窗口，按原右边界裁剪，避免滑到相邻显示器。
/// 不使用要求窗口所属线程同步执行的 AnimateWindow，也不改变窗口焦点。
pub fn slide_panel(w: &WebviewWindow, showing: bool, duration: u32) -> Result<(), String> {
    use windows_sys::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject, SetWindowRgn};
    if duration == 0 {
        return if showing { show(w) } else { hide(w) };
    }
    let h = handle(w)? as HWND;
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(h, &mut rect) } == 0 {
        return Err(winerr("读取滑动面板位置失败"));
    }
    let width = (rect.right - rect.left).max(1) as u32;
    let height = (rect.bottom - rect.top).max(1) as u32;
    let result = (|| {
        let start = std::time::Instant::now();
        loop {
            let progress =
                (start.elapsed().as_secs_f64() * 1000.0 / duration.min(1000) as f64).min(1.0);
            let eased = 1.0 - (1.0 - progress).powi(3);
            let offset = (width as f64 * if showing { 1.0 - eased } else { eased }).round() as i32;
            let region =
                unsafe { CreateRectRgn(0, 0, (width as i32 - offset).max(0), height as i32) };
            if region.is_null() {
                return Err(winerr("创建面板滑动裁剪失败"));
            }
            if unsafe { SetWindowRgn(h, region, 1) } == 0 {
                unsafe {
                    DeleteObject(region);
                }
                return Err(winerr("设置面板滑动裁剪失败"));
            }
            // 成功后区域由系统拥有。
            place(w, rect.left + offset, rect.top, width, height, showing)?;
            if progress >= 1.0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        Ok(())
    })();
    if !showing {
        hide(w)?;
    }
    let restored = place(w, rect.left, rect.top, width, height, showing);
    unsafe {
        SetWindowRgn(h, std::ptr::null_mut(), 1);
    }
    if showing {
        show(w)?;
    }
    restored?;
    result
}
pub fn check_attachment(w: &WebviewWindow, pinned: bool) -> Result<(), String> {
    let h = handle(w)? as HWND;
    unsafe {
        attachment_valid(
            IsWindow(h) != 0 && pid(h) == std::process::id(),
            GetWindowLongPtrW(h, GWL_STYLE) as u32 & WS_CHILD != 0,
            if pinned {
                GetParent(h).is_null()
            } else {
                GetParent(h) == desktop()?
            },
            GetWindowLongPtrW(h, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0,
            pinned,
        )
        .map_err(|e| {
            format!(
                "{} 层级校验失败（预期置顶={pinned}）：{e}；实际 {}",
                w.label(),
                describe(w)
            )
        })?;
    }
    Ok(())
}
pub fn toggle_pin(w: &WebviewWindow) -> Result<bool, String> {
    let h = handle(w)? as HWND;
    let mut rect = RECT::default();
    unsafe {
        if GetWindowRect(h, &mut rect) == 0 {
            return Err(winerr("读取卡片位置失败"));
        }
        if GetWindowLongPtrW(h, GWL_STYLE) as u32 & WS_CHILD == 0 {
            attach(w, rect.left, rect.top)?;
            return Ok(false);
        }
        windows_sys::Win32::Foundation::SetLastError(0);
        SetParent(h, std::ptr::null_mut());
        if windows_sys::Win32::Foundation::GetLastError() != 0 {
            return Err(winerr("卡片脱离桌面失败"));
        }
        let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
        windows_sys::Win32::Foundation::SetLastError(0);
        let old = SetWindowLongPtrW(h, GWL_STYLE, ((style & !WS_CHILD) | WS_POPUP) as isize);
        if old == 0 && windows_sys::Win32::Foundation::GetLastError() != 0 {
            return Err(winerr("设置卡片置顶样式失败"));
        }
        if SetWindowPos(
            h,
            HWND_TOPMOST,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_FRAMECHANGED,
        ) == 0
        {
            return Err(winerr("卡片置顶失败"));
        }
    }
    check_attachment(w, true)?;
    Ok(true)
}
pub fn describe(w: &WebviewWindow) -> Value {
    let Ok(raw) = handle(w) else {
        return json!({"label":w.label(),"error":"无原生句柄"});
    };
    let h = raw as HWND;
    let mut rect = RECT::default();
    unsafe {
        GetWindowRect(h, &mut rect);
        json!({"label":w.label(),"hwnd":raw,"pid":pid(h),"parent":GetParent(h) as usize,"parentClass":class(GetParent(h)),"class":class(h),"style":GetWindowLongPtrW(h,GWL_STYLE),"exStyle":GetWindowLongPtrW(h,GWL_EXSTYLE),"child":GetWindowLongPtrW(h,GWL_STYLE) as u32&WS_CHILD!=0,"topmost":GetWindowLongPtrW(h,GWL_EXSTYLE) as u32&WS_EX_TOPMOST!=0,"visible":IsWindowVisible(h)!=0,"minimized":IsIconic(h)!=0,"dpi":GetDpiForWindow(h),"dpiContext":GetWindowDpiAwarenessContext(h) as usize,"parentDpiContext":GetWindowDpiAwarenessContext(GetParent(h)) as usize,"dwm":dwm_state(h),"keyboard":keyboard_state(h),"rect":{"left":rect.left,"top":rect.top,"width":rect.right-rect.left,"height":rect.bottom-rect.top}})
    }
}
pub fn owned_alive(h: usize) -> bool {
    unsafe { IsWindow(h as HWND) != 0 && pid(h as HWND) == std::process::id() }
}
fn dwm_state(h: HWND) -> Value {
    use windows_sys::Win32::Graphics::Dwm::*;
    let mut backdrop = 0u32;
    let mut dark = 0u32;
    let mut composition = 0;
    unsafe {
        let backdrop_hr = DwmGetWindowAttribute(
            h,
            DWMWA_SYSTEMBACKDROP_TYPE as u32,
            &mut backdrop as *mut _ as _,
            4,
        );
        let dark_hr = DwmGetWindowAttribute(
            h,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            &mut dark as *mut _ as _,
            4,
        );
        let composition_hr = DwmIsCompositionEnabled(&mut composition);
        json!({"backdropQueryHresult":backdrop_hr,"backdropType":if backdrop_hr==0{Some(backdrop)}else{None},"darkMode":if dark_hr==0{Some(dark)}else{None},"compositionEnabled":if composition_hr==0{Some(composition!=0)}else{None}})
    }
}
pub fn persistent_acrylic(w: &WebviewWindow) -> Result<(), String> {
    apply_accent(handle(w)? as HWND)
}
fn apply_accent(h: HWND) -> Result<(), String> {
    use windows_sys::Win32::Foundation::E_INVALIDARG;
    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_SYSTEMBACKDROP_TYPE};
    if unsafe { IsWindow(h) } == 0 || pid(h) != std::process::id() {
        return Err("拒绝向失效或其他进程窗口应用材质".into());
    }
    let none = 1u32;
    let hr = unsafe {
        DwmSetWindowAttribute(
            h,
            DWMWA_SYSTEMBACKDROP_TYPE as u32,
            &none as *const _ as _,
            4,
        )
    };
    // 该可选属性从Windows 11 22H2才支持；旧系统没有这层材质可清除。
    // 只接受“不支持属性”的结果，后续Accent调用仍核对真实返回值。
    if hr < 0 && hr != E_INVALIDARG {
        return Err(format!("关闭系统背景材质失败：HRESULT={hr:#x}"));
    }
    set_accent(h, true)
}
pub fn clear_accent(w: &WebviewWindow) -> Result<(), String> {
    set_accent(handle(w)? as HWND, false)
}
// ABI 参考 window-vibrancy 0.8.1 src/windows.rs（MIT），许可见 docs/licenses/。
// Copyright 2019-2022 Tauri Programme within The Commons Conservancy
// 入口未公开；只对本进程窗口调用，检查句柄、导出入口及真实返回值。
#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient: u32,
    animation: u32,
}
#[repr(C)]
struct CompositionAttribute {
    attribute: u32,
    data: *mut std::ffi::c_void,
    size: usize,
}
fn set_accent(h: HWND, enabled: bool) -> Result<(), String> {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    if unsafe { IsWindow(h) } == 0 || pid(h) != std::process::id() {
        return Err("拒绝向失效或其他进程窗口应用材质".into());
    }
    unsafe {
        let module = GetModuleHandleW(wide("user32.dll").as_ptr());
        if module.is_null() {
            return Err(winerr("读取窗口合成模块失败"));
        }
        let entry = GetProcAddress(module, c"SetWindowCompositionAttribute".as_ptr() as _)
            .ok_or("系统没有窗口合成入口，候选不可用")?;
        let set: unsafe extern "system" fn(HWND, *mut CompositionAttribute) -> i32 =
            std::mem::transmute(entry);
        let mut policy = AccentPolicy {
            state: if enabled { 4 } else { 0 },
            flags: 0,
            gradient: 0x011c1c1c,
            animation: 0,
        };
        let mut data = CompositionAttribute {
            attribute: 0x13,
            data: &mut policy as *mut _ as _,
            size: std::mem::size_of_val(&policy),
        };
        windows_sys::Win32::Foundation::SetLastError(0);
        if set(h, &mut data) == 0 {
            return Err(winerr("窗口合成请求被拒绝"));
        }
    }
    Ok(())
}
pub fn system() -> Value {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
        .ok();
    let get = |name: &str| {
        key.as_ref()
            .and_then(|k| k.get_value::<String, _>(name).ok())
    };
    let transparency = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
        .ok()
        .and_then(|k| k.get_value::<u32, _>("EnableTransparency").ok());
    json!({"os":"windows","build":get("CurrentBuildNumber"),"displayVersion":get("DisplayVersion"),"pid":std::process::id(),"tauri":"2.12.5","attachment":"existing-shell-defview-parent-child-experiment","material":"per-window-accent-candidate","transparencySetting":transparency})
}

#[cfg(test)]
#[test]
fn accent_candidate_checks_handles_and_does_not_change_foreground() {
    assert!(apply_accent(std::ptr::null_mut()).is_err());
    struct Owned(HWND);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    let h = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            wide("STATIC").as_ptr(),
            wide("SoulDesk hidden material test").as_ptr(),
            WS_POPUP,
            0,
            0,
            160,
            120,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    assert!(!h.is_null());
    let owned = Owned(h);
    let before = foreground();
    // 此断言只证明 API 接受请求及未改变前台，不能证明肉眼磨砂效果。
    apply_accent(owned.0).unwrap();
    assert_eq!(foreground(), before);
    assert_eq!(unsafe { IsWindowVisible(owned.0) }, 0);
}
#[cfg(test)]
#[test]
fn pin_to_child_roundtrip_releases_the_actual_topmost_style() {
    struct Owned(HWND);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    // 仅本测试的两个不可见窗口，不附着 Explorer、不显示或激活窗口。
    let make = |extra| {
        let h = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | extra,
                wide("STATIC").as_ptr(),
                wide("SoulDesk hidden native test").as_ptr(),
                WS_POPUP,
                0,
                0,
                160,
                120,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!h.is_null());
        Owned(h)
    };
    let parent = make(0);
    let child = make(WS_EX_TOPMOST);
    unsafe {
        assert_ne!(
            SetWindowPos(
                child.0,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE
            ),
            0
        );
    }
    bind_child(child.0, parent.0).unwrap();
    unsafe {
        assert_eq!(GetParent(child.0), parent.0);
        assert_ne!(GetWindowLongPtrW(child.0, GWL_STYLE) as u32 & WS_CHILD, 0);
        assert_eq!(
            GetWindowLongPtrW(child.0, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
            0
        );
        assert_eq!(IsWindowVisible(child.0), 0);
    }
}

/// 只移动本进程组件，使用实际屏幕坐标，避免父客户区坐标被当作桌面坐标。
pub fn clamp_desktop(w: &WebviewWindow) -> Result<(), String> {
    use tauri::Manager;
    let r = describe(w);
    let r = &r["rect"];
    let (x, y, width, height) = (
        r["left"].as_i64().ok_or("组件位置无效")? as i32,
        r["top"].as_i64().ok_or("组件位置无效")? as i32,
        r["width"].as_u64().ok_or("组件尺寸无效")? as u32,
        r["height"].as_u64().ok_or("组件尺寸无效")? as u32,
    );
    let m = w
        .app_handle()
        .monitor_from_point(x as f64, y as f64)
        .map_err(err)?
        .or(w.primary_monitor().map_err(err)?)
        .ok_or("未找到显示器")?;
    let a = m.work_area();
    let (nw, nh) = (width.min(a.size.width), height.min(a.size.height));
    let nx = x.clamp(
        a.position.x,
        a.position.x + a.size.width.saturating_sub(nw) as i32,
    );
    let ny = y.clamp(
        a.position.y,
        a.position.y + a.size.height.saturating_sub(nh) as i32,
    );
    if (x, y, width, height) != (nx, ny, nw, nh) {
        place(w, nx, ny, nw, nh, false)?;
    }
    Ok(())
}
/// 仅当前物理鼠标按下且命中自有组件时开始拖动/调整，不注入系统输入。
pub fn pointer_move(
    w: &WebviewWindow,
    resize: bool,
    min_width: f64,
    min_height: f64,
) -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON,
    };
    let h = handle(w)? as HWND;
    let mut start = POINT::default();
    let key = unsafe {
        if GetSystemMetrics(SM_SWAPBUTTON) != 0 {
            VK_RBUTTON
        } else {
            VK_LBUTTON
        }
    };
    unsafe {
        if IsWindowVisible(h) == 0
            || pid(h) != std::process::id()
            || GetAsyncKeyState(key as i32) >= 0
            || GetCursorPos(&mut start) == 0
        {
            return Ok(());
        }
        let hit = WindowFromPoint(start);
        if hit != h && IsChild(h, hit) == 0 {
            return Ok(());
        }
    }
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(h, &mut rect) } == 0 {
        return Err(winerr("读取组件几何失败"));
    }
    let scale = w.scale_factor().map_err(err)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while unsafe { GetAsyncKeyState(key as i32) } < 0 && std::time::Instant::now() < deadline {
        let mut p = POINT::default();
        if unsafe { GetCursorPos(&mut p) } == 0 {
            break;
        }
        let (dx, dy) = (p.x - start.x, p.y - start.y);
        if resize {
            place(
                w,
                rect.left,
                rect.top,
                (rect.right - rect.left + dx)
                    .clamp((min_width * scale).ceil() as i32, (1600. * scale) as i32)
                    as u32,
                (rect.bottom - rect.top + dy)
                    .clamp((min_height * scale).ceil() as i32, (1200. * scale) as i32)
                    as u32,
                false,
            )?;
        } else {
            place(
                w,
                rect.left + dx,
                rect.top + dy,
                (rect.right - rect.left) as u32,
                (rect.bottom - rect.top) as u32,
                false,
            )?;
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    clamp_desktop(w)
}
