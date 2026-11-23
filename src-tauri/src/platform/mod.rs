//! 平台差异集中在这里：打开目录、托盘图标、平台名。
//! 其余模块不允许出现平台判断。

use std::path::Path;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

pub fn platform_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else {
        "Linux"
    }
}

pub fn is_macos() -> bool {
    cfg!(target_os = "macos")
}

/// 用系统文件管理器打开一个目录（只打开，不修改）。
pub fn open_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        windows::open(path)
    }
    #[cfg(target_os = "macos")]
    {
        macos::open(path)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn().map(|_| ())
    }
}

/// 托盘/菜单栏图标。macOS 用模板图（自动适配浅色/深色菜单栏）。
pub fn tray_icon(on: bool) -> tauri::Result<tauri::image::Image<'static>> {
    #[cfg(target_os = "macos")]
    let bytes: &'static [u8] = if on {
        include_bytes!("../../../icons/tray/tray-on-template.png")
    } else {
        include_bytes!("../../../icons/tray/tray-off-template.png")
    };
    #[cfg(not(target_os = "macos"))]
    let bytes: &'static [u8] = if on {
        include_bytes!("../../../icons/tray/tray-on.png")
    } else {
        include_bytes!("../../../icons/tray/tray-off.png")
    };
    tauri::image::Image::from_bytes(bytes)
}

/// macOS 菜单栏图标需要标记为 template 才会随系统主题反色。
pub fn tray_icon_is_template() -> bool {
    cfg!(target_os = "macos")
}

pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
