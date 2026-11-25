//! TLS ClientHello 解析与分片。
//!
//! 只做"读一条记录 → 取 SNI → 重新切成多条记录"，不终止 TLS。

/// 从一条 TLS 记录里取出 ClientHello 的 SNI。
pub fn parse_sni(record: &[u8]) -> Option<String> {
    parse_sni_ex(record).map(|(name, _)| name)
}

/// 同 [`parse_sni`]，另外返回域名在**整条记录**里的起始偏移。
/// 用于「切在域名中间」这种更抗 DPI 的分片方式。
pub fn parse_sni_ex(record: &[u8]) -> Option<(String, usize)> {
    if record.len() < 9 || record[0] != 0x16 {
        return None; // 不是 handshake
    }
    let rec_len = u16::from_be_bytes([record[3], record[4]]) as usize;
    let body = &record[5..(5 + rec_len).min(record.len())];
    if body.len() < 4 || body[0] != 0x01 {
        return None; // 不是 ClientHello
    }
    let hs_len = ((body[1] as usize) << 16) | ((body[2] as usize) << 8) | body[3] as usize;
    let hs = &body[4..(4 + hs_len).min(body.len())];
    // 记录头 5 字节 + 握手头 4 字节
    let hs_base = 9;

    let mut i = 2 + 32; // legacy_version + random
    let sid = *hs.get(i)? as usize;
    i += 1 + sid;
    let cs_len = u16::from_be_bytes([*hs.get(i)?, *hs.get(i + 1)?]) as usize;
    i += 2 + cs_len;
    let cm_len = *hs.get(i)? as usize;
    i += 1 + cm_len;
    if i + 2 > hs.len() {
        return None;
    }
    let ext_len = u16::from_be_bytes([hs[i], hs[i + 1]]) as usize;
    i += 2;
    let end = (i + ext_len).min(hs.len());

    while i + 4 <= end {
        let ty = u16::from_be_bytes([hs[i], hs[i + 1]]);
        let l = u16::from_be_bytes([hs[i + 2], hs[i + 3]]) as usize;
        let data_start = i + 4;
        let data = &hs[data_start..(data_start + l).min(hs.len())];
        if ty == 0x0000 && data.len() >= 5 {
            let name_len = u16::from_be_bytes([data[3], data[4]]) as usize;
            if data.len() >= 5 + name_len {
                let name = String::from_utf8_lossy(&data[5..5 + name_len]).to_string();
                if !name.is_empty() {
                    return Some((name.to_ascii_lowercase(), hs_base + data_start + 5));
                }
            }
        }
        i += 4 + l;
    }
    None
}

/// 「切在域名中间」：把 ClientHello 拆成 2 条记录，切点落在 SNI 域名的中央。
///
/// 这样任何单条记录里都不会出现完整域名，抗"只重组第一个记录/单记录匹配"的 DPI 更有效。
/// 返回 None 表示没解析出 SNI（调用方退回普通分片）。
pub fn split_mid_sni(record: &[u8]) -> Option<Vec<Vec<u8>>> {
    let (name, name_off) = parse_sni_ex(record)?;
    let body_start = 5;
    let name_len = name.len();
    if name_len < 2 || name_off < body_start {
        return None;
    }
    // 域名中点（相对整条记录），再换算成 payload 内偏移
    let mid = name_off + name_len / 2;
    let payload_split = mid.saturating_sub(body_start);
    if payload_split == 0 || payload_split >= record.len() - body_start {
        return None;
    }
    Some(split_at(record, payload_split))
}

/// 在 payload 的 `at` 处切成两条记录。
fn split_at(record: &[u8], at: usize) -> Vec<Vec<u8>> {
    let header = &record[..5];
    let body = &record[5..];
    let at = at.min(body.len());
    let mut first = Vec::with_capacity(5 + at);
    first.extend_from_slice(header);
    first[3..5].copy_from_slice(&(at as u16).to_be_bytes());
    first.extend_from_slice(&body[..at]);

    let rest = body.len() - at;
    let mut second = Vec::with_capacity(5 + rest);
    second.extend_from_slice(header);
    second[3..5].copy_from_slice(&(rest as u16).to_be_bytes());
    second.extend_from_slice(&body[at..]);

    vec![first, second]
}

/// 把一条 TLS 记录按 `parts` 份重新封装成多条记录（握手消息跨记录是合法的）。
pub fn split_record(record: &[u8], parts: usize) -> Vec<Vec<u8>> {
    if record.len() <= 5 || parts < 2 {
        return vec![record.to_vec()];
    }
    let header = &record[..5];
    let body = &record[5..];
    let n = parts.clamp(2, 12).min(body.len());
    let chunk = (body.len() + n - 1) / n;

    let mut out = Vec::with_capacity(n);
    for piece in body.chunks(chunk) {
        let mut rec = Vec::with_capacity(5 + piece.len());
        rec.extend_from_slice(header);
        rec[3..5].copy_from_slice(&(piece.len() as u16).to_be_bytes());
        rec.extend_from_slice(piece);
        out.push(rec);
    }
    out
}

/// TCP 分段模式：把整条记录切成若干段（调用方负责逐段写出并加延迟）。
pub fn segment_chunks(record: &[u8], parts: usize) -> Vec<Vec<u8>> {
    let n = parts.clamp(2, 32).min(record.len().max(1));
    let chunk = (record.len() + n - 1) / n;
    record.chunks(chunk.max(1)).map(|c| c.to_vec()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手工构造一个带 SNI 的 ClientHello 记录（格式合法即可，不参与加密）。
    fn client_hello(sni: &str) -> Vec<u8> {
        let name = sni.as_bytes();
        let mut ext = Vec::new();
        // server_name
        ext.extend_from_slice(&[0x00, 0x00]);
        let list_len = 1 + 2 + name.len() as u16;
        ext.extend_from_slice(&(list_len + 2).to_be_bytes());
        ext.extend_from_slice(&(list_len as u16).to_be_bytes());
        ext.push(0x00);
        ext.extend_from_slice(&(name.len() as u16).to_be_bytes());
        ext.extend_from_slice(name);

        let mut hs = Vec::new();
        hs.extend_from_slice(&[0x03, 0x03]); // legacy_version
        hs.extend_from_slice(&[0u8; 32]); // random
        hs.push(0); // session id len
        hs.extend_from_slice(&[0x00, 0x02, 0x13, 0x01]); // cipher suites
        hs.extend_from_slice(&[0x01, 0x00]); // compression
        hs.extend_from_slice(&(ext.len() as u16).to_be_bytes());
        hs.extend_from_slice(&ext);

        let mut body = vec![0x01];
        let l = hs.len();
        body.extend_from_slice(&[(l >> 16) as u8, (l >> 8) as u8, l as u8]);
        body.extend_from_slice(&hs);

        let mut rec = vec![0x16, 0x03, 0x01];
        rec.extend_from_slice(&(body.len() as u16).to_be_bytes());
        rec.extend_from_slice(&body);
        rec
    }

    #[test]
    fn parses_sni() {
        let rec = client_hello("api.nrd.nie.163.com");
        assert_eq!(parse_sni(&rec).as_deref(), Some("api.nrd.nie.163.com"));
    }

    #[test]
    fn rejects_non_tls() {
        assert!(parse_sni(b"GET / HTTP/1.1\r\n").is_none());
        assert!(parse_sni(&[0x16, 0x03, 0x01, 0x00, 0x04, 0x02, 0x00, 0x00, 0x00]).is_none());
    }

    #[test]
    fn split_keeps_total_payload() {
        let rec = client_hello("example.com");
        let parts = split_record(&rec, 4);
        assert!(parts.len() >= 2);
        let body_len: usize = parts.iter().map(|p| p.len() - 5).sum();
        assert_eq!(body_len, rec.len() - 5);
        for p in &parts {
            assert_eq!(p[0], 0x16);
            let l = u16::from_be_bytes([p[3], p[4]]) as usize;
            assert_eq!(l, p.len() - 5);
        }
    }

    #[test]
    fn segments_cover_record() {
        let rec = client_hello("example.com");
        let segs = segment_chunks(&rec, 6);
        assert!(segs.len() >= 2);
        assert_eq!(segs.concat(), rec);
    }

    #[test]
    fn mid_sni_split_breaks_the_name() {
        let rec = client_hello("api.nrd.nie.163.com");
        let parts = split_mid_sni(&rec).expect("应能按 SNI 中点切分");
        assert_eq!(parts.len(), 2);
        // 两条记录的 payload 拼起来必须等于原始 payload（握手消息跨记录合法）
        let joined: Vec<u8> = parts.iter().flat_map(|p| p[5..].to_vec()).collect();
        assert_eq!(joined, rec[5..].to_vec());
        // 关键：任何一条记录里都不能出现完整域名
        for p in &parts {
            let text = String::from_utf8_lossy(&p[5..]).to_string();
            assert!(
                !text.contains("api.nrd.nie.163.com"),
                "单条记录里不该有完整域名"
            );
        }
        // 两条记录都是合法 handshake 记录
        for p in &parts {
            assert_eq!(p[0], 0x16);
            let l = u16::from_be_bytes([p[3], p[4]]) as usize;
            assert_eq!(l, p.len() - 5);
        }
    }

    #[test]
    fn mid_sni_split_keeps_sni_parsable() {
        let rec = client_hello("sig-3303-d.nrd.nie.163.com");
        let parts = split_mid_sni(&rec).unwrap();
        let rejoined: Vec<u8> = parts.iter().flat_map(|p| p[5..].to_vec()).collect();
        let mut whole = rec[..5].to_vec();
        whole[3..5].copy_from_slice(&(rejoined.len() as u16).to_be_bytes());
        whole.extend_from_slice(&rejoined);
        assert_eq!(
            parse_sni(&whole).as_deref(),
            Some("sig-3303-d.nrd.nie.163.com"),
            "拼回去必须还能解析出 SNI"
        );
    }

    #[test]
    fn mid_sni_split_none_without_sni() {
        let rec = vec![0x16, 0x03, 0x01, 0x00, 0x04, 0x02, 0x00, 0x00, 0x00];
        assert!(split_mid_sni(&rec).is_none());
    }
}
