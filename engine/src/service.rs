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

pub fn log(msg: &str) {
    println!("[uuengine] {msg}");
    let _ = std::io::stdout().flush();
}

/// 启动所有监听并阻塞运行。
pub fn run(cfg: Config) -> Result<(), String> {
    let stats = Arc::new(Stats::default());
    let cfg = Arc::new(cfg);

    if cfg.kind == "socks" {
        let addr = format!("127.0.0.1:{}", cfg.socks_port);
        let l = TcpListener::bind(&addr).map_err(|e| bind_err(&addr, e))?;
        log(&format!("SOCKS5 监听 {addr}"));
        accept_loop(l, cfg.clone(), stats.clone(), Kind::Socks)?;
    } else {
        let a = format!("127.0.0.1:{}", cfg.listen_port);
        let l = TcpListener::bind(&a).map_err(|e| bind_err(&a, e))?;
        log(&format!("TLS 转发监听 {a}"));
        if cfg.http_port > 0 {
            let h = format!("127.0.0.1:{}", cfg.http_port);
            match TcpListener::bind(&h) {
                Ok(hl) => {
                    log(&format!("HTTP 转发监听 {h}"));
                    let c = cfg.clone();
                    let s = stats.clone();
                    std::thread::spawn(move || {
                        if let Err(e) = accept_loop(hl, c, s, Kind::Http) {
                            log(&format!("HTTP 监听结束：{e}"));
                        }
                    });
                }
                Err(e) => log(&format!("HTTP 端口 {h} 未监听：{e}")),
            }
        }
        accept_loop(l, cfg.clone(), stats.clone(), Kind::Https)?;
    }
    Ok(())
}

fn bind_err(addr: &str, e: std::io::Error) -> String {
    let hint = match e.kind() {
        std::io::ErrorKind::PermissionDenied => "（需要管理员权限）",
        std::io::ErrorKind::AddrInUse => "（端口被占用）",
        _ => "",
    };
    format!("监听 {addr} 失败{hint}：{e}")
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
                    let r = match kind {
                        Kind::Socks => socks_conn(s, &cfg, stats.clone()),
                        Kind::Https => https_conn(s, &cfg, stats.clone()),
                        Kind::Http => http_conn(s, &cfg, stats.clone()),
                    };
                    if let Err(e) = r {
                        stats.failed.fetch_add(1, Ordering::Relaxed);
                        log(&format!("连接处理失败：{e}"));
                    }
                });
            }
            Err(e) => {
                log(&format!("accept 失败：{e}"));
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
        return Err(format!("记录长度异常：{len}"));
    }
    let mut body = vec![0u8; len];
    client
        .read_exact(&mut body)
        .map_err(|e| format!("读取记录体失败：{e}"))?;
    let mut record = hdr.to_vec();
    record.extend_from_slice(&body);

    let Some(sni) = tls::parse_sni(&record) else {
        return Ok(());
    };
    relay(client, &record, &sni, 443, cfg, stats)
}

fn socks_conn(mut client: TcpStream, cfg: &Config, stats: Arc<Stats>) -> Result<(), String> {
    client.set_read_timeout(Some(Duration::from_secs(15))).ok();
    let _ = client.set_nodelay(true);

    let mut head = [0u8; 2];
    if client.read_exact(&mut head).is_err() {
        return Ok(());
    }
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

    client
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .map_err(|e| format!("SOCKS 回复失败：{e}"))?;

    let mut hdr = [0u8; 5];
    client
        .read_exact(&mut hdr)
        .map_err(|e| format!("读取记录头失败：{e}"))?;
    if hdr[0] != 0x16 {
        return Err(format!("目标不是 TLS（首字节 0x{:02x}）", hdr[0]));
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
    relay(client, &record, &sni, port, cfg, stats)
}

fn http_conn(mut client: TcpStream, cfg: &Config, stats: Arc<Stats>) -> Result<(), String> {
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

    let ip = resolve_candidates(&host, &cfg.dns_server)
        .into_iter()
        .next()
        .ok_or_else(|| format!("{host} 解析失败"))?;
    let mut remote =
        TcpStream::connect_timeout(&SocketAddr::new(ip, 80), Duration::from_secs(6))
            .map_err(|e| format!("连接 {host} 失败：{e}"))?;
    remote
        .write_all(&buf)
        .map_err(|e| format!("转发 HTTP 请求失败：{e}"))?;
    log(&format!("{host} → {ip}:80（HTTP 直连）"));
    pump(client, remote, None, stats)
}

fn relay(
    client: TcpStream,
    record: &[u8],
    host: &str,
    port: u16,
    cfg: &Config,
    stats: Arc<Stats>,
) -> Result<(), String> {
    let candidates = resolve_candidates(host, &cfg.dns_server);
    if candidates.is_empty() {
        return Err(format!("{host} 解析失败（没有可用的 A 记录）"));
    }

    let matched = cfg.matches(host);
    let fragmented = matched && cfg.fragment.mode != "off";
    let (payload, how) = build_payload(record, cfg, fragmented);

    let mut last = String::from("无可用 IP");
    for (idx, ip) in candidates.iter().enumerate() {
        match connect_probe(&payload, *ip, port) {
            Ok(Some((remote, first))) => {
                if cfg.log_connections {
                    log(&format!("{host} → {ip}（{how}）"));
                }
                if matched {
                    stats.fragmented.fetch_add(1, Ordering::Relaxed);
                }
                return pump(client, remote, Some(first), stats);
            }
            Ok(None) => {
                log(&format!("{host} 在 {ip} 上没有响应，换下一个候选"));
                last = format!("{ip} 无响应");
            }
            Err(e) => {
                log(&format!("{host} 连接 {ip} 失败：{e}"));
                last = e;
            }
        }
        if idx + 1 < candidates.len() && cfg.log_connections {
            log(&format!("{host} 重试候选 #{}", idx + 2));
        }
    }
    Err(format!("{host} 全部候选 IP 失败：{last}"))
}

/// 按分片配置生成要发出去的首包（以及给日志用的描述）。
fn build_payload(record: &[u8], cfg: &Config, fragmented: bool) -> (Vec<u8>, String) {
    if fragmented && cfg.fragment.mode == "record" {
        // 优先「切在 SNI 域名中间」拆成 2 条记录：任何一条记录里都不含完整域名
        // （参照 campus-tls-fix 的实现）。
        let (parts, how) = match tls::split_mid_sni(record) {
            Some(p) => (p, "切在 SNI 中间"),
            None => (
                tls::split_record(record, cfg.fragment.parts as usize),
                "等分",
            ),
        };
        let mut buf = Vec::with_capacity(record.len() + 5 * parts.len());
        for p in &parts {
            buf.extend_from_slice(p);
        }
        (buf, format!("TLS Record {} 段，{}", parts.len(), how))
    } else if fragmented {
        let parts = tls::segment_chunks(record, cfg.fragment.parts as usize);
        (parts.concat(), format!("TCP 分段 {} 段", parts.len()))
    } else {
        (record.to_vec(), "未分片".to_string())
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
        .map_err(|e| format!("连接失败：{e}"))?;
    let _ = remote.set_nodelay(true);
    remote
        .write_all(payload)
        .map_err(|e| format!("发送失败：{e}"))?;

    remote
        .set_read_timeout(Some(Duration::from_secs(4)))
        .map_err(|e| e.to_string())?;
    let mut buf = [0u8; 8192];
    match remote.read(&mut buf) {
        Ok(0) => Ok(None),
        Ok(n) => {
            remote.set_read_timeout(None).ok();
            Ok(Some((remote, buf[..n].to_vec())))
        }
        Err(_) => Ok(None),
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
            log(&format!("{host} 经 DoH 解析到 {} 个 IP", out.len()));
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

fn resolve_via(host: &str, servers: &[String]) -> Option<IpAddr> {
    let (tx, rx) = std::sync::mpsc::channel();
    for server in servers {
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

fn pump(
    mut client: TcpStream,
    remote: TcpStream,
    pre: Option<Vec<u8>>,
    stats: Arc<Stats>,
) -> Result<(), String> {
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

    // 探测阶段已经读到的服务器首段，先写回客户端
    let mut total = 0u64;
    if let Some(bytes) = pre {
        if !bytes.is_empty() {
            client
                .write_all(&bytes)
                .map_err(|e| format!("回写首段失败：{e}"))?;
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
