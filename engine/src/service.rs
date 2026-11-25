//! 服务实现：Host（80/443 监听）与 SOCKS5。
//!
//! 链路：客户端 → 本内核 → 读 ClientHello 取 SNI → DNS 查真实 IP（绕过 hosts）
//!       → 按配置分片 → 转发 → 双向透传（不终止 TLS）。

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
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
static DEBUG_ON: AtomicU64 = AtomicU64::new(0);

/// 打开 DEBUG 级别（配置里 log_level: debug 时调用）。
#[allow(dead_code)]
pub fn set_debug(on: bool) {
    DEBUG_ON.store(if on { 1 } else { 0 }, Ordering::Relaxed);
}

pub fn log(msg: &str) {
    log_level("INFO", msg);
}

/// 只在 log_level: debug 下输出的细节（逐连接、逐个候选 IP）。
pub fn debug(msg: &str) {
    if DEBUG_ON.load(Ordering::Relaxed) == 1 {
        log_level("DEBUG", msg);
    }
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

    // 日志文件：进程内保持一个追加句柄（避免每条日志 open/close）；
    // 打开失败（例如装到 Program Files）就只输出到 stdout。
    use std::io::Write as _;
    static FILE: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();
    let slot = FILE.get_or_init(|| {
        let f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path())
            .ok();
        Mutex::new(f)
    });
    if let Ok(mut g) = slot.lock() {
        if g.is_none() {
            *g = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path())
                .ok();
        }
        if let Some(f) = g.as_mut() {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
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
    // 阻塞 accept：不忙等、不占 CPU。停止进程时由操作系统回收监听套接字。
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                stats.conns.fetch_add(1, Ordering::Relaxed);
                let cfg = cfg.clone();
                let conn_stats = stats.clone();
                let failed_stats = stats.clone();
                let builder = std::thread::Builder::new().stack_size(256 * 1024);
                let spawned = builder.spawn(move || {
                    let _ = s.set_nonblocking(false);
                    let _ = s.set_nodelay(true);
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        match kind {
                            Kind::Socks => socks_conn(s, &cfg, conn_stats.clone()),
                            Kind::Https => https_conn(s, &cfg, conn_stats.clone()),
                            Kind::Http => http_conn(s, &cfg, conn_stats.clone()),
                        }
                    }))
                    .unwrap_or_else(|_| {
                        log_level("ERROR", "connection handler panicked; connection dropped");
                        Ok(())
                    });
                    if let Err(e) = r {
                        conn_stats.failed.fetch_add(1, Ordering::Relaxed);
                        log_level("WARN", &format!("connection failed: {e}"));
                    }
                });
                if spawned.is_err() {
                    // 极端并发下线程创建失败：记录告警，避免连接被静默丢弃
                    log_level("WARN", "cannot spawn handler thread; connection dropped");
                    failed_stats.failed.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(e) => {
                // 单个连接出错不应导致整个监听退出
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::Interrupted
                ) {
                    continue;
                }
                log_level("WARN", &format!("accept failed: {e}"));
                std::thread::sleep(Duration::from_millis(50));
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
        debug(&format!("SOCKS CONNECT reply sent: {host}:{port}"));
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
        debug(&format!("SOCKS target {host}:{port} sni={sni} first-record={}B", record.len()));
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
    let (_, outcome) = pump(client, remote, None, stats);
    match outcome {
        PumpOutcome::Done => Ok(()),
        PumpOutcome::Failed(e) => Err(e),
    }
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
    let mut client = Some(client);
    for (idx, ip) in candidates.iter().enumerate() {
        let Some(c) = client.take() else { break };
        match connect_probe(&payload, *ip, port) {
            Ok(Some((remote, first))) => {
                if cfg.log_connections {
                    log(&format!("{host} -> {ip} ({how})"));
                }
                if matched {
                    stats.fragmented.fetch_add(1, Ordering::Relaxed);
                }
                let (back, outcome) = pump(c, remote, Some(first), stats.clone());
                match outcome {
                    PumpOutcome::Done => return Ok(()),
                    PumpOutcome::Failed(e) => {
                        // 还没给客户端写过数据 → 这条线路不可用，换下一个候选
                        mark_bad(host, *ip);
                        log_level("WARN", &format!("{host}: {ip} unusable ({e})"));
                        last = e;
                        client = Some(back);
                    }
                }
            }
            Ok(None) => {
                mark_bad(host, *ip);
                log_level("WARN", &format!("{host}: no response from {ip}"));
                last = format!("{ip} no response");
                client = Some(c);
            }
            Err(e) => {
                mark_bad(host, *ip);
                log_level("WARN", &format!("{host}: {e} ({ip})"));
                last = e;
                client = Some(c);
            }
        }
        if idx + 1 < candidates.len() {
            debug(&format!("{host}: retry candidate #{}", idx + 2));
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

/// 连一个候选 IP 并发出首包。
///
/// 不在这里等服务器响应：等待会额外增加一个 RTT 量级的延迟，而且对
/// "连接成功但服务器慢" 的正常情况是纯浪费。线路是否真的可用，交给
/// 后续真实读操作判断（读不到数据就是不可用，此时换下一个候选）。
fn connect_send(payload: &[u8], ip: IpAddr, port: u16) -> Result<TcpStream, String> {
    let mut remote = TcpStream::connect_timeout(&SocketAddr::new(ip, port), Duration::from_secs(5))
        .map_err(|e| format!("connect failed: {e}"))?;
    let _ = remote.set_nodelay(true);
    // 开启 keepalive：防止中间设备静默丢弃长连接状态
    enable_keepalive(&remote);
    remote
        .write_all(payload)
        .map_err(|e| format!("send failed: {e}"))?;
    Ok(remote)
}

/// 连上并发出首包后，等服务器的第一个字节（超时 2.5s）。
/// 返回 Ok(None) 表示这条线路不可用（黑洞/瞬断），应换下一个候选。
fn connect_probe(payload: &[u8], ip: IpAddr, port: u16) -> Result<Option<(TcpStream, Vec<u8>)>, String> {
    let mut remote = connect_send(payload, ip, port)?;
    remote
        .set_read_timeout(Some(Duration::from_millis(2500)))
        .map_err(|e| e.to_string())?;
    let mut buf = [0u8; 8192];
    let r = match remote.read(&mut buf) {
        Ok(0) => None,
        Ok(n) => Some(buf[..n].to_vec()),
        Err(_) => None,
    };
    remote.set_read_timeout(None).ok();
    Ok(r.map(|first| (remote, first)))
}

/// 开启 TCP keepalive（平台相关，失败不影响功能）。
#[cfg(windows)]
fn enable_keepalive(sock: &TcpStream) {
    use std::os::windows::io::AsRawSocket;
    #[repr(C)]
    struct TcpKeepalive {
        onoff: u32,
        keepalivetime: u32,
        keepaliveinterval: u32,
    }
    #[link(name = "ws2_32")]
    extern "system" {
        fn WSAIoctl(
            s: usize,
            code: u32,
            inbuf: *mut core::ffi::c_void,
            inlen: u32,
            outbuf: *mut core::ffi::c_void,
            outlen: u32,
            bytes: *mut u32,
            overlapped: *mut core::ffi::c_void,
            completion: *mut core::ffi::c_void,
        ) -> i32;
    }
    const SIO_KEEPALIVE_VALS: u32 = 0x9800_0004;
    let mut ka = TcpKeepalive {
        onoff: 1,
        keepalivetime: 30_000,
        keepaliveinterval: 5_000,
    };
    unsafe {
        let mut bytes = 0u32;
        WSAIoctl(
            sock.as_raw_socket() as usize,
            SIO_KEEPALIVE_VALS,
            &mut ka as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<TcpKeepalive>() as u32,
            std::ptr::null_mut(),
            0,
            &mut bytes,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
    }
}

#[cfg(unix)]
fn enable_keepalive(sock: &TcpStream) {
    use std::os::fd::AsRawFd;
    #[link(name = "c")]
    extern "C" {
        fn setsockopt(
            fd: i32,
            level: i32,
            name: i32,
            val: *const core::ffi::c_void,
            len: u32,
        ) -> i32;
    }
    const SOL_SOCKET: i32 = 0xffff;
    const SO_KEEPALIVE: i32 = 0x0008;
    let one: i32 = 1;
    unsafe {
        setsockopt(
            sock.as_raw_fd(),
            SOL_SOCKET,
            SO_KEEPALIVE,
            &one as *const i32 as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }
}

#[cfg(not(any(windows, unix)))]
fn enable_keepalive(_sock: &TcpStream) {}

/// 已知可用的兜底 IP（校园 TLS 修复项目公开清单，MIT）。
const FALLBACK_IPS: &[(&str, &str)] = &[
    ("api.nrd.nie.163.com", "112.13.123.98"),
    ("api-ipv4.nrd.nie.163.com", "112.13.123.98"),
    ("api-dcdn.nrd.nie.163.com", "117.148.172.102"),
    ("api-event.nrd.nie.163.com", "117.147.201.23"),
    ("uuyc.163.com", "59.111.44.89"),
    ("uuyc.webapp.163.com", "117.148.172.102"),
    ("nrd-ugc.fp.ps.netease.com", "111.3.79.64"),
    ("fp.ps.netease.com", "111.3.79.64"),
];

/// 排除 fake-ip（198.18.0.0/15）和回环/链路本地地址。
fn usable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(o[0] == 127
                || o[0] == 0
                || (o[0] == 169 && o[1] == 254)
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19)))
        }
        IpAddr::V6(v6) => !v6.is_loopback(),
    }
}

const MAX_CACHE: usize = 512;

fn cache() -> &'static Mutex<std::collections::HashMap<String, (Vec<IpAddr>, std::time::Instant)>> {
    static C: OnceLock<Mutex<std::collections::HashMap<String, (Vec<IpAddr>, std::time::Instant)>>> =
        OnceLock::new();
    C.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn bad_ips() -> &'static Mutex<std::collections::HashMap<String, std::time::Instant>> {
    static B: OnceLock<Mutex<std::collections::HashMap<String, std::time::Instant>>> =
        OnceLock::new();
    B.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn mark_bad(host: &str, ip: IpAddr) {
    if let Ok(mut g) = bad_ips().lock() {
        if g.len() > MAX_CACHE {
            g.retain(|_, exp| *exp > std::time::Instant::now());
        }
        g.insert(
            format!("{host}|{ip}"),
            std::time::Instant::now() + Duration::from_secs(300),
        );
    }
    debug(&format!("{host}: {ip} marked bad for 5 min"));
}

/// 把最近失败的 IP 挪到候选末尾（不是删除：全都失败时仍可回退）。
fn filter_bad(host: &str, list: Vec<IpAddr>) -> Vec<IpAddr> {
    let now = std::time::Instant::now();
    let mut good = Vec::with_capacity(list.len());
    let mut bad = Vec::new();
    if let Ok(g) = bad_ips().lock() {
        for ip in list {
            let is_bad = g
                .get(&format!("{host}|{ip}"))
                .map(|exp| *exp > now)
                .unwrap_or(false);
            if is_bad {
                bad.push(ip);
            } else {
                good.push(ip);
            }
        }
    } else {
        return list;
    }
    good.extend(bad);
    good
}

/// 解析目标域名的候选 IP：首选 DNS 与公共 DoH 并行，合并去重，5 分钟缓存。
fn resolve_candidates(host: &str, preferred: &str) -> Vec<IpAddr> {
    if let Ok(ip) = host.trim().parse::<IpAddr>() {
        return vec![ip];
    }

    let key = host.trim_end_matches('.').to_ascii_lowercase();
    if let Ok(g) = cache().lock() {
        if let Some((list, exp)) = g.get(&key) {
            if *exp > std::time::Instant::now() {
                return filter_bad(&key, list.clone());
            }
        }
    }

    let mut servers = vec![preferred.trim().to_string()];
    for url in [
        "https://223.5.5.5/dns-query",
        "https://1.1.1.1/dns-query",
        "https://cloudflare-dns.com/dns-query",
        "https://dns.google/dns-query",
    ] {
        if !servers.iter().any(|s| s == url) {
            servers.push(url.to_string());
        }
    }

    let (tx, rx) = std::sync::mpsc::channel();
    for server in servers {
        let host = host.to_string();
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(dns::resolve_all(&host, &server));
        });
    }
    drop(tx);

    let mut out: Vec<IpAddr> = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_millis(2000);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            break;
        }
        match rx.recv_timeout(left) {
            Ok(Ok(list)) => {
                for v4 in list {
                    let ip = IpAddr::V4(v4);
                    if usable(ip) && !out.contains(&ip) {
                        out.push(ip);
                    }
                }
                if out.len() >= 2 {
                    break;
                }
            }
            Ok(Err(_)) => continue,
            Err(_) => break,
        }
    }

    for ip in family_fallback(host) {
        if let Ok(ip) = ip.parse::<IpAddr>() {
            if !out.contains(&ip) {
                out.push(ip);
            }
        }
    }

    if !out.is_empty() {
        if let Ok(mut g) = cache().lock() {
            if g.len() > MAX_CACHE {
                g.retain(|_, (_, exp)| *exp > std::time::Instant::now());
            }
            g.insert(
                key.clone(),
                (
                    out.clone(),
                    std::time::Instant::now() + Duration::from_secs(300),
                ),
            );
        }
    }
    debug(&format!("{host}: {} candidate IP(s)", out.len()));
    filter_bad(&key, out)
}

/// UU 节点名会变（sig-3207-g / relay-mg-3207-g…），按前缀给兜底。
fn family_fallback(host: &str) -> Vec<&'static str> {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    if h.ends_with(".nrd.nie.163.com") || h == "nrd.nie.163.com" {
        if h.starts_with("sig-") {
            return vec!["34.95.122.33"];
        }
        if h.starts_with("relay-mg-") {
            return vec!["117.147.201.40"];
        }
        if h.starts_with("api") {
            return vec!["112.13.123.98"];
        }
    }
    for (name, ip) in FALLBACK_IPS {
        if h == *name {
            return vec![*ip];
        }
    }
    Vec::new()
}


/// 透传结果：Failed 表示"还没给客户端写过任何数据就失败了"，可以换 IP 重试。
enum PumpOutcome {
    Done,
    Failed(String),
}

fn pump(
    mut client: TcpStream,
    remote: TcpStream,
    pre: Option<Vec<u8>>,
    stats: Arc<Stats>,
) -> (TcpStream, PumpOutcome) {
    // 握手阶段设的读超时必须在透传前清掉：否则空闲超过该时长，
    // 读线程会得到 WouldBlock 而提前结束，整个连接被我们主动 abort。
    let _ = client.set_read_timeout(None);
    let _ = client.set_write_timeout(None);
    let (mut c2, mut r2) = match (client.try_clone(), remote.try_clone()) {
        (Ok(a), Ok(b)) => (a, b),
        _ => return (client, PumpOutcome::Done),
    };
    let up_stats = stats.clone();

    let up_thread = std::thread::spawn(move || {
        let mut buf = [0u8; 65536];
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
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        up_stats.bytes_up.fetch_add(total, Ordering::Relaxed);
        let _ = r2.shutdown(std::net::Shutdown::Write);
    });

    // 探测阶段已经读到的服务器首段，先写回客户端
    let mut total = 0u64;
    let mut wrote_to_client = false;
    if let Some(bytes) = pre {
        if !bytes.is_empty() {
            if client.write_all(&bytes).is_err() {
                let _ = up_thread.join();
                return (client, PumpOutcome::Done);
            }
            total += bytes.len() as u64;
            wrote_to_client = true;
        }
    }

    let mut buf = [0u8; 65536];
    let mut r = remote;
    let mut last_err: Option<String> = None;
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if client.write_all(&buf[..n]).is_err() {
                    break;
                }
                total += n as u64;
                wrote_to_client = true;
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                last_err = Some(e.to_string());
                break;
            }
        }
    }
    stats.bytes_down.fetch_add(total, Ordering::Relaxed);
    // 下行结束时先半关闭写方向，让对端有机会把剩余数据读完
    let _ = client.shutdown(std::net::Shutdown::Write);
    let _ = up_thread.join();
    let _ = client.shutdown(std::net::Shutdown::Both);

    // 一个字节都没收到就失败 → 允许换 IP 重试；已经通过数据就不能再换
    match last_err {
        Some(e) if !wrote_to_client => (client, PumpOutcome::Failed(e)),
        _ => (client, PumpOutcome::Done),
    }
}
