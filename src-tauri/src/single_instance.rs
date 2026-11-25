//! 单实例：第二次启动时不再抢 WebView2 数据目录（那会直接报 0x800700AA），
//! 而是把已有窗口叫到前台。

#[cfg(windows)]
pub fn claim() -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(
            attr: *mut core::ffi::c_void,
            owner: i32,
            name: *const u16,
        ) -> *mut core::ffi::c_void;
        fn GetLastError() -> u32;
    }
    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(class: *const u16, title: *const u16) -> *mut core::ffi::c_void;
        fn ShowWindow(hwnd: *mut core::ffi::c_void, cmd: i32) -> i32;
        fn SetForegroundWindow(hwnd: *mut core::ffi::c_void) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    const ERROR_ALREADY_EXISTS: u32 = 183;
    let name = wide("Local\\UURelayBridge-SingleInstance-v1");
    let handle = unsafe { CreateMutexW(std::ptr::null_mut(), 1, name.as_ptr()) };
    if handle.is_null() {
        return false; // 创建失败就当没占用，继续启动
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // 把已有窗口叫出来
        unsafe {
            let title = wide("UURelayBridge");
            let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
            if !hwnd.is_null() {
                ShowWindow(hwnd, 9); // SW_RESTORE
                SetForegroundWindow(hwnd);
            }
        }
        return true;
    }
    // 句柄故意不释放：进程活着就一直占着这个互斥体
    false
}

#[cfg(not(windows))]
pub fn claim() -> bool {
    false
}
