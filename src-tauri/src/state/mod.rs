//! 运行期状态：内存中的设置/项目/日志/上次结果。所有落盘都经过 commands 层。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

// 纯逻辑（路径 / 设置读写）放在 core crate，这里只做转出口
pub use relaybridge_core::{paths, settings};
pub use paths::AppPaths;
pub use settings::Settings;

use relaybridge_core::model::Project as ProjectModel;

pub const LOG_CAPACITY: usize = 800;
pub const LOG_EVENT: &str = "rb://log";

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub ts: String,
    pub level: String,
    pub source: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LastRun {
    pub at: String,
    pub ok: bool,
    pub message: String,
    pub sha256: Option<String>,
    pub bytes: Option<u64>,
    pub backup: Option<String>,
    pub rule_count: usize,
    pub enabled: bool,
}

pub struct Runtime {
    pub paths: AppPaths,
    pub settings: RwLock<Settings>,
    pub project: RwLock<ProjectModel>,
    /// 启动时读取配置遇到的问题（损坏、版本不符……），只展示不自动修复。
    pub issues: Mutex<Vec<String>>,
    pub logs: Mutex<VecDeque<LogEntry>>,
    pub last: RwLock<Option<LastRun>>,
    pub quitting: AtomicBool,
    /// 串行化所有写操作，避免并发覆盖。
    pub write_lock: Mutex<()>,
    app: OnceLock<AppHandle>,
}

pub fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

pub fn now_hms() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

impl Runtime {
    pub fn new(paths: AppPaths) -> Self {
        let s = settings::load_settings(&paths.settings());
        let p = settings::load_project(&paths.project());
        let mut issues = Vec::new();
        if let Some(i) = &s.issue {
            issues.push(i.clone());
        }
        if let Some(i) = &p.issue {
            issues.push(i.clone());
        }

        let rt = Self {
            paths,
            settings: RwLock::new(s.value),
            project: RwLock::new(p.value),
            issues: Mutex::new(issues),
            logs: Mutex::new(VecDeque::with_capacity(LOG_CAPACITY)),
            last: RwLock::new(None),
            quitting: AtomicBool::new(false),
            write_lock: Mutex::new(()),
            app: OnceLock::new(),
        };
        rt
    }

    pub fn attach(&self, app: AppHandle) {
        let _ = self.app.set(app);
    }

    pub fn app(&self) -> Option<&AppHandle> {
        self.app.get()
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn project(&self) -> ProjectModel {
        self.project.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn last(&self) -> Option<LastRun> {
        self.last.read().ok().and_then(|g| g.clone())
    }

    pub fn set_last(&self, l: LastRun) {
        if let Ok(mut g) = self.last.write() {
            *g = Some(l);
        }
    }

    /// 写一条日志：进内存环形缓冲，同时推给前端。
    pub fn log(&self, level: &str, source: &str, message: impl Into<String>) {
        let entry = LogEntry {
            ts: now_hms(),
            level: level.to_string(),
            source: source.to_string(),
            message: message.into(),
        };
        if let Ok(mut buf) = self.logs.lock() {
            if buf.len() >= LOG_CAPACITY {
                buf.pop_front();
            }
            buf.push_back(entry.clone());
        }
        // 排障用：设置 RELAYBRIDGE_LOG_FILE 后把日志同时写进文件（默认关闭）
        if let Ok(path) = std::env::var("RELAYBRIDGE_LOG_FILE") {
            if !path.trim().is_empty() {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path.trim())
                {
                    let _ = writeln!(
                        f,
                        "{} [{}] [{}] {}",
                        entry.ts, entry.level, entry.source, entry.message
                    );
                }
            }
        }
        if let Some(app) = self.app.get() {
            let _ = app.emit(LOG_EVENT, entry);
        }
    }

    pub fn log_tail(&self, n: usize) -> Vec<LogEntry> {
        let buf = match self.logs.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        buf.iter().rev().take(n).rev().cloned().collect()
    }

    pub fn clear_logs(&self) {
        if let Ok(mut buf) = self.logs.lock() {
            buf.clear();
        }
    }

    pub fn add_issue(&self, msg: String) {
        if let Ok(mut v) = self.issues.lock() {
            if !v.contains(&msg) {
                v.push(msg);
            }
        }
    }

    pub fn issues(&self) -> Vec<String> {
        self.issues.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }

    pub fn begin_quit(&self) {
        self.quitting.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn runtime_starts_without_touching_disk() {
        let d = relaybridge_core::test_tmp_dir("rt");
        std::fs::create_dir_all(&d).unwrap();
        let rt = Runtime::new(AppPaths::new(d.clone()));
        assert!(!rt.settings().enabled);
        assert!(rt.project().rules.is_empty());
        assert!(rt.last().is_none());
        assert!(!d.join("settings.json").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn log_ring_is_bounded() {
        let d = relaybridge_core::test_tmp_dir("rt-log");
        let rt = Runtime::new(AppPaths::new(d.clone()));
        for i in 0..(LOG_CAPACITY + 50) {
            rt.log("info", "test", format!("line {}", i));
        }
        assert_eq!(rt.log_tail(LOG_CAPACITY + 100).len(), LOG_CAPACITY);
        let tail = rt.log_tail(1);
        assert!(tail[0].message.contains(&format!("line {}", LOG_CAPACITY + 49)));
        let _ = std::fs::remove_dir_all(&d);
    }
}
