//! RelayBridge —— Clash Verge Rev 扩展配置生成器。
//!
//! 设计约束（见 docs/）：只读写自己的数据目录；不碰 Clash Verge；
//! 所有写操作必须由用户显式授权。

pub mod commands;
pub mod hostmode;
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
    show_error_dialog(&text);
    // 弹窗是阻塞的，用户点掉之后直接结束进程，避免 Tauri 因 setup 失败而 panic
    std::process::exit(1);
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
fn create_main_window(app: &tauri::AppHandle, start_hidden: bool) -> tauri::Result<()> {
    let mut builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("UURelayBridge")
        .inner_size(1080.0, 780.0)
        .min_inner_size(920.0, 640.0)
        .center()
        .visible(!start_hidden)
        .resizable(true)
        .decorations(true);

    // 便携 / 测试用的显式覆盖优先，其次是默认的本地数据目录，最后是临时目录
    let override_dir = env_opt(&["UURELAYBRIDGE_WEBVIEW_DIR", "RELAYBRIDGE_WEBVIEW_DIR"])
        .map(PathBuf::from);

    match override_dir.or_else(|| webview_data_dir(app)) {
        Some(dir) => {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                log_line(&format!("创建 WebView2 数据目录失败 {}：{e}", dir.display()));
            } else {
                log_line(&format!("WebView2 数据目录：{}", dir.display()));
            }
            builder = builder.data_directory(dir);
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

    let window = match builder.build() {
        Ok(w) => w,
        Err(e) => {
            log_line(&format!("创建窗口失败：{e}"));
            return Err(e);
        }
    };
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
    topbar: r('.topbar'), banner: r('.subtle-banner'), tabs: r('.tabs'),
    content: r('.content'), logbar: r('.logbar'), loghead: r('.logbar-head'), log: r('.log'),
    logScroll: log ? { sh: log.scrollHeight, ch: log.clientHeight,
                       sbw: log.offsetWidth - log.clientWidth, lines: log.children.length,
                       scrollTop: log.scrollTop,
                       right: Math.round(log.getBoundingClientRect().right) } : null,
    bodyBg: getComputedStyle(document.body).backgroundColor,
    codeBg: getComputedStyle(document.documentElement).getPropertyValue('--code-bg').trim(),
    sheets: Array.prototype.map.call(document.styleSheets, function (s) { return s.href || 'inline'; }),
    resize: (function () {
      var before = getComputedStyle(document.documentElement).getPropertyValue('--logbar-h').trim();
      var handle = document.getElementById('logResize');
      try {
        handle.dispatchEvent(new PointerEvent('pointerdown', { clientY: 600, pointerId: 1, bubbles: true }));
        handle.dispatchEvent(new PointerEvent('pointermove', { clientY: 480, pointerId: 1, bubbles: true }));
        handle.dispatchEvent(new PointerEvent('pointerup', { clientY: 480, pointerId: 1, bubbles: true }));
      } catch (e) { return { error: String(e) }; }
      return {
        before: before,
        after: getComputedStyle(document.documentElement).getPropertyValue('--logbar-h').trim(),
        barH: Math.round(document.querySelector('.logbar').getBoundingClientRect().height)
      };
    })(),
    hostPage: { fields: document.querySelectorAll('#page-host input, #page-host textarea, #page-host select').length,
      domains: (function () { var t = document.getElementById('hostDomains'); if (!t) return []; return t.value.split('\n').filter(Boolean); })(),
      dns: (document.getElementById('hostDns') || {}).value },
    ui: { bodyClass: document.body.className, mode: (document.querySelector(".mode-btn.active") || {}).dataset ? document.querySelector(".mode-btn.active").dataset.mode : null, visibleTabs: Array.prototype.filter.call(document.querySelectorAll(".tab"), function (t) { return !t.hidden; }).map(function (t) { return t.textContent; }) },
    preview: (function () {
      var t = (document.getElementById('previewBox') || {}).textContent || '';
      return { sni: t.indexOf('RB_SNI_DOMAINS') >= 0, hosts: t.indexOf('hosts[d]') >= 0, len: t.length };
    })(),
    logbarParent: document.querySelector('.logbar') ? document.querySelector('.logbar').parentElement.className : null,
    contentParent: document.querySelector('.content') ? document.querySelector('.content').parentElement.className : null,
    appKids: Array.prototype.map.call(document.querySelector('.app').children, function (e) {
      var b = e.getBoundingClientRect();
      return { tag: e.tagName, cls: e.className, y: Math.round(b.y), h: Math.round(b.height) };
    }),
    rowsCss: getComputedStyle(document.querySelector('.app')).gridTemplateRows
  };
  window.__TAURI__.core.invoke('frontend_log', { level: 'info', message: 'LAYOUT ' + JSON.stringify(o) });
  // 模拟点击「本机转发」，1.5 秒后回报模式与标签
  try {
    var hostBtn = document.querySelector('#modeSwitch [data-mode="host"]');
    if (hostBtn) {
      setTimeout(function () {
        hostBtn.click();
        setTimeout(function () {
          var active = document.querySelector('.mode-btn.active');
          var tabs = Array.prototype.filter.call(document.querySelectorAll('.tab'), function (t) { return !t.hidden; })
            .map(function (t) { return t.textContent.trim(); });
          var ta = document.getElementById('hostDomains');
          var msg = 'MODE ' + JSON.stringify({
            mode: active ? active.dataset.mode : null,
            tabs: tabs,
            page: (document.querySelector('.page.active') || {}).id || null,
            domains: ta ? ta.value.split('\n').filter(Boolean).length : -1
          });
          window.__TAURI__.core.invoke('frontend_log', { level: 'info', message: msg });
        }, 1500);
      }, 400);
    }
  } catch (e) {}
})();
"#;

pub fn run() {
    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::get_logs,
            commands::frontend_log,
            commands::get_presets,
            commands::host_get,
            commands::host_set_config,
            commands::host_start,
            commands::host_stop,
            commands::host_apply_hosts,
            commands::host_clear_hosts,
            commands::host_open_help,
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
                    match hostmode::start(cfg, logger) {
                        Ok(a) => commands::log(
                            &h_main,
                            "ok",
                            "host",
                            format!("selftest 转发已启动：{a}"),
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
