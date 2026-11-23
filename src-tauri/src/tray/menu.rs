//! 托盘菜单的构建与文案。平台差异只在文案与左键行为上。

use tauri::menu::{CheckMenuItem, Menu, MenuBuilder, MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, Wry};

use crate::platform;

pub struct TrayMenu {
    pub menu: Menu<Wry>,
    pub status: MenuItem<Wry>,
    pub check: CheckMenuItem<Wry>,
}

pub fn status_text(enabled: bool, last_failed: bool) -> String {
    if last_failed {
        "⚠ 上次生成失败·请打开主界面".to_string()
    } else if enabled {
        "RelayBridge 已启用".to_string()
    } else {
        "RelayBridge 已停用".to_string()
    }
}

pub fn build(app: &AppHandle, enabled: bool, last_failed: bool) -> tauri::Result<TrayMenu> {
    let status = MenuItem::with_id(app, "status", status_text(enabled, last_failed), false, None::<&str>)?;
    let check = CheckMenuItem::with_id(
        app,
        "toggle",
        "启用 RelayBridge 规则",
        true,
        enabled,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(app, "open", "打开主界面", true, None::<&str>)?;
    let regen = MenuItem::with_id(app, "regen", "重新生成配置", true, None::<&str>)?;
    let outdir = MenuItem::with_id(app, "outdir", "打开配置目录", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let quit_label = if platform::is_macos() {
        "Quit RelayBridge"
    } else {
        "退出"
    };
    let quit = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?;

    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let sep3 = PredefinedMenuItem::separator(app)?;

    let menu = MenuBuilder::new(app)
        .items(&[
            &status as &dyn tauri::menu::IsMenuItem<Wry>,
            &sep1,
            &check,
            &sep2,
            &open,
            &regen,
            &outdir,
            &sep3,
            &settings,
            &quit,
        ])
        .build()?;

    Ok(TrayMenu { menu, status, check })
}
