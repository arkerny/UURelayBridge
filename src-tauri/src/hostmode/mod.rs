//! Host 模式：本地 TLS 转发 + 分片。
//!
//! 链路：应用 → hosts 把域名指向 127.0.0.1 → 本模块的监听端口 → 解析 ClientHello、
//! 取出 SNI → 用 DNS 查真实 IP（绕过被改写的 hosts）→ 分片转发 → 之后双向透传。
//!
//! 注意：这里只做"读一条记录、按域名选择是否分片"，不终止 TLS、不替换证书。

pub mod elevate;

pub use uurelaybridge_core::sni::{dns, hosts, tls};

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub fn config_path(root: &Path) -> PathBuf {
    root.join("hostmode.json")
}

pub fn load_config(root: &Path) -> HostConfig {
    let mut cfg: HostConfig = std::fs::read_to_string(config_path(root))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
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

/// 给前端的一份完整视图：配置 + 运行状态。
#[derive(Debug, Clone, Serialize)]
pub struct HostView {
    pub config: HostConfig,
    pub status: HostStatus,
}

fn d_port() -> u16 {
    443
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
fn d_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "d_port")]
    pub listen_port: u16,
    /// 需要走分片的域名（后缀匹配，写 `nrd.nie.163.com` 即覆盖所有子域）
    #[serde(default)]
    pub domains: Vec<String>,
    /// record = TLS Record 分片；segment = TCP 分段；off = 只转发不分片
    #[serde(default = "d_mode")]
    pub fragment_mode: String,
    #[serde(default = "d_parts")]
    pub fragment_parts: u8,
    #[serde(default = "d_dns")]
    pub dns_server: String,
    /// 是否把同一份域名/参数同时写进 Clash 脚本（dns.hosts + DIRECT 交给本机转发器）
    #[serde(default = "d_true")]
    pub link_clash: bool,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen_port: d_port(),
            domains: Vec::new(),
            fragment_mode: d_mode(),
            fragment_parts: d_parts(),
            dns_server: d_dns(),
            link_clash: true,
        }
    }
}

impl HostConfig {
    pub fn sanitize(&mut self) {
        if self.listen_port < 1024 && self.listen_port != 443 {
            // 允许 443（Windows 上普通用户也能绑），其余低位端口容易失败但不强改
        }
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

#[derive(Debug, Default)]
pub struct Stats {
    pub conns: AtomicU64,
    pub fragmented: AtomicU64,
    pub failed: AtomicU64,
    pub bytes_up: AtomicU64,
    pub bytes_down: AtomicU64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostStatus {
    pub running: bool,
    pub listen: Option<String>,
    pub started_at: Option<String>,
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

#[derive(Default)]
struct Inner {
    stop: Option<Arc<AtomicBool>>,
    listen: Option<String>,
    started_at: Option<String>,
    last_error: Option<String>,
    stats: Arc<Stats>,
}

fn inner() -> &'static Mutex<Inner> {
    static I: OnceLock<Mutex<Inner>> = OnceLock::new();
    I.get_or_init(|| Mutex::new(Inner::default()))
}

type Logger = Arc<dyn Fn(&str) + Send + Sync>;

pub fn status() -> HostStatus {
    let g = match inner().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    HostStatus {
        running: g.stop.is_some(),
        listen: g.listen.clone(),
        started_at: g.started_at.clone(),
        last_error: g.last_error.clone(),
        hosts_applied: hosts::is_applied(),
        hosts_domains: hosts::current_domains(),
        can_write_hosts: can_write_hosts(),
        conns: g.stats.conns.load(Ordering::Relaxed),
        fragmented: g.stats.fragmented.load(Ordering::Relaxed),
        failed: g.stats.failed.load(Ordering::Relaxed),
        bytes_up: g.stats.bytes_up.load(Ordering::Relaxed),
        bytes_down: g.stats.bytes_down.load(Ordering::Relaxed),
    }
}

pub fn can_write_hosts() -> bool {
    std::fs::OpenOptions::new()
        .append(true)
        .open(hosts::hosts_path())
        .is_ok()
}

/// 启动监听。返回实际监听地址。
pub fn start(cfg: HostConfig, log: Logger) -> Result<String, String> {
    let mut g = match inner().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if g.stop.is_some() {
        return Err("Host 模式已经在运行".to_string());
    }

    let addr = format!("127.0.0.1:{}", cfg.listen_port);
    let listener = TcpListener::bind(&addr).map_err(|e| {
        let hint = if e.kind() == std::io::ErrorKind::PermissionDenied {
            "（该端口需要管理员权限，换 8443 或提权运行）"
        } else if e.kind() == std::io::ErrorKind::AddrInUse {
            "（端口被占用：可能已有实例在跑，或被其他程序占用）"
        } else {
            ""
        };
        format!("监听 {addr} 失败{hint}：{e}")
    })?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("设置非阻塞失败：{e}"))?;

    let stop = Arc::new(AtomicBool::new(false));
    let stats = g.stats.clone();
    let cfg = Arc::new(cfg);
    let stop_thread = stop.clone();
    let addr_log = addr.clone();

    std::thread::spawn(move || {
        log(&format!("Host 模式已监听 {addr_log}"));
        while !stop_thread.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, peer)) => {
                    stats.conns.fetch_add(1, Ordering::Relaxed);
                    // Windows 上 accept 出来的 socket 可能继承监听套接字的非阻塞状态
                    let _ = stream.set_nonblocking(false);
                    let cfg = cfg.clone();
                    let stats = stats.clone();
                    let log = log.clone();
                    std::thread::spawn(move || {
                        if let Err(e) = handle_conn(stream, &cfg, stats.clone(), &log) {
                            stats.failed.fetch_add(1, Ordering::Relaxed);
                            log(&format!("连接 {} 处理失败：{e}", peer));
                        }
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(80));
                }
                Err(e) => {
                    log(&format!("accept 失败：{e}"));
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
        log("Host 模式已停止监听");
    });

    g.stop = Some(stop);
    g.listen = Some(addr.clone());
    g.started_at = Some(crate::state::now_rfc3339());
    g.last_error = None;
    Ok(addr)
}

pub fn stop() {
    let mut g = match inner().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if let Some(s) = g.stop.take() {
        s.store(true, Ordering::SeqCst);
    }
    g.listen = None;
    g.started_at = None;
}

fn handle_conn(
    mut client: TcpStream,
    cfg: &HostConfig,
    stats: Arc<Stats>,
    log: &Logger,
) -> Result<(), String> {
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok();
    let _ = client.set_nodelay(true);

    // 1) 读一条完整的 TLS 记录（可能跨多次 read）
    let mut hdr = [0u8; 5];
    match client.read_exact(&mut hdr) {
        Ok(()) => {}
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
            ) =>
        {
            // 探测连接 / 客户端提前关闭，属正常现象
            return Ok(());
        }
        Err(e) => return Err(format!("读取记录头失败：{e}")),
    }
    if hdr[0] != 0x16 {
        log(&format!("非 TLS 握手（首字节 0x{:02x}），直接断开", hdr[0]));
        return Ok(());
    }
    let len = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
    if len == 0 || len > 16384 {
        return Err(format!("记录长度异常：{len}"));
    }
    let mut body = vec![0u8; len];
    client
        .read_exact(&mut body)
        .map_err(|e| format!("读取记录体失败：{e}"))?;
    let mut record = hdr.to_vec();
    record.extend_from_slice(&body);

    // 2) 取 SNI
    let Some(sni) = tls::parse_sni(&record) else {
        log("没有解析出 SNI，直接断开");
        return Ok(());
    };

    // 3) 查真实地址（绕过 hosts）。多个 DNS 并行问，取最先成功的。
    let t_dns = std::time::Instant::now();
    let target = resolve_fast(&sni, &cfg.dns_server);
    let dns_ms = t_dns.elapsed().as_millis();
    let Some(ip) = target else {
        return Err(format!("{sni} 解析失败（DNS 未返回 A 记录）"));
    };

    // 4) 连接真实服务器
    let addr = SocketAddr::new(ip, 443);
    let t_conn = std::time::Instant::now();
    let mut remote = TcpStream::connect_timeout(&addr, Duration::from_secs(6))
        .map_err(|e| format!("连接 {sni}（{ip}）失败：{e}"))?;
    let conn_ms = t_conn.elapsed().as_millis();
    let _ = remote.set_nodelay(true);

    // 5) 按配置分片发出 ClientHello
    let matched = cfg.matches(&sni);
    let fragmented = matched && cfg.fragment_mode != "off";
    if fragmented && cfg.fragment_mode == "record" {
        let parts = tls::split_record(&record, cfg.fragment_parts as usize);
        for p in &parts {
            remote
                .write_all(p)
                .map_err(|e| format!("发送分片失败：{e}"))?;
        }
        log(&format!(
            "{sni} → {ip}（TLS Record 分片 {} 段，共 {} 字节；DNS {dns_ms}ms，连接 {conn_ms}ms）",
            parts.len(),
            record.len()
        ));
    } else if fragmented {
        let parts = tls::segment_chunks(&record, cfg.fragment_parts as usize);
        for (i, p) in parts.iter().enumerate() {
            remote
                .write_all(p)
                .map_err(|e| format!("发送分段失败：{e}"))?;
            if i + 1 < parts.len() {
                std::thread::sleep(Duration::from_millis(4));
            }
        }
        log(&format!(
            "{sni} → {ip}（TCP 分段 {} 段，共 {} 字节）",
            parts.len(),
            record.len()
        ));
    } else {
        remote
            .write_all(&record)
            .map_err(|e| format!("转发 ClientHello 失败：{e}"))?;
        if matched {
            log(&format!("{sni} → {ip}（未分片）"));
        }
    }
    if fragmented {
        stats.fragmented.fetch_add(1, Ordering::Relaxed);
    }

    // 6) 双向透传
    pump(client, remote, stats)
}

fn resolver_candidates(preferred: &str) -> Vec<String> {
    let mut v = vec![preferred.trim().to_string()];
    for s in ["223.5.5.5", "119.29.29.29"] {
        if !v.iter().any(|x| x == s) {
            v.push(s.to_string());
        }
    }
    v.retain(|s| !s.is_empty());
    v
}

/// 并行问多个 DNS，取最先返回的结果；全部失败才返回 None。
fn resolve_fast(host: &str, preferred: &str) -> Option<IpAddr> {
    let servers = resolver_candidates(preferred);
    let (tx, rx) = std::sync::mpsc::channel();
    for server in &servers {
        let host = host.to_string();
        let server = server.clone();
        let tx = tx.clone();
        std::thread::spawn(move || {
            let r = dns::resolve_any(&host, &server);
            let _ = tx.send(r);
        });
    }
    drop(tx);

    let deadline = std::time::Instant::now() + Duration::from_millis(2500);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return None;
        }
        match rx.recv_timeout(left) {
            Ok(Ok(ip)) => return Some(ip),
            Ok(Err(_)) => continue,
            Err(_) => return None,
        }
    }
}

fn pump(mut client: TcpStream, remote: TcpStream, stats: Arc<Stats>) -> Result<(), String> {
    let mut c2 = client
        .try_clone()
        .map_err(|e| format!("复制套接字失败：{e}"))?;
    let mut r2 = remote
        .try_clone()
        .map_err(|e| format!("复制套接字失败：{e}"))?;
    let up_stats = stats.clone();

    let up_thread = std::thread::spawn(move || {
        let mut buf = [0u8; 16384];
        let mut total = 0u64;
        loop {
            match c2.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if r2.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    total += n as u64;
                }
                Err(_) => break,
            }
        }
        up_stats.bytes_up.fetch_add(total, Ordering::Relaxed);
        let _ = r2.shutdown(std::net::Shutdown::Write);
    });

    let mut buf = [0u8; 16384];
    let mut total = 0u64;
    let mut r = remote;
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if client.write_all(&buf[..n]).is_err() {
                    break;
                }
                total += n as u64;
            }
            Err(_) => break,
        }
    }
    stats.bytes_down.fetch_add(total, Ordering::Relaxed);
    let _ = client.shutdown(std::net::Shutdown::Both);
    let _ = up_thread.join();
    Ok(())
}
