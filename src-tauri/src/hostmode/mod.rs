//! 内核进程管理：前端只负责写配置、拉起 `uuengine`、转发日志、停止它。
//!
//! 这与 Clash Verge 的做法一致：UI 不实现数据面，内核是独立进程。

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

pub mod elevate;
pub mod selftest;

/// hosts 读写在 core 里，这里直接转出给命令层用。
pub use uurelaybridge_core::sni::hosts;

fn d_true() -> bool {
    true
}
fn d_port() -> u16 {
    443
}
fn d_http_port() -> u16 {
    80
}
fn d_socks_port() -> u16 {
    7899
}
fn d_mode() -> String {
    "record".to_string()
}
fn d_parts() -> u8 {
    4
}
fn d_dns() -> String {
    "223.5.5.5".to_string()
}

/// 前端维护的配置；写盘后交给内核。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "d_port")]
    pub listen_port: u16,
    #[serde(default = "d_http_port")]
    pub http_port: u16,
    #[serde(default = "d_socks_port")]
    pub socks_port: u16,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default = "d_mode")]
    pub fragment_mode: String,
    #[serde(default = "d_parts")]
    pub fragment_parts: u8,
    #[serde(default = "d_dns")]
    pub dns_server: String,
    #[serde(default = "d_true")]
    pub link_clash: bool,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen_port: d_port(),
            http_port: d_http_port(),
            socks_port: d_socks_port(),
            domains: uurelaybridge_core::presets::uu_domains(),
            fragment_mode: d_mode(),
            fragment_parts: d_parts(),
            dns_server: d_dns(),
            link_clash: true,
        }
    }
}

impl HostConfig {
    pub fn sanitize(&mut self) {
        if self.fragment_parts < 2 {
            self.fragment_parts = 2;
        }
        if self.fragment_parts > 12 {
            self.fragment_parts = 12;
        }
        if !matches!(self.fragment_mode.as_str(), "record" | "segment" | "off") {
            self.fragment_mode = d_mode();
        }
        if self.dns_server.trim().is_empty() {
            self.dns_server = d_dns();
        }
        if self.socks_port == 0 {
            self.socks_port = d_socks_port();
        }
        self.domains = self
            .domains
            .iter()
            .map(|d| d.trim().trim_start_matches("*.").to_ascii_lowercase())
            .filter(|d| !d.is_empty())
            .collect();
        self.domains.dedup();
    }

    pub fn matches(&self, host: &str) -> bool {
        let h = host.trim_end_matches('.').to_ascii_lowercase();
        self.domains
            .iter()
            .any(|d| h == *d || h.ends_with(&format!(".{}", d)))
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ServiceStatus {
    pub running: bool,
    pub socks: Option<String>,
    pub https: Option<String>,
    pub http: Option<String>,
    pub last_error: Option<String>,
    pub hosts_applied: bool,
    pub hosts_domains: Vec<String>,
    pub can_write_hosts: bool,
    pub conns: u64,
    pub fragmented: u64,
    pub failed: u64,
    pub bytes_up: u64,
    pub bytes_down: u64,
}

impl ServiceStatus {
    pub fn view(&self) -> String {
        if !self.running {
            return "未启动".to_string();
        }
        if let Some(s) = &self.socks {
            return format!("SOCKS {s}");
        }
        [
            self.https.as_ref().map(|a| format!("TLS {a}")),
            self.http.as_ref().map(|a| format!("HTTP {a}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
    }
}

struct Inner {
    child: Option<Child>,
    socks: Option<String>,
    https: Option<String>,
    http: Option<String>,
    last_error: Option<String>,
    conns: u64,
    fragmented: u64,
}

fn inner() -> &'static Mutex<Inner> {
    static I: OnceLock<Mutex<Inner>> = OnceLock::new();
    I.get_or_init(|| {
        Mutex::new(Inner {
            child: None,
            socks: None,
            https: None,
            http: None,
            last_error: None,
            conns: 0,
            fragmented: 0,
        })
    })
}

pub fn config_path(root: &Path) -> PathBuf {
    root.join("hostmode.json")
}

pub fn load_config(root: &Path) -> HostConfig {
    let mut cfg: HostConfig = std::fs::read_to_string(config_path(root))
        .ok()
        .and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default();
    cfg.sanitize();
    cfg
}

pub fn save_config(root: &Path, cfg: &HostConfig) -> std::io::Result<()> {
    std::fs::write(
        config_path(root),
        serde_json::to_string_pretty(cfg).unwrap_or_default(),
    )
}

/// 内核可执行文件：优先同目录，其次 resources/ 子目录。
pub fn engine_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let names = ["uuengine.exe", "uuengine"];
    for base in [dir.to_path_buf(), dir.join("resources")] {
        for n in names {
            let p = base.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

pub fn status() -> ServiceStatus {
    let mut g = match inner().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    // 子进程意外退出时更新状态
    if let Some(child) = g.child.as_mut() {
        if let Ok(Some(code)) = child.try_wait() {
            g.child = None;
            g.last_error = Some(format!("内核已退出（code {code}）"));
            g.socks = None;
            g.https = None;
            g.http = None;
        }
    }
    ServiceStatus {
        running: g.child.is_some(),
        socks: g.socks.clone(),
        https: g.https.clone(),
        http: g.http.clone(),
        last_error: g.last_error.clone(),
        hosts_applied: uurelaybridge_core::sni::hosts::is_applied(),
        hosts_domains: uurelaybridge_core::sni::hosts::current_domains(),
        can_write_hosts: can_write_hosts(),
        conns: g.conns,
        fragmented: g.fragmented,
        failed: 0,
        bytes_up: 0,
        bytes_down: 0,
    }
}

pub fn can_write_hosts() -> bool {
    std::fs::OpenOptions::new()
        .append(true)
        .open(uurelaybridge_core::sni::hosts::hosts_path())
        .is_ok()
}

pub fn stop() {
    let mut g = match inner().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if let Some(mut child) = g.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    g.socks = None;
    g.https = None;
    g.http = None;
}

/// 拉起内核。`mode` = "clash"（socks）或 "host"。
pub fn start(
    mode: &str,
    cfg: &HostConfig,
    root: &Path,
    log: std::sync::Arc<dyn Fn(&str) + Send + Sync>,
) -> Result<ServiceStatus, String> {
    stop();

    let engine = engine_path().ok_or_else(|| {
        "找不到内核 uuengine（应与主程序放在同一目录，或在 resources/ 下）".to_string()
    })?;

    // 写内核配置（YAML，保持简洁，用户也可以手改）
    let kind = if mode == "host" { "host" } else { "socks" };
    let fragment = match cfg.fragment_mode.as_str() {
        "off" => "off".to_string(),
        "segment" => format!("tcp:{}", cfg.fragment_parts),
        _ => cfg.fragment_parts.to_string(),
    };
    let mut yaml = String::new();
    yaml.push_str("# generated by UURelayBridge; you may edit it and restart the service.\n");
    yaml.push_str(&format!("mode: {kind}\n"));
    yaml.push_str("domains:\n");
    for d in &cfg.domains {
        yaml.push_str(&format!("  - {d}\n"));
    }
    yaml.push_str(&format!("socks: {}\n", cfg.socks_port));
    yaml.push_str(&format!("listen: {}\n", cfg.listen_port));
    yaml.push_str(&format!("http: {}\n", cfg.http_port));
    yaml.push_str(&format!("fragment: {fragment}\n"));
    yaml.push_str(&format!("dns: {}\n", cfg.dns_server));
    // 前端自己会检查 hosts 冲突并提示，交给内核再拦一次
    yaml.push_str("check_hosts: false\n");

    // 配置默认放在内核同目录（引擎自己找的也是这里）；不可写就退回数据目录。
    // 如果同目录那份被用户手改过（没有生成标记），就不覆盖，改用数据目录那份。
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let mut cfg_path = root.join("engine.yaml");
    if let Some(dir) = exe_dir {
        let candidate = dir.join("engine.yaml");
        let ours = match std::fs::read_to_string(&candidate) {
            Ok(old) => old.contains("由 UURelayBridge 生成") || !old.contains("mode:"),
            Err(_) => true, // 不存在：可以写
        };
        if ours && std::fs::write(&candidate, &yaml).is_ok() {
            cfg_path = candidate;
        }
    }
    if cfg_path == root.join("engine.yaml") {
        std::fs::write(&cfg_path, &yaml).map_err(|e| format!("写内核配置失败：{e}"))?;
    }

    let mut cmd = Command::new(&engine);
    cmd.arg("run")
        .arg("--config")
        .arg(&cfg_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Windows：内核是控制台程序，不加 CREATE_NO_WINDOW 会弹出一个黑框 CLI 窗口。
    // 日志走管道进 GUI，不需要那个窗口。
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动内核失败（{}）：{e}", engine.display()))?;


    if let Some(out) = child.stdout.take() {
        let log = log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if !line.trim().is_empty() {
                    log(&line);
                }
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        let log = log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                if !line.trim().is_empty() {
                    log(&line);
                }
            }
        });
    }

    // 等一小会儿，确认没有立刻退出
    std::thread::sleep(std::time::Duration::from_millis(600));
    if let Ok(Some(code)) = child.try_wait() {
        return Err(format!("内核启动后立即退出（code {code}），请查看日志"));
    }

    let mut g = match inner().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    g.child = Some(child);
    g.last_error = None;
    g.conns = 0;
    g.fragmented = 0;
    if kind == "socks" {
        g.socks = Some(format!("127.0.0.1:{}", cfg.socks_port));
        g.https = None;
        g.http = None;
    } else {
        g.socks = None;
        g.https = Some(format!("127.0.0.1:{}", cfg.listen_port));
        g.http = if cfg.http_port > 0 {
            Some(format!("127.0.0.1:{}", cfg.http_port))
        } else {
            None
        };
    }
    Ok(status())
}
