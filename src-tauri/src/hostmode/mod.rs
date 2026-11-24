//! 本地服务：SOCKS5（Clash 模式）与 80/443 监听（Host 模式）。
//!
//! - Clash 模式：本机起 SOCKS5，Clash 把目标域名交给它；TLS ClientHello 在这里分片。
//! - Host 模式：监听 127.0.0.1:443（TLS，分片）与 :80（HTTP，按 Host 头直连）。
//!
//! 两种模式共用同一套"读 ClientHello → 查真实 IP → 分片 → 透传"逻辑。

pub mod elevate;

pub use uurelaybridge_core::sni::{dns, hosts, tls};

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Host 模式：TLS 监听端口（默认 443）
    #[serde(default = "d_port")]
    pub listen_port: u16,
    /// Host 模式：HTTP 监听端口（默认 80，0 = 不监听）
    #[serde(default = "d_http_port")]
    pub http_port: u16,
    /// Clash 模式：本地 SOCKS5 端口
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
    /// 是否把同一份域名/参数写进 Clash 脚本
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
            domains: uurelaybridge_core::presets::uu_endpoints(),
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

#[derive(Debug, Default)]
pub struct Stats {
    pub conns: AtomicU64,
    pub fragmented: AtomicU64,
    pub failed: AtomicU64,
    pub bytes_up: AtomicU64,
    pub bytes_down: AtomicU64,
}

#[derive(Debug, Clone, Serialize)]
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
            "未启动".to_string()
        } else if let Some(s) = &self.socks {
            format!("SOCKS {s}")
        } else {
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
}

#[derive(Default)]
struct Inner {
    stop: Option<Arc<AtomicBool>>,
    socks: Option<String>,
    https: Option<String>,
    http: Option<String>,
    last_error: Option<String>,
    stats: Arc<Stats>,
}

fn inner() -> &'static Mutex<Inner> {
    static I: OnceLock<Mutex<Inner>> = OnceLock::new();
    I.get_or_init(|| Mutex::new(Inner::default()))
}

type Logger = Arc<dyn Fn(&str) + Send + Sync>;

pub fn status() -> ServiceStatus {
    let g = match inner().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    ServiceStatus {
        running: g.stop.is_some(),
        socks: g.socks.clone(),
        https: g.https.clone(),
        http: g.http.clone(),
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

pub fn stop() {
    let mut g = match inner().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if let Some(s) = g.stop.take() {
        s.store(true, Ordering::SeqCst);
    }
    g.socks = None;
    g.https = None;
    g.http = None;
}

/// 启动服务。`mode` = "clash"（SOCKS）或 "host"（80/443 监听）。
pub fn start(mode: &str, cfg: HostConfig, log: Logger) -> Result<ServiceStatus, String> {
    stop();

    let mut g = match inner().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stats = g.stats.clone();
    let cfg = Arc::new(cfg);

    let mut socks = None;
    let mut https = None;
    let mut http = None;

    if mode == "clash" {
        let addr = format!("127.0.0.1:{}", cfg.socks_port);
        let listener = TcpListener::bind(&addr).map_err(|e| bind_hint(&addr, e))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("设置非阻塞失败：{e}"))?;
        spawn_acceptor(listener, stop_flag.clone(), cfg.clone(), stats.clone(), log.clone(), Kind::Socks);
        socks = Some(addr);
        log(&format!("SOCKS5 已监听 {socks:?}"));
    } else {
        let a = format!("127.0.0.1:{}", cfg.listen_port);
        match TcpListener::bind(&a) {
            Ok(l) => {
                l.set_nonblocking(true).ok();
                spawn_acceptor(l, stop_flag.clone(), cfg.clone(), stats.clone(), log.clone(), Kind::Https);
                https = Some(a.clone());
                log(&format!("TLS 转发已监听 {a}"));
            }
            Err(e) => {
                let msg = bind_hint(&a, e);
                g.last_error = Some(msg.clone());
                return Err(msg);
            }
        }
        if cfg.http_port > 0 {
            let h = format!("127.0.0.1:{}", cfg.http_port);
            match TcpListener::bind(&h) {
                Ok(l) => {
                    l.set_nonblocking(true).ok();
                    spawn_acceptor(l, stop_flag.clone(), cfg.clone(), stats.clone(), log.clone(), Kind::Http);
                    http = Some(h.clone());
                    log(&format!("HTTP 转发已监听 {h}"));
                }
                Err(e) => log(&format!("HTTP 端口 {h} 未监听：{e}")),
            }
        }
    }

    g.stop = Some(stop_flag);
    g.socks = socks.clone();
    g.https = https.clone();
    g.http = http.clone();
    g.last_error = None;
    Ok(status())
}

fn bind_hint(addr: &str, e: std::io::Error) -> String {
    let hint = if e.kind() == std::io::ErrorKind::PermissionDenied {
        "（该端口需要管理员权限）"
    } else if e.kind() == std::io::ErrorKind::AddrInUse {
        "（端口被占用，可能已有实例在运行）"
    } else {
        ""
    };
    format!("监听 {addr} 失败{hint}：{e}")
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Socks,
    Https,
    Http,
}

fn spawn_acceptor(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    cfg: Arc<HostConfig>,
    stats: Arc<Stats>,
    log: Logger,
    kind: Kind,
) {
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, peer)) => {
                    stats.conns.fetch_add(1, Ordering::Relaxed);
                    let _ = stream.set_nonblocking(false);
                    let cfg = cfg.clone();
                    let stats = stats.clone();
                    let log = log.clone();
                    std::thread::spawn(move || {
                        let r = match kind {
                            Kind::Socks => socks_conn(stream, &cfg, stats.clone(), &log),
                            Kind::Https => https_conn(stream, &cfg, stats.clone(), &log),
                            Kind::Http => http_conn(stream, &cfg, stats.clone(), &log),
                        };
                        if let Err(e) = r {
                            stats.failed.fetch_add(1, Ordering::Relaxed);
                            log(&format!("{peer} 处理失败：{e}"));
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
    });
}

// ---------------------------------------------------------------- 三种入口

fn https_conn(
    mut client: TcpStream,
    cfg: &HostConfig,
    stats: Arc<Stats>,
    log: &Logger,
) -> Result<(), String> {
    client.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let _ = client.set_nodelay(true);

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
            return Ok(());
        }
        Err(e) => return Err(format!("读取记录头失败：{e}")),
    }
    if hdr[0] != 0x16 {
        log(&format!("非 TLS 握手（首字节 0x{:02x}），断开", hdr[0]));
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

    let Some(sni) = tls::parse_sni(&record) else {
        log("没有解析出 SNI，断开");
        return Ok(());
    };
    relay(client, &record, &sni, 443, cfg, stats, log)
}

fn socks_conn(
    mut client: TcpStream,
    cfg: &HostConfig,
    stats: Arc<Stats>,
    log: &Logger,
) -> Result<(), String> {
    client.set_read_timeout(Some(Duration::from_secs(15))).ok();
    let _ = client.set_nodelay(true);

    // 1) 握手：05 NMETHODS METHODS...
    let mut head = [0u8; 2];
    client
        .read_exact(&mut head)
        .map_err(|e| format!("SOCKS 握手读取失败：{e}"))?;
    if head[0] != 0x05 {
        return Err("不是 SOCKS5".to_string());
    }
    let mut methods = vec![0u8; head[1] as usize];
    client
        .read_exact(&mut methods)
        .map_err(|e| format!("SOCKS 方法读取失败：{e}"))?;
    client
        .write_all(&[0x05, 0x00])
        .map_err(|e| format!("SOCKS 握手回复失败：{e}"))?;

    // 2) 请求：05 CMD RSV ATYP ADDR PORT
    let mut req = [0u8; 4];
    client
        .read_exact(&mut req)
        .map_err(|e| format!("SOCKS 请求读取失败：{e}"))?;
    if req[1] != 0x01 {
        let _ = client.write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
        return Err("仅支持 CONNECT".to_string());
    }
    let host = match req[3] {
        0x01 => {
            let mut a = [0u8; 4];
            client.read_exact(&mut a).map_err(|e| e.to_string())?;
            IpAddr::from(a).to_string()
        }
        0x03 => {
            let mut l = [0u8; 1];
            client.read_exact(&mut l).map_err(|e| e.to_string())?;
            let mut name = vec![0u8; l[0] as usize];
            client.read_exact(&mut name).map_err(|e| e.to_string())?;
            String::from_utf8_lossy(&name).to_string()
        }
        0x04 => {
            let mut a = [0u8; 16];
            client.read_exact(&mut a).map_err(|e| e.to_string())?;
            IpAddr::from(a).to_string()
        }
        other => return Err(format!("未知地址类型 {other}")),
    };
    let mut port_b = [0u8; 2];
    client.read_exact(&mut port_b).map_err(|e| e.to_string())?;
    let port = u16::from_be_bytes(port_b);

    // 3) 成功回复（0.0.0.0:0）
    client
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .map_err(|e| format!("SOCKS 回复失败：{e}"))?;

    // 4) 读第一条 TLS 记录用于判断是否分片
    let mut hdr = [0u8; 5];
    client
        .read_exact(&mut hdr)
        .map_err(|e| format!("读取记录头失败：{e}"))?;
    if hdr[0] != 0x16 {
        return Err(format!("SOCKS 目标不是 TLS（首字节 0x{:02x}）", hdr[0]));
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

    let sni = tls::parse_sni(&record).unwrap_or_else(|| host.clone());
    relay(client, &record, &sni, port, cfg, stats, log)
}

fn http_conn(
    mut client: TcpStream,
    cfg: &HostConfig,
    stats: Arc<Stats>,
    log: &Logger,
) -> Result<(), String> {
    client.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = client
            .read(&mut tmp)
            .map_err(|e| format!("读取 HTTP 请求失败：{e}"))?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 16384 {
            break;
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let host = text
        .lines()
        .find_map(|l| {
            let lower = l.to_ascii_lowercase();
            lower.strip_prefix("host:").map(|v| v.trim().to_string())
        })
        .ok_or_else(|| "HTTP 请求没有 Host 头".to_string())?;
    let host = host.split(':').next().unwrap_or(&host).to_string();

    let ip = resolve_fast(&host, &cfg.dns_server).ok_or_else(|| format!("{host} 解析失败"))?;
    let mut remote = TcpStream::connect_timeout(
        &SocketAddr::new(ip, 80),
        Duration::from_secs(6),
    )
    .map_err(|e| format!("连接 {host} 失败：{e}"))?;
    remote
        .write_all(&buf)
        .map_err(|e| format!("转发 HTTP 请求失败：{e}"))?;
    log(&format!("{host} → {ip}:80（HTTP 直连）"));
    pump(client, remote, stats)
}

fn relay(
    client: TcpStream,
    record: &[u8],
    host: &str,
    port: u16,
    cfg: &HostConfig,
    stats: Arc<Stats>,
    log: &Logger,
) -> Result<(), String> {
    let ip = resolve_fast(host, &cfg.dns_server)
        .ok_or_else(|| format!("{host} 解析失败（DNS 未返回 A 记录）"))?;
    let mut remote = TcpStream::connect_timeout(
        &SocketAddr::new(ip, port),
        Duration::from_secs(6),
    )
    .map_err(|e| format!("连接 {host}（{ip}）失败：{e}"))?;
    let _ = remote.set_nodelay(true);

    let matched = cfg.matches(host);
    let fragmented = matched && cfg.fragment_mode != "off";
    if fragmented && cfg.fragment_mode == "record" {
        let parts = tls::split_record(record, cfg.fragment_parts as usize);
        for p in &parts {
            remote
                .write_all(p)
                .map_err(|e| format!("发送分片失败：{e}"))?;
        }
        log(&format!(
            "{host} → {ip}（TLS Record 分片 {} 段）",
            parts.len()
        ));
    } else if fragmented {
        let parts = tls::segment_chunks(record, cfg.fragment_parts as usize);
        for (i, p) in parts.iter().enumerate() {
            remote
                .write_all(p)
                .map_err(|e| format!("发送分段失败：{e}"))?;
            if i + 1 < parts.len() {
                std::thread::sleep(Duration::from_millis(4));
            }
        }
        log(&format!("{host} → {ip}（TCP 分段 {} 段）", parts.len()));
    } else {
        remote
            .write_all(record)
            .map_err(|e| format!("转发 ClientHello 失败：{e}"))?;
        if matched {
            log(&format!("{host} → {ip}（未分片）"));
        }
    }
    if fragmented {
        stats.fragmented.fetch_add(1, Ordering::Relaxed);
    }
    pump(client, remote, stats)
}

fn resolve_fast(host: &str, preferred: &str) -> Option<IpAddr> {
    let servers = resolver_candidates(preferred);
    let (tx, rx) = std::sync::mpsc::channel();
    for server in &servers {
        let host = host.to_string();
        let server = server.clone();
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(dns::resolve_any(&host, &server));
        });
    }
    drop(tx);
    let deadline = std::time::Instant::now() + Duration::from_millis(3500);
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

fn resolver_candidates(preferred: &str) -> Vec<String> {
    let mut v = vec![preferred.trim().to_string()];
    if dns::is_doh(preferred) {
        return v;
    }
    for s in ["223.5.5.5", "119.29.29.29"] {
        if !v.iter().any(|x| x == s) {
            v.push(s.to_string());
        }
    }
    v.retain(|s| !s.is_empty());
    v
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
