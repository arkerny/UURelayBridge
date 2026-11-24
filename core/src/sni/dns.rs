//! 极简 DNS A 记录查询 + 缓存。
//!
//! 为什么要自己查：Hosts 把域名指到了 127.0.0.1，系统解析器会读到 Hosts 里的 127.0.0.1，
//! 直接 connect 就会绕回自己。这里绕过 Hosts，直接问 DNS 服务器要真实地址。

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct Entry {
    addrs: Vec<Ipv4Addr>,
    expires: Instant,
}

fn cache() -> &'static Mutex<HashMap<String, Entry>> {
    static C: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 查询 A 记录。`server` 形如 `223.5.5.5`，可带 `:53`。
pub fn resolve(host: &str, server: &str) -> Result<Vec<Ipv4Addr>, String> {
    let key = format!("{}|{}", host, server);
    if let Ok(g) = cache().lock() {
        if let Some(e) = g.get(&key) {
            if e.expires > Instant::now() {
                return Ok(e.addrs.clone());
            }
        }
    }

    let addrs = query(host, server)?;
    if addrs.is_empty() {
        return Err(format!("DNS 没有返回 A 记录：{host}"));
    }
    if let Ok(mut g) = cache().lock() {
        g.insert(
            key,
            Entry {
                addrs: addrs.clone(),
                expires: Instant::now() + Duration::from_secs(60),
            },
        );
    }
    Ok(addrs)
}

pub fn resolve_one(host: &str, server: &str) -> Result<IpAddr, String> {
    resolve(host, server)
        .map(|v| IpAddr::V4(v[0]))
        .map_err(|e| e)
}

fn build_query(host: &str, id: u16) -> Vec<u8> {
    let mut q = Vec::with_capacity(512);
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00]); // RD
    q.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    q.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // AN/NS/AR
    for label in host.trim_end_matches('.').split('.') {
        let b = label.as_bytes();
        q.push(b.len() as u8);
        q.extend_from_slice(b);
    }
    q.push(0);
    q.extend_from_slice(&1u16.to_be_bytes()); // A
    q.extend_from_slice(&1u16.to_be_bytes()); // IN
    q
}

fn query(host: &str, server: &str) -> Result<Vec<Ipv4Addr>, String> {
    let addr = if server.contains(':') {
        server.to_string()
    } else {
        format!("{}:53", server)
    };
    let sa: SocketAddr = addr
        .parse()
        .map_err(|_| format!("DNS 服务器地址无效：{server}"))?;

    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("创建 UDP 套接字失败：{e}"))?;
    sock.set_read_timeout(Some(Duration::from_secs(2)))
        .ok();
    sock.set_write_timeout(Some(Duration::from_secs(2))).ok();

    let id = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0) & 0xFFFF) as u16;
    let q = build_query(host, id);
    sock.send_to(&q, sa)
        .map_err(|e| format!("发送 DNS 查询失败：{e}"))?;

    let mut buf = [0u8; 1500];
    let (n, _) = sock
        .recv_from(&mut buf)
        .map_err(|e| format!("DNS 查询超时/失败：{e}"))?;
    let resp = &buf[..n];

    parse_response(resp, id).ok_or_else(|| "DNS 响应无法解析".to_string())
}

fn parse_response(resp: &[u8], id: u16) -> Option<Vec<Ipv4Addr>> {
    if resp.len() < 12 {
        return None;
    }
    if u16::from_be_bytes([resp[0], resp[1]]) != id {
        return None;
    }
    let flags = u16::from_be_bytes([resp[2], resp[3]]);
    if flags & 0x8000 == 0 {
        return None; // 不是响应
    }
    let qd = u16::from_be_bytes([resp[4], resp[5]]) as usize;
    let an = u16::from_be_bytes([resp[6], resp[7]]) as usize;

    let mut i = 12;
    for _ in 0..qd {
        i = skip_name(resp, i)?;
        i += 4; // type + class
    }

    let mut out = Vec::new();
    for _ in 0..an {
        i = skip_name(resp, i)?;
        if i + 10 > resp.len() {
            return None;
        }
        let ty = u16::from_be_bytes([resp[i], resp[i + 1]]);
        let rdlen = u16::from_be_bytes([resp[i + 8], resp[i + 9]]) as usize;
        i += 10;
        if i + rdlen > resp.len() {
            return None;
        }
        if ty == 1 && rdlen == 4 {
            out.push(Ipv4Addr::new(
                resp[i],
                resp[i + 1],
                resp[i + 2],
                resp[i + 3],
            ));
        }
        i += rdlen;
    }
    Some(out)
}

fn skip_name(buf: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let b = *buf.get(i)?;
        if b == 0 {
            return Some(i + 1);
        }
        if b & 0xC0 == 0xC0 {
            return Some(i + 2); // 压缩指针
        }
        i += 1 + b as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_encodes_name() {
        let q = build_query("a.b.com", 0x1234);
        assert_eq!(&q[0..2], &[0x12, 0x34]);
        assert_eq!(q[12], 1); // 第一个标签长度
        assert_eq!(&q[13..17], b"a\x01b\x03");
    }

    #[test]
    fn parses_a_response() {
        // 手工构造：id, flags(响应), 1 question, 1 answer
        let mut r = vec![0x12, 0x34, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0, 0, 0, 0];
        r.extend_from_slice(&[3, b'w', b'w', b'w', 7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0]);
        r.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
        r.extend_from_slice(&[0xC0, 0x0C]); // 指针
        r.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
        r.extend_from_slice(&[0x00, 0x00, 0x00, 0x3C]); // ttl
        r.extend_from_slice(&[0x00, 0x04, 93, 184, 216, 34]);
        let addrs = parse_response(&r, 0x1234).unwrap();
        assert_eq!(addrs, vec![Ipv4Addr::new(93, 184, 216, 34)]);
    }
}
