//! 托盘 / 菜单栏：创建、事件分发、状态刷新。

pub mod menu;

use std::sync::Mutex;

use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::commands;
use crate::platform;
use crate::state::Runtime as AppRuntime;

struct Handles {
    tray: TrayIcon<Wry>,
    status: tauri::menu::MenuItem<Wry>,
    check: tauri::menu::CheckMenuItem<Wry>,
}

static HANDLES: Mutex<Option<Handles>> = Mutex::new(None);

/// 创建托盘。失败时返回错误，由上层决定是否继续运行 GUI。
pub fn setup(app: &AppHandle, enabled: bool, last_failed: bool) -> tauri::Result<()> {
    let built = menu::build(app, enabled, last_failed)?;
    let icon = platform::tray_icon(enabled)?;

    let tray = TrayIconBuilder::with_id("relaybridge-tray")
        .icon(icon)
        .icon_as_template(platform::tray_icon_is_template())
        .tooltip(if enabled {
            "UURelayBridge · 已启用"
        } else {
            "UURelayBridge · 已停用"
        })
        .menu(&built.menu)
        // macOS：左键弹菜单（符合菜单栏应用习惯）；Windows：左键打开主界面
        .show_menu_on_left_click(platform::is_macos())
        .on_menu_event(|app, event| {
            let id = event.id().as_ref().to_string();
            on_menu(app, &id);
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if !platform::is_macos() {
                    commands::toggle_main_window(tray.app_handle());
                }
            }
        })
        .build(app)?;

    if let Ok(mut g) = HANDLES.lock() {
        *g = Some(Handles {
            tray,
            status: built.status,
            check: built.check,
        });
    }
    Ok(())
}

fn on_menu(app: &AppHandle, id: &str) {
    let handle = app.clone();
    match id {
        "toggle" => {
            let enabled = current_enabled(&handle);
            commands::apply_enabled(&handle, !enabled, true, "tray");
        }
        "regen" => {
            commands::apply_regenerate(&handle, true, "tray");
        }
        "open" => commands::show_main_window(&handle),
        "outdir" => {
            let rt = handle.state::<AppRuntime>();
            if let Err(e) = commands::open_output_dir_impl(&rt) {
                rt.log("error", "tray", format!("打开配置目录失败：{}", e));
            }
        }
        "settings" => {
            commands::show_main_window(&handle);
            let _ = tauri::Emitter::emit(&handle, commands::NAV_EVENT, "settings");
        }
        "quit" => {
            let rt = handle.state::<AppRuntime>();
            rt.begin_quit();
            rt.log("info", "tray", "用户从托盘选择了退出");
            handle.exit(0);
        }
        _ => {}
    }
}

fn current_enabled(app: &AppHandle) -> bool {
    app.state::<AppRuntime>().settings().mode_enabled()
}

/// 状态变化后同步菜单文案、勾选与图标。任何失败都只记日志，不 panic。
pub fn refresh(app: &AppHandle) {
    let rt = app.state::<AppRuntime>();
    let settings = rt.settings();
    let failed = rt.last().map(|l| !l.ok).unwrap_or(false);

    if let Ok(guard) = HANDLES.lock() {
        if let Some(h) = guard.as_ref() {
            let _ = h.check.set_checked(settings.mode_enabled());
            let _ = h.status.set_text(menu::status_text(settings.mode_enabled(), failed));
            if let Ok(img) = platform::tray_icon(settings.mode_enabled()) {
                let _ = h.tray.set_icon(Some(img));
            }
            let _ = h.tray.set_tooltip(Some(if failed {
                "UURelayBridge · 上次生成失败"
            } else if settings.mode_enabled() {
                "UURelayBridge · 已启用"
            } else {
                "UURelayBridge · 已停用"
            }));
        }
    }
}

/// 退出时把托盘图标移除，避免 Windows 上残留幽灵图标。
pub fn teardown() {
    if let Ok(mut g) = HANDLES.lock() {
        *g = None;
    }
}
