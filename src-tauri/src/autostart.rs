//! 开机自启：Windows 写 HKCU Run 键，macOS 用 LaunchAgent。

use std::path::Path;

/// 设置/取消开机自启。
pub fn apply(enable: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        apply_windows(enable)
    }
    #[cfg(target_os = "macos")]
    {
        apply_macos(enable)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = enable;
        Err("当前平台不支持自动设置开机自启".to_string())
    }
}

/// 查询当前是否已启用（以系统状态为准）。
pub fn is_enabled() -> bool {
    #[cfg(windows)]
    {
        read_windows().is_some()
    }
    #[cfg(target_os = "macos")]
    {
        Path::new(&macos_plist()).exists()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        false
    }
}

#[cfg(windows)]
fn apply_windows(enable: bool) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(hkey: usize, sub: *const u16, opt: u32, sam: u32, out: *mut usize) -> i32;
        fn RegSetValueExW(
            hkey: usize,
            name: *const u16,
            reserved: u32,
            ty: u32,
            data: *const u8,
            len: u32,
        ) -> i32;
        fn RegDeleteValueW(hkey: usize, name: *const u16) -> i32;
        fn RegCloseKey(hkey: usize) -> i32;
        fn RegQueryValueExW(
            hkey: usize,
            name: *const u16,
            reserved: *mut u32,
            ty: *mut u32,
            data: *mut u8,
            len: *mut u32,
        ) -> i32;
    }

    const HKEY_CURRENT_USER: usize = 0x8000_0001;
    const KEY_SET_VALUE: u32 = 0x0002;
    const KEY_QUERY_VALUE: u32 = 0x0001;
    const REG_SZ: u32 = 1;

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    let sub = wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let name = wide("UURelayBridge");

    let mut hkey: usize = 0;
    let rc = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            0,
            KEY_SET_VALUE | KEY_QUERY_VALUE,
            &mut hkey,
        )
    };
    if rc != 0 {
        return Err(format!("打开注册表失败（code {rc}）"));
    }

    let result = if enable {
        let exe = std::env::current_exe().map_err(|e| format!("取自身路径失败：{e}"))?;
        let value = format!("\"{}\" --autostart", exe.display());
        let data = wide(&value);
        let bytes = unsafe {
            std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2)
        };
        let rc = unsafe {
            RegSetValueExW(hkey, name.as_ptr(), 0, REG_SZ, bytes.as_ptr(), bytes.len() as u32)
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("写注册表失败（code {rc}）"))
        }
    } else {
        unsafe { RegDeleteValueW(hkey, name.as_ptr()) };
        Ok(())
    };

    unsafe { RegCloseKey(hkey) };
    result
}

#[cfg(windows)]
fn read_windows() -> Option<String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(hkey: usize, sub: *const u16, opt: u32, sam: u32, out: *mut usize) -> i32;
        fn RegQueryValueExW(
            hkey: usize,
            name: *const u16,
            reserved: *mut u32,
            ty: *mut u32,
            data: *mut u8,
            len: *mut u32,
        ) -> i32;
        fn RegCloseKey(hkey: usize) -> i32;
    }

    const HKEY_CURRENT_USER: usize = 0x8000_0001;
    const KEY_QUERY_VALUE: u32 = 0x0001;

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    let sub = wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let name = wide("UURelayBridge");
    let mut hkey: usize = 0;
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_QUERY_VALUE, &mut hkey) } != 0
    {
        return None;
    }
    let mut buf = [0u16; 512];
    let mut len = (buf.len() * 2) as u32;
    let mut ty = 0u32;
    let rc = unsafe {
        RegQueryValueExW(
            hkey,
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut ty,
            buf.as_mut_ptr() as *mut u8,
            &mut len,
        )
    };
    unsafe { RegCloseKey(hkey) };
    if rc != 0 {
        return None;
    }
    let n = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n.min(buf.len())]))
}

#[cfg(target_os = "macos")]
fn macos_plist() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/Library/LaunchAgents/com.uurelaybridge.desktop.plist")
}

#[cfg(target_os = "macos")]
fn apply_macos(enable: bool) -> Result<(), String> {
    let path = macos_plist();
    if enable {
        let exe = std::env::current_exe().map_err(|e| format!("取自身路径失败：{e}"))?;
        let body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.uurelaybridge.desktop</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>--autostart</string>
  </array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
"#,
            exe.display()
        );
        std::fs::create_dir_all(Path::new(&path).parent().unwrap_or(Path::new("/tmp")))
            .map_err(|e| format!("创建 LaunchAgents 目录失败：{e}"))?;
        std::fs::write(&path, body).map_err(|e| format!("写 plist 失败：{e}"))?;
        let _ = std::process::Command::new("launchctl")
            .args(["load", "-w", &path])
            .status();
        Ok(())
    } else {
        let _ = std::process::Command::new("launchctl")
            .args(["unload", "-w", &path])
            .status();
        let _ = std::fs::remove_file(&path);
        Ok(())
    }
}
