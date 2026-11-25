//! 服务实现：Host（80/443 监听）与 SOCKS5。
//!
//! 链路：客户端 → 本内核 → 读 ClientHello 取 SNI → DNS 查真实 IP（绕过 hosts）
//!       → 按配置分片 → 转发 → 双向透传（不终止 TLS）。

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use uurelaybridge_core::sni::{dns, tls};

use crate::config::Config;

#[derive(Default)]
pub struct Stats {
    pub conns: AtomicU64,
    pub fragmented: AtomicU64,
    pub failed: AtomicU64,
    pub bytes_up: AtomicU64,
    pub bytes_down: AtomicU64,
}

fn log_path() -> std::path::PathBuf {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    dir.join("uuengine.log")
}

/// 统一日志格式：`HH:MM:SS.mmm [LEVEL] message`，同时写 stdout 和运行目录下的日志文件。
pub fn log(msg: &str) {
    log_level("INFO", msg);
}

pub fn log_level(level: &str, msg: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (h, m, s) = ((now / 3600) % 24, (now / 60) % 60, now % 60);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_millis())
        .unwrap_or(0);
    let line = format!("{h:02}:{m:02}:{s:02}.{millis:03} [{level}] {msg}");

    println!("{line}");
    let _ = std::io::stdout().flush();

    // 日志文件：运行目录不可写时静默跳过（例如装到 Program Files）
    use std::io::Write as _;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    {
        let _ = writeln!(f, "{line}");
    }
}

/// 启动所有监听并阻塞运行。
pub fn run(cfg: Config) -> Result<(), String> {
    let stats = Arc::new(Stats::default());
    let cfg = Arc::new(cfg);

    if cfg.kind == "socks" {
        let addr = format!("127.0.0.1:{}", cfg.socks_port);
        let l = TcpListener::bind(&addr).map_err(|e| bind_err(&addr, e))?;
        log(&format!("SOCKS5 listening on {addr}"));
        accept_loop(l, cfg.clone(), stats.clone(), Kind::Socks)?;
    } else {
        let a = format!("127.0.0.1:{}", cfg.listen_port);
        let l = TcpListener::bind(&a).map_err(|e| bind_err(&a, e))?;
        log(&format!("TLS relay listening on {a}"));
        if cfg.http_port > 0 {
            let h = format!("127.0.0.1:{}", cfg.http_port);
            match TcpListener::bind(&h) {
                Ok(hl) => {
                    log(&format!("HTTP relay listening on {h}"));
                    let c = cfg.clone();
                    let s = stats.clone();
                    std::thread::spawn(move || {
                        if let Err(e) = accept_loop(hl, c, s, Kind::Http) {
                            log_level("WARN", &format!("HTTP listener stopped: {e}"));
                        }
                    });
                }
                Err(e) => log_level("WARN", &format!("HTTP port {h} not bound: {e}")),
            }
        }
        accept_loop(l, cfg.clone(), stats.clone(), Kind::Https)?;
    }
    Ok(())
}

fn bind_err(addr: &str, e: std::io::Error) -> String {
    let hint = match e.kind() {
        std::io::ErrorKind::PermissionDenied => " (needs admin)",
        std::io::ErrorKind::AddrInUse => " (port in use)",
        _ => "",
    };
    format!("bind {addr} failed{hint}: {e}")
}

#[derive(Clone, Copy)]
enum Kind {
    Socks,
    Https,
    Http,
}

fn accept_loop(
    listener: TcpListener,
    cfg: Arc<Config>,
    stats: Arc<Stats>,
    kind: Kind,
) -> Result<(), String> {
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                stats.conns.fetch_add(1, Ordering::Relaxed);
                let cfg = cfg.clone();
                let stats = stats.clone();
                std::thread::spawn(move || {
                    let _ = s.set_nonblocking(false);
                    // 单个连接出任何意外都不允许拖垮整个内核
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        match kind {
                            Kind::Socks => socks_conn(s, &cfg, stats.clone()),
                            Kind::Https => https_conn(s, &cfg, stats.clone()),
                            Kind::Http => http_conn(s, &cfg, stats.clone()),
                        }
                    }))
                    .unwrap_or_else(|_| {
                        log_level("ERROR", "connection handler panicked; connection dropped");
                        Ok(())
                    });
                    if let Err(e) = r {
                        stats.failed.fetch_add(1, Ordering::Relaxed);
                        log_level("WARN", &format!("connection failed: {e}"));
                    }
                });
            }
            Err(e) => {
                log_level("WARN", &format!("accept failed: {e}"));
                std::thread::sleep(Duration::from_millis(150));
            }
        }
    }
    Ok(())
}

fn https_conn(mut client: TcpStream, cfg: &Config, stats: Arc<Stats>) -> Result<(), String> {
    client.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let _ = client.set_nodelay(true);

    let mut hdr = [0u8; 5];
    if client.read_exact(&mut hdr).is_err() {
        return Ok(());
    }
    if hdr[0] != 0x16 {
        return Ok(()); // 非 TLS，忽略
    }
    let len = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
    if len == 0 || len > 16384 {
        return Err(format!("bad record length: {len}"));
    }
    let mut body = vec![0u8; len];
    client
        .read_exact(&mut body)
        .map_err(|e| format!("read record body failed: {e}"))?;
    let mut record = hdr.to_vec();
    record.extend_from_slice(&body);

    let Some(sni) = tls::parse_sni(&record) else {
        return Ok(());
    };
    relay(client, &record, &sni, &sni, 443, cfg, stats)
}

fn socks_conn(mut client: TcpStream, cfg: &Config, stats: Arc<Stats>) -> Result<(), String> {
    client.set_read_timeout(Some(Duration::from_secs(15))).ok();
    let _ = client.set_nodelay(true);

    let mut head = [0u8; 2];
    if client.read_exact(&mut head).is_err() {
        return Ok(());
    }
    if head[0] != 0x05 {
        return Err("not SOCKS5".to_string());
    }
    let mut methods = vec![0u8; head[1] as usize];
    client
        .read_exact(&mut methods)
        .map_err(|e| format!("socks methods read failed: {e}"))?;
    client
        .write_all(&[0x05, 0x00])
        .map_err(|e| format!("socks greeting reply failed: {e}"))?;

    let mut req = [0u8; 4];
    client
        .read_exact(&mut req)
        .map_err(|e| format!("socks request read failed: {e}"))?;
    if req[1] != 0x01 {
        let _ = client.write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
        return Err("only CONNECT is supported".to_string());
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
        other => return Err(format!("unsupported ATYP {other}")),
    };
    let mut port_b = [0u8; 2];
    client.read_exact(&mut port_b).map_err(|e| e.to_string())?;
    let port = u16::from_be_bytes(port_b);

    client
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .map_err(|e| format!("socks reply failed: {e}"))?;
    if cfg.log_connections {
        log(&format!("SOCKS CONNECT reply sent: {host}:{port}"));
    }

    let mut hdr = [0u8; 5];
    client
        .read_exact(&mut hdr)
        .map_err(|e| format!("read record header failed: {e}"))?;
    if hdr[0] != 0x16 {
        return Err(format!("target is not TLS (first byte 0x{:02x})", hdr[0]));
    }
    let len = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
    if len == 0 || len > 16384 {
        return Err(format!("bad record length: {len}"));
    }
    let mut body = vec![0u8; len];
    client
        .read_exact(&mut body)
        .map_err(|e| format!("read record body failed: {e}"))?;
    let mut record = hdr.to_vec();
    record.extend_from_slice(&body);

    let sni = tls::parse_sni(&record).unwrap_or_else(|| host.clone());
    if cfg.log_connections {
        log(&format!("SOCKS target {host}:{port} sni={sni} first-record={}B", record.len()));
    }
    relay(client, &record, &host, &sni, port, cfg, stats)
}

fn http_conn(mut client: TcpStream, cfg: &Config, stats: Arc<Stats>) -> Result<(), String> {
    client.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = client
            .read(&mut tmp)
            .map_err(|e| format!("read HTTP request failed: {e}"))?;
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
        .ok_or_else(|| "HTTP request has no Host header".to_string())?;
    let host = host.split(':').next().unwrap_or(&host).to_string();

    let ip = resolve_candidates(&host, &cfg.dns_server)
        .into_iter()
        .next()
        .ok_or_else(|| format!("{host}: resolve failed"))?;
    let mut remote =
        TcpStream::connect_timeout(&SocketAddr::new(ip, 80), Duration::from_secs(6))
            .map_err(|e| format!("connect {host} failed: {e}"))?;
    remote
        .write_all(&buf)
        .map_err(|e| format!("forward HTTP request failed: {e}"))?;
    log(&format!("{host} -> {ip}:80 (HTTP direct)"));
    pump(client, remote, None, stats)
}

fn relay(
    client: TcpStream,
    record: &[u8],
    connect_host: &str,
    host: &str,
    port: u16,
    cfg: &Config,
    stats: Arc<Stats>,
) -> Result<(), String> {
    let candidates = resolve_candidates(connect_host, &cfg.dns_server);
    if candidates.is_empty() {
        return Err(format!("{host}: no usable A record"));
    }

    let matched = cfg.matches(host);
    let fragmented = matched && cfg.fragment.mode != "off";
    let (payload, how) = build_payload(record, cfg, fragmented);

    let mut last = String::from("no usable IP");
    for (idx, ip) in candidates.iter().enumerate() {
        match connect_probe(&payload, *ip, port) {
            Ok(Some((remote, first))) => {
                if cfg.log_connections {
                    log(&format!("{host} -> {ip} ({how})"));
                }
                if matched {
                    stats.fragmented.fetch_add(1, Ordering::Relaxed);
                }
                return pump(client, remote, Some(first), stats);
            }
            Ok(None) => {
                log_level("WARN", &format!("{host}: no response from {ip}, trying next candidate"));
                last = format!("{ip} no response");
            }
            Err(e) => {
                log_level("WARN", &format!("{host}: connect to {ip} failed: {e}"));
                last = e;
            }
        }
        if idx + 1 < candidates.len() && cfg.log_connections {
            log(&format!("{host}: retry candidate #{}", idx + 2));
        }
    }
    Err(format!("{host}: all candidate IPs failed ({last})"))
}

/// 按分片配置生成要发出去的首包（以及给日志用的描述）。
fn build_payload(record: &[u8], cfg: &Config, fragmented: bool) -> (Vec<u8>, String) {
    if fragmented && cfg.fragment.mode == "record" {
        // 优先「切在 SNI 域名中间」拆成 2 条记录：任何一条记录里都不含完整域名
        // （参照 campus-tls-fix 的实现）。
        let (parts, how) = match tls::split_mid_sni(record) {
            Some(p) => (p, "split mid-SNI"),
            None => (
                tls::split_record(record, cfg.fragment.parts as usize),
                "even split",
            ),
        };
        let mut buf = Vec::with_capacity(record.len() + 5 * parts.len());
        for p in &parts {
            buf.extend_from_slice(p);
        }
        (buf, format!("TLS record x{} ({})", parts.len(), how))
    } else if fragmented {
        let parts = tls::segment_chunks(record, cfg.fragment.parts as usize);
        (parts.concat(), format!("TCP segments x{}", parts.len()))
    } else {
        (record.to_vec(), "no fragment".to_string())
    }
}

/// 连一个候选 IP、把首包发出去，然后等服务器的第一个字节。
/// 返回 Some((连接, 已读到的字节)) 表示这条线路是通的。
fn connect_probe(
    payload: &[u8],
    ip: IpAddr,
    port: u16,
) -> Result<Option<(TcpStream, Vec<u8>)>, String> {
    let mut remote = TcpStream::connect_timeout(&SocketAddr::new(ip, port), Duration::from_secs(5))
        .map_err(|e| format!("connect failed: {e}"))?;
    let _ = remote.set_nodelay(true);
    remote
        .write_all(payload)
        .map_err(|e| format!("send failed: {e}"))?;
    log(&format!("  probe sent {}B to {ip}:{port}", payload.len()));

    remote
        .set_read_timeout(Some(Duration::from_secs(4)))
        .map_err(|e| e.to_string())?;
    let mut buf = [0u8; 8192];
    match remote.read(&mut buf) {
        Ok(0) => {
            log_level("WARN", &format!("  {ip} closed immediately (EOF)"));
            Ok(None)
        }
        Ok(n) => {
            log(&format!("  {ip} replied {n}B"));
            remote.set_read_timeout(None).ok();
            Ok(Some((remote, buf[..n].to_vec())))
        }
        Err(e) => {
            log_level("WARN", &format!("  {ip} no reply ({e})"));
            Ok(None)
        }
    }
}

/// 已知可用的兜底 IP（来自 campus-tls-fix，MIT；DNS 被污染/fake-ip 时用）。
const FALLBACK_IPS: &[(&str, &str)] = &[
    ("api.nrd.nie.163.com", "112.13.123.98"),
    ("api-ipv4.nrd.nie.163.com", "112.13.123.98"),
    ("api-dcdn.nrd.nie.163.com", "117.148.172.102"),
    ("api-event.nrd.nie.163.com", "117.147.201.23"),
    ("sig-3303-d.nrd.nie.163.com", "34.95.122.33"),
    ("relay-mg-3303-d.nrd.nie.163.com", "117.147.201.40"),
    ("uuyc.163.com", "59.111.44.89"),
    ("uuyc.webapp.163.com", "117.148.172.102"),
    ("nrd-ugc.fp.ps.netease.com", "111.3.79.64"),
    ("fp.ps.netease.com", "111.3.79.64"),
    ("dns.qq.com", "119.29.29.229"),
    ("dns.weixin.qq.com", "120.204.0.63"),
    ("dns.weixin.qq.com.cn", "120.204.0.63"),
    ("httpdns.kg.qq.com", "117.135.175.48"),
    ("dns.wechat.com", "43.152.112.101"),
];

/// 排除 fake-ip（Clash TUN 常给 198.18.0.0/15）和回环/链路本地地址，
/// 否则我们会连回 TUN，再被规则绕回自己，形成环路。
fn usable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            if o[0] == 127 || o[0] == 0 {
                return false;
            }
            if o[0] == 169 && o[1] == 254 {
                return false;
            }
            // 198.18.0.0/15 → fake-ip
            if o[0] == 198 && (o[1] == 18 || o[1] == 19) {
                return false;
            }
            true
        }
        IpAddr::V6(v6) => !v6.is_loopback(),
    }
}

fn resolve_candidates(host: &str, preferred: &str) -> Vec<IpAddr> {
    // 目标本身是 IP 字面量（SOCKS 客户端常见）：直接连，不做 DNS、不做过滤
    if let Ok(ip) = host.trim().parse::<IpAddr>() {
        return vec![ip];
    }
    let mut out: Vec<IpAddr> = Vec::new();

    // 1) 配置里的 DNS（明文或 DoH），取全部 A 记录
    for server in resolver_candidates(preferred) {
        if let Ok(list) = dns::resolve_all(host, &server) {
            for v4 in list {
                let ip = IpAddr::V4(v4);
                if usable(ip) && !out.contains(&ip) {
                    out.push(ip);
                }
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    if !out.is_empty() {
        return out;
    }

    // 2) 配置的 DNS 给的是 fake-ip 或失败：改用 DoH 重查
    for doh in [
        "https://223.5.5.5/dns-query",
        "https://dns.alidns.com/dns-query",
        "https://1.1.1.1/dns-query",
    ] {
        if let Ok(list) = dns::resolve_all(host, doh) {
            for v4 in list {
                let ip = IpAddr::V4(v4);
                if usable(ip) && !out.contains(&ip) {
                    out.push(ip);
                }
            }
        }
        if !out.is_empty() {
            log(&format!("{host}: resolved {} IP(s) via DoH", out.len()));
            break;
        }
    }

    // 3) 内置兜底 IP 放到最后再试
    let key = host.trim_end_matches('.').to_ascii_lowercase();
    for (name, ip) in FALLBACK_IPS {
        if key == *name {
            if let Ok(ip) = ip.parse::<IpAddr>() {
                if !out.contains(&ip) {
                    out.push(ip);
                }
            }
        }
    }
    out
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

fn pump(
    mut client: TcpStream,
    remote: TcpStream,
    pre: Option<Vec<u8>>,
    stats: Arc<Stats>,
) -> Result<(), String> {
    let mut c2 = client
        .try_clone()
        .map_err(|e| format!("socket clone failed: {e}"))?;
    let mut r2 = remote
        .try_clone()
        .map_err(|e| format!("socket clone failed: {e}"))?;
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

    // 探测阶段已经读到的服务器首段，先写回客户端
    let mut total = 0u64;
    if let Some(bytes) = pre {
        if !bytes.is_empty() {
            client
                .write_all(&bytes)
                .map_err(|e| format!("write first chunk failed: {e}"))?;
            total += bytes.len() as u64;
        }
    }

    let mut buf = [0u8; 16384];
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
