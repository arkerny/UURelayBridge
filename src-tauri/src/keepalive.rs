//! 内核保活：定期检查内核，异常退出或卡死时自动重启。

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use tauri::Manager;

use crate::commands;
use crate::hostmode;
use crate::state::Runtime;

static FAIL_STREAK: AtomicU32 = AtomicU32::new(0);

const TICK: Duration = Duration::from_secs(5);
const MAX_STREAK: u32 = 3;

pub fn run(app: tauri::AppHandle) {
    commands::log(&app, "info", "keepalive", "keepalive loop started (5s interval)");
    loop {
        std::thread::sleep(TICK);

        let rt = match app.try_state::<Runtime>() {
            Some(r) => r,
            None => {
                commands::log(&app, "warn", "keepalive", "no runtime state");
                continue;
            }
        };
        let settings = rt.settings();
        if !settings.keepalive || !settings.mode_enabled() {
            FAIL_STREAK.store(0, Ordering::Relaxed);
            continue;
        }

        let st = hostmode::status();
        if !st.running {
            commands::log(&app, "warn", "keepalive", "engine not running; restarting");
            match commands::start_service(&app, &rt) {
                Ok(s) => commands::log(&app, "ok", "keepalive", format!("recovered: {}", s.view())),
                Err(e) => {
                    commands::log(&app, "error", "keepalive", format!("restart failed: {e}"));
                    commands::emit_state(&app);
                    continue;
                }
            }
            commands::emit_state(&app);
            continue;
        }

        let addr = st
            .socks
            .clone()
            .or_else(|| st.https.clone())
            .unwrap_or_default();
        if addr.is_empty() {
            continue;
        }
        let healthy = addr
            .parse::<std::net::SocketAddr>()
            .ok()
            .map(|a| std::net::TcpStream::connect_timeout(&a, Duration::from_secs(2)).is_ok())
            .unwrap_or(false);

        if healthy {
            if FAIL_STREAK.swap(0, Ordering::Relaxed) > 0 {
                commands::log(&app, "ok", "keepalive", "engine responds again");
            }
            continue;
        }

        let n = FAIL_STREAK.fetch_add(1, Ordering::Relaxed) + 1;
        commands::log(&app, "warn", "keepalive", format!("engine unresponsive {n}/{MAX_STREAK}"));
        if n >= MAX_STREAK {
            FAIL_STREAK.store(0, Ordering::Relaxed);
            commands::log(&app, "error", "keepalive", "engine stuck; restarting");
            hostmode::stop();
            std::thread::sleep(Duration::from_millis(300));
            match commands::start_service(&app, &rt) {
                Ok(s) => commands::log(&app, "ok", "keepalive", format!("restarted: {}", s.view())),
                Err(e) => commands::log(&app, "error", "keepalive", format!("restart failed: {e}")),
            }
            commands::emit_state(&app);
        }
    }
}
