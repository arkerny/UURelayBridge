//! RelayBridge —— Clash Verge Rev 扩展配置生成器。
//!
//! 设计约束（见 docs/）：只读写自己的数据目录；不碰 Clash Verge；
//! 所有写操作必须由用户显式授权。

pub mod commands;
pub mod platform;
pub mod selftest;
pub mod state;
pub mod tray;

use std::path::PathBuf;

use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};

/// 数据目录：<系统应用数据目录>/RelayBridge
/// Windows: %APPDATA%\RelayBridge
/// macOS:   ~/Library/Application Support/RelayBridge
///
/// 可用环境变量 RELAYBRIDGE_DATA_DIR 覆盖（便携模式 / 自动化测试用）。
fn data_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    if let Ok(custom) = std::env::var("RELAYBRIDGE_DATA_DIR") {
        let t = custom.trim();
        if !t.is_empty() {
            return Ok(PathBuf::from(t));
        }
    }
    let base = app
        .path()
        .data_dir()
        .map_err(|e| format!("无法获取应用数据目录：{}", e))?;
    Ok(base.join("RelayBridge"))
}

/// 主窗口在这里创建（而不是由配置自动创建），
/// 这样可以在便携模式下把 WebView2 数据目录指到别处；
/// 普通用户不设这个环境变量，行为与默认完全一致。
fn create_main_window(app: &tauri::AppHandle, start_hidden: bool) -> tauri::Result<()> {
    let mut builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("RelayBridge")
        .inner_size(1080.0, 780.0)
        .min_inner_size(920.0, 640.0)
        .center()
        .visible(!start_hidden)
        .resizable(true)
        .decorations(true);

    if let Ok(dir) = std::env::var("RELAYBRIDGE_WEBVIEW_DIR") {
        let t = dir.trim();
        if !t.is_empty() {
            builder = builder.data_directory(PathBuf::from(t));
        }
    }

    let window = builder.build()?;
    let handle = app.clone();
    let w = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            if commands::close_to_tray(&handle) {
                api.prevent_close();
                let _ = w.hide();
                commands::log(&handle, "info", "window", "主窗口已隐藏到托盘，程序继续常驻");
            }
        }
    });
    Ok(())
}

pub fn run() {
    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::get_logs,
            commands::clear_logs,
            commands::preview,
            commands::set_enabled,
            commands::regenerate,
            commands::save_project,
            commands::update_settings,
            commands::open_output_dir,
            commands::open_data_dir,
            commands::open_backups_dir,
            commands::hide_main_window,
            commands::show_main_window_cmd,
            commands::quit_app,
            commands::resolve_paths,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            let root = data_root(&handle)?;
            let paths = state::AppPaths::new(root);
            let start_hidden = state::settings::load_settings(&paths.settings())
                .value
                .start_minimized;

            let rt = state::Runtime::new(paths.clone());
            rt.attach(handle.clone());
            app.manage(rt);

            commands::log_startup(&handle, &paths);

            create_main_window(&handle, start_hidden)?;
            if start_hidden {
                commands::log(&handle, "info", "window", "按设置以隐藏状态启动");
            }
            // 启动即应用主题（标题栏跟随系统 / 浅色 / 深色）
            commands::apply_window_theme(&handle, &handle.state::<state::Runtime>().settings().theme);

            // 托盘初始化失败不应让 GUI 起不来：记录错误，主窗口仍可用
            let enabled = handle.state::<state::Runtime>().settings().enabled;
            let failed = handle
                .state::<state::Runtime>()
                .last()
                .map(|l| !l.ok)
                .unwrap_or(false);
            if let Err(e) = tray::setup(&handle, enabled, failed) {
                commands::log(&handle, "error", "tray", format!("托盘初始化失败：{}", e));
            } else {
                commands::log(&handle, "info", "tray", "托盘/菜单栏已就绪");
            }

            // 无界面自检：relaybridge --selftest
            if std::env::args().any(|a| a == "--selftest") {
                let h = handle.clone();
                std::thread::spawn(move || {
                    let code = selftest::run(&h);
                    h.exit(code);
                });
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("RelayBridge 初始化失败");

    app.run(|handle, event| match event {
        RunEvent::ExitRequested { api, .. } => {
            let rt = handle.state::<state::Runtime>();
            // Windows：关掉窗口不等于退出，保持托盘常驻；
            // macOS：Cmd+Q / 菜单退出应当真的退出。
            if !rt.is_quitting() && !platform::is_macos() {
                api.prevent_exit();
            }
        }
        RunEvent::Exit => {
            tray::teardown();
        }
        _ => {}
    });
}
