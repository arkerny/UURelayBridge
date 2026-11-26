//! RelayBridge —— Clash Verge Rev 扩展配置生成器。
//!
//! 设计约束（见 docs/）：只读写自己的数据目录；不碰 Clash Verge；
//! 所有写操作必须由用户显式授权。

pub mod commands;
pub mod hostmode;
pub mod platform;
pub mod selftest;
pub mod state;
pub mod autostart;
pub mod keepalive;
mod single_instance;
mod tray;

use std::path::PathBuf;

use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};

/// 数据目录：<系统应用数据目录>/RelayBridge
/// Windows: %APPDATA%\RelayBridge
/// macOS:   ~/Library/Application Support/RelayBridge
///
/// 可用环境变量 UURELAYBRIDGE_DATA_DIR 覆盖（便携模式 / 自动化测试用）。
pub fn env_opt(names: &[&str]) -> Option<String> {
    for n in names {
        if let Ok(v) = std::env::var(n) {
            let t = v.trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

fn data_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    if let Some(t) = env_opt(&["UURELAYBRIDGE_DATA_DIR", "RELAYBRIDGE_DATA_DIR"]) {
        return Ok(PathBuf::from(t));
    }
    // 提权重启时由旧实例传入，保证两个进程用同一份数据目录
    if let Some(pos) = std::env::args().position(|a| a == "--data-dir") {
        if let Some(dir) = std::env::args().nth(pos + 1) {
            if !dir.trim().is_empty() {
                return Ok(PathBuf::from(dir));
            }
        }
    }
    let base = app
        .path()
        .data_dir()
        .map_err(|e| format!("无法获取应用数据目录：{}", e))?;
    Ok(base.join("UURelayBridge"))
}

/// 极简的文件日志：GUI 程序没有控制台，启动期出错必须留下痕迹。
fn log_line(msg: &str) {
    if let Some(path) = env_opt(&["UURELAYBRIDGE_LOG_FILE", "RELAYBRIDGE_LOG_FILE"]) {
        let t = path.trim();
        if !t.is_empty() {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(t) {
                let _ = writeln!(f, "[boot] {msg}");
            }
        }
    }
}

/// WebView2 的用户数据目录。
///
/// Tauri 默认把它设为 `app_local_data_dir()`（Windows 上是
/// `%LOCALAPPDATA%\<identifier>`）。但 WebView2 **不会**替我们创建这个目录：
/// 目录不存在时 `CreateCoreWebView2Environment` 直接返回 `Access denied (0x5)`，
/// 于是首次启动（或用户清理过该目录）时窗口根本建不出来。
/// 这里先自己建好，再显式传给 wry，并按"越靠近标准位置越好"的顺序兜底。
fn webview_data_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    let primary = app.path().app_local_data_dir();
    log_line(&format!("app_local_data_dir = {:?}", primary));

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(p) = primary {
        candidates.push(p);
    }
    // 次选：RelayBridge 自己的数据目录（%APPDATA%\RelayBridge\webview2）
    if let Ok(p) = data_root(app) {
        candidates.push(p.join("webview2"));
    }
    // 最后：系统临时目录
    candidates.push(std::env::temp_dir().join("RelayBridge-webview2"));

    for dir in candidates {
        let r = std::fs::create_dir_all(&dir);
        log_line(&format!("create_dir_all({}) => {:?}", dir.display(), r));
        if r.is_ok() {
            return Some(dir);
        }
    }
    None
}

/// 启动失败时的可见反馈：没有控制台的 GUI 静默退出等于"双击没反应"。
fn report_fatal(_app: &tauri::AppHandle, message: &str) {
    let body = format!(
        "{message}\n\n--- RelayBridge {} ---\n",
        env!("CARGO_PKG_VERSION")
    );
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("RELAYBRIDGE_LOG_FILE") {
        let t = p.trim();
        if !t.is_empty() {
            candidates.push(PathBuf::from(t));
        }
    }
    candidates.push(std::env::temp_dir().join("relaybridge-startup-error.log"));

    let written = candidates
        .into_iter()
        .find(|p| std::fs::write(p, &body).is_ok());

    let text = match &written {
        Some(p) => format!("{message}\n\n详细信息：{}", p.display()),
        None => message.to_string(),
    };
    // 弹窗放到独立线程：主线程直接返回，避免 Windows 把进程标成"未响应"
    std::thread::spawn(move || {
        show_error_dialog(&text);
        std::process::exit(1);
    });
}

#[cfg(windows)]
fn show_error_dialog(message: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(
            hwnd: *mut core::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            utype: u32,
        ) -> i32;
    }
    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }
    let text = wide(message);
    let caption = wide("UURelayBridge 启动失败");
    // MB_ICONERROR | MB_TOPMOST
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            0x0000_0010 | 0x0004_0000,
        );
    }
}

#[cfg(target_os = "macos")]
fn show_error_dialog(message: &str) {
    let escaped = message.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display alert \"RelayBridge 启动失败\" message \"{escaped}\" as critical");
    let _ = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .spawn();
}

#[cfg(not(any(windows, target_os = "macos")))]
fn show_error_dialog(message: &str) {
    eprintln!("{message}");
}

/// 主窗口在这里创建（而不是由配置自动创建），
/// 这样能先准备好 WebView2 数据目录，避免首次启动 Access denied。
///
/// 另外：上一个进程被强杀时，它的 WebView2 子进程可能还占着数据目录
/// （表现为 0x800700AA / "resource is in use"）。这里会在失败后用带后缀的
/// 备用目录重试，保证"双击一定能打开"。
fn create_main_window(app: &tauri::AppHandle, start_hidden: bool) -> tauri::Result<()> {
    // 便携 / 测试用的显式覆盖优先，其次是默认的本地数据目录
    let base_dir = env_opt(&["UURELAYBRIDGE_WEBVIEW_DIR", "RELAYBRIDGE_WEBVIEW_DIR"])
        .map(PathBuf::from)
        .or_else(|| webview_data_dir(app));

    let mut candidates: Vec<Option<PathBuf>> = vec![base_dir.clone()];
    if let Some(dir) = &base_dir {
        for suffix in ["-2", "-3"] {
            let mut p = dir.clone().into_os_string();
            p.push(suffix);
            candidates.push(Some(PathBuf::from(p)));
        }
    }

    let mut last_err = None;
    for (i, dir) in candidates.into_iter().enumerate() {
        let mut builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
            .title("UURelayBridge")
            .inner_size(1080.0, 780.0)
            .min_inner_size(920.0, 640.0)
            .center()
            .visible(!start_hidden)
            .resizable(true)
            .decorations(true);

        match &dir {
            Some(d) => {
                if let Err(e) = std::fs::create_dir_all(d) {
                    log_line(&format!("创建 WebView2 数据目录失败 {}：{e}", d.display()));
                }
                log_line(&format!("WebView2 数据目录：{}", d.display()));
                builder = builder.data_directory(d.clone());
            }
            None => log_line("未能解析 WebView2 数据目录，沿用系统默认位置"),
        }

        // 排障钩子：给 WebView2 传额外的浏览器参数（例如 --remote-debugging-port=9333）
        if let Some(args) = env_opt(&["UURELAYBRIDGE_WEBVIEW_ARGS", "RELAYBRIDGE_WEBVIEW_ARGS"]) {
            let t = args.trim();
            if !t.is_empty() {
                builder = builder.additional_browser_args(t);
                log_line(&format!("WebView2 额外参数：{t}"));
            }
        }

        match builder.build() {
            Ok(window) => {
                if i > 0 {
                    log_line(&format!("主窗口已用备用数据目录启动（第 {} 次尝试）", i + 1));
                }
                let handle = app.clone();
                let w = window.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        if commands::close_to_tray(&handle) {
                            api.prevent_close();
                            let _ = w.hide();
                            commands::log(
                                &handle,
                                "info",
                                "window",
                                "主窗口已隐藏到托盘，程序继续常驻",
                            );
                        }
                    }
                });
                return Ok(());
            }
            Err(e) => {
                log_line(&format!("创建窗口失败（第 {} 次尝试）：{e}", i + 1));
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| tauri::Error::WindowNotFound))
}

/// 排障用：读取首屏布局并写进日志。
const LAYOUT_PROBE_JS: &str = r#"
(function () {
  function r(sel) {
    var e = document.querySelector(sel);
    if (!e) return null;
    var b = e.getBoundingClientRect();
    var cs = getComputedStyle(e);
    return { x: Math.round(b.x), y: Math.round(b.y), w: Math.round(b.width), h: Math.round(b.height),
             bg: cs.backgroundColor, ovY: cs.overflowY, disp: cs.display };
  }
  var log = document.getElementById('logBox');
  // 灌 120 行，验证滚动条是否出现在日志面板右边缘、并且内容可滚动
  if (log) {
    for (var i = 0; i < 120; i++) {
      var d = document.createElement('div');
      d.className = 'line';
      d.innerHTML = '<span class="ts">00:00:00</span><span class="lv info">INFO</span>' +
                    '<span class="src">probe</span><span class="msg">layout probe line ' + i + '</span>';
      log.appendChild(d);
    }
    log.scrollTop = log.scrollHeight;
  }
  var o = {
    theme: document.documentElement.getAttribute('data-theme'),
    vp: { w: innerWidth, h: innerHeight, dpr: devicePixelRatio },
    sidebar: r('.sidebar'), nav: r('.nav'), page: r('.page.active'),
    status: r('.side-status'), log: r('#logBox'),
    logScroll: log ? { sh: log.scrollHeight, ch: log.clientHeight,
                       sbw: log.offsetWidth - log.clientWidth, lines: log.children.length } : null,
    bodyBg: getComputedStyle(document.body).backgroundColor,
    sheets: Array.prototype.map.call(document.styleSheets, function (s) { return s.href || 'inline'; }),
    hostPage: { fields: document.querySelectorAll('#page-host input, #page-host textarea, #page-host select').length,
      domains: (function () { var t = document.getElementById('hostDomains'); return t ? t.value.split('\n').filter(Boolean) : []; })(),
      dns: (document.getElementById('hostDns') || {}).value },
    ui: {
      mode: (document.querySelector('.mode-btn.active') || {}).dataset
        ? document.querySelector('.mode-btn.active').dataset.mode : null,
      activeTab: (document.querySelector('.nav-item.active') || {}).dataset
        ? document.querySelector('.nav-item.active').dataset.tab : null,
      visibleNav: Array.prototype.filter.call(document.querySelectorAll('.nav-item'),
        function (n) { return !n.hidden; }).map(function (n) { return n.textContent.trim(); }),
      pageShown: Array.prototype.filter.call(document.querySelectorAll('.page'),
        function (p) { return p.classList.contains('active'); }).map(function (p) { return p.id; }),
      toggleChecked: (document.getElementById('toggleEnabled') || {}).checked,
      cards: document.querySelectorAll('.card').length,
      steps: document.querySelectorAll('.step').length
    },
    preview: (function () {
      var t = window.__LAST_PREVIEW__ || '';
      return { sni: t.indexOf('RB_SNI_DOMAINS') >= 0, hosts: t.indexOf('hosts[d]') >= 0, socks: t.indexOf('RB_SOCKS_PORT') >= 0, len: t.length };
    })()
  };
  window.__TAURI__.core.invoke('frontend_log', { level: 'info', message: 'LAYOUT ' + JSON.stringify(o) });

















})();
"#;

pub fn run() {
    // 已有实例就把窗口叫到前台，避免抢 WebView2 数据目录导致启动失败
    if single_instance::claim() {
        return;
    }
    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::get_logs,
            commands::frontend_log,
            commands::get_presets,
            commands::host_get,
            commands::host_set_config,
            commands::service_start,
            commands::restart_service,
            commands::service_stop,
            commands::host_apply_hosts,
            commands::host_clear_hosts,
            commands::host_restore_hosts,
            commands::clear_logs,
            commands::preview,
            commands::set_mode,
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

            let root = match data_root(&handle) {
                Ok(p) => p,
                Err(e) => {
                    report_fatal(&handle, &format!("无法确定应用数据目录：{e}"));
                    return Ok(());
                    return Ok(());
                }
            };
            let paths = state::AppPaths::new(root);
            let start_hidden = state::settings::load_settings(&paths.settings())
                .value
                .start_minimized;

            let rt = state::Runtime::new(paths.clone());
            rt.attach(handle.clone());
            app.manage(rt);

            commands::log_startup(&handle, &paths);

            if let Err(e) = create_main_window(&handle, start_hidden) {
                let webview_hint = handle
                    .path()
                    .app_local_data_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "%LOCALAPPDATA%\\<应用ID>".to_string());
                report_fatal(
                    &handle,
                    &format!(
                        "创建主窗口失败：{e}\n\n\
                         WebView2 需要在 {webview_hint} 下建立数据目录，但当前无法写入。\n\n\
                         常见原因：\n\
                         1) 程序被放在受限制的目录里（部分沙箱 / 安全软件会限制该目录下的进程只能写自己的目录）；\n\
                         2) 该目录权限异常，或磁盘已满。\n\n\
                         处理建议：\n\
                         • 用安装包安装，或把 RelayBridge.exe 移到可写目录（例如 %LOCALAPPDATA%\\Programs\\RelayBridge）后再运行；\n\
                         • 或设置环境变量 UURELAYBRIDGE_WEBVIEW_DIR 指向一个可写目录；\n\
                         • 仍然失败时，安装 Microsoft Edge WebView2 Runtime 后重试。"
                    ),
                );
                return Ok(());
            }
            if start_hidden {
                commands::log(&handle, "info", "window", "按设置以隐藏状态启动");
            }
            // 启动即应用主题（标题栏跟随系统 / 浅色 / 深色）
            commands::apply_window_theme(&handle, &handle.state::<state::Runtime>().settings().theme);

            // 排障钩子：把首屏真实布局回写到日志（只在该环境变量存在时执行）
            if env_opt(&["UURELAYBRIDGE_DUMP_LAYOUT", "RELAYBRIDGE_DUMP_LAYOUT"]).is_some() {
                if let Some(w) = handle.get_webview_window("main") {
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(2500));
                        let _ = w.eval(LAYOUT_PROBE_JS);
                    });
                }
            }

            // 托盘初始化失败不应让 GUI 起不来：记录错误，主窗口仍可用
            let enabled = handle.state::<state::Runtime>().settings().mode_enabled();
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

            // SOCKS 模式自检：--socks-selftest <端口>
            if let Some(pos) = std::env::args().position(|a| a == "--socks-selftest") {
                let port: u16 = std::env::args()
                    .nth(pos + 1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(17899);
                let h_log = handle.clone();
                let h_main = handle.clone();
                std::thread::spawn(move || {
                    let mut cfg = hostmode::HostConfig::default();
                    cfg.enabled = true;
                    cfg.socks_port = port;
                    cfg.fragment_mode = "record".to_string();
                    cfg.fragment_parts = 4;
                    let logger: std::sync::Arc<dyn Fn(&str) + Send + Sync> =
                        std::sync::Arc::new(move |m: &str| {
                            if let Some(rt) = h_log.try_state::<state::Runtime>() {
                                rt.log("info", "host", m);
                            }
                        });
                    let root = h_main.state::<state::Runtime>().paths.root.clone();
                    match hostmode::start("clash", &cfg, &root, logger) {
                        Ok(st) => commands::log(
                            &h_main,
                            "ok",
                            "host",
                            format!("socks-selftest 已启动：{}", st.view()),
                        ),
                        Err(e) => commands::log(
                            &h_main,
                            "error",
                            "host",
                            format!("socks-selftest 启动失败：{e}"),
                        ),
                    }
                    std::thread::sleep(std::time::Duration::from_secs(20));
                    std::process::exit(0);
                });
            }
            // 上次开着开关的话，启动时把服务重新拉起来。
            // 必须放到后台线程：启动内核要等它 bind 端口（数百毫秒），
            // 放在 setup 里会推迟窗口创建，用户看到的是"打开没反应"。
            if handle.state::<state::Runtime>().settings().mode_enabled() {
                let h = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    let rt = h.state::<state::Runtime>();
                    match commands::start_service(&h, &rt) {
                        Ok(st) => commands::log(
                            &h,
                            "ok",
                            "host",
                            format!("已按上次状态启动服务：{}", st.view()),
                        ),
                        Err(e) => {
                            commands::log(&h, "error", "host", format!("启动服务失败：{e}"));
                            if let Ok(mut s) = rt.settings.write() {
                                s.enabled = false;
                                let _ = state::settings::save_settings(&rt.paths.settings(), &s);
                            }
                            commands::emit_state(&h);
                        }
                    }
                });
            }
            // 内核保活：进程意外退出 / 卡死时自动重启
            {
                let h = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    crate::keepalive::run(h);
                });
            }

            // 开机自启：同步当前设置到系统
            {
                let rt = handle.state::<state::Runtime>();
                let want = rt.settings().autostart;
                if let Err(e) = crate::autostart::apply(want) {
                    commands::log(&handle, "warn", "app", format!("同步开机自启失败：{e}"));
                }
            }

            // Host 模式端到端自检：--host-selftest <端口> <域名>
            // 启动本地转发 20 秒后退出，供外部脚本验证"解析 SNI → DNS 查真实 IP → 分片转发"。
            if let Some(pos) = std::env::args().position(|a| a == "--host-selftest") {
                let port: u16 = std::env::args()
                    .nth(pos + 1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(18443);
                let domain = std::env::args()
                    .nth(pos + 2)
                    .unwrap_or_else(|| "example.com".to_string());
                let h_log = handle.clone();
                let h_main = handle.clone();
                std::thread::spawn(move || {
                    let mut cfg = hostmode::HostConfig::default();
                    cfg.enabled = true;
                    cfg.listen_port = port;
                    cfg.domains = vec![domain.clone()];
                    cfg.fragment_mode = "record".to_string();
                    cfg.fragment_parts = 4;
                    let logger: std::sync::Arc<dyn Fn(&str) + Send + Sync> =
                        std::sync::Arc::new(move |m: &str| {
                            if let Some(rt) = h_log.try_state::<state::Runtime>() {
                                rt.log("info", "host", m);
                            }
                        });
                    let root = h_main.state::<state::Runtime>().paths.root.clone();
                    match hostmode::start("host", &cfg, &root, logger) {
                        Ok(a) => commands::log(
                            &h_main,
                            "ok",
                            "host",
                            format!("selftest 转发已启动：{}", a.view()),
                        ),
                        Err(e) => commands::log(
                            &h_main,
                            "error",
                            "host",
                            format!("selftest 转发启动失败：{e}"),
                        ),
                    }
                    std::thread::sleep(std::time::Duration::from_secs(20));
                    std::process::exit(0);
                });
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("UURelayBridge 初始化失败");

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


