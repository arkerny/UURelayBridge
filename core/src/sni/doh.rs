//! DoH（DNS over HTTPS，RFC 8484）。
//!
//! 不引入额外 TLS 依赖：
//! - Windows：走系统 WinHTTP（自带 TLS/证书/代理设置）；
//! - 其它平台：调用系统 curl。
//!
//! 只实现 POST `application/dns-message` 这一种最小可用形式。

#[cfg(not(windows))]
use std::io::Read;
#[cfg(not(windows))]
use std::process::{Command, Stdio};

/// 简单拆 URL：scheme://host[:port]/path
fn split_url(url: &str) -> Result<(bool, String, u16, String), String> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| format!("DoH 地址必须以 http:// 或 https:// 开头：{url}"))?;
    let secure = match scheme.to_ascii_lowercase().as_str() {
        "https" => true,
        "http" => false,
        other => return Err(format!("不支持的协议：{other}")),
    };
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/dns-query"),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => (
            h.to_string(),
            p.parse::<u16>().map_err(|_| "端口无效".to_string())?,
        ),
        _ => (hostport.to_string(), if secure { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err("DoH 地址缺少主机名".to_string());
    }
    Ok((secure, host, port, path.to_string()))
}

pub fn query(url: &str, wire: &[u8], timeout_ms: u32) -> Result<Vec<u8>, String> {
    let (secure, host, port, path) = split_url(url)?;
    doh_request(url, secure, &host, port, &path, wire, timeout_ms)
}

#[cfg(windows)]
fn doh_request(
    _url: &str,
    secure: bool,
    host: &str,
    port: u16,
    path: &str,
    wire: &[u8],
    timeout_ms: u32,
) -> Result<Vec<u8>, String> {
    win::post(secure, host, port, path, wire, timeout_ms)
}

#[cfg(not(windows))]
fn doh_request(
    url: &str,
    _secure: bool,
    _host: &str,
    _port: u16,
    _path: &str,
    wire: &[u8],
    timeout_ms: u32,
) -> Result<Vec<u8>, String> {
    let mut child = Command::new("curl")
        .arg("-sS")
        .arg("--max-time")
        .arg((timeout_ms / 1000).max(1).to_string())
        .arg("-H")
        .arg("content-type: application/dns-message")
        .arg("-H")
        .arg("accept: application/dns-message")
        .arg("--data-binary")
        .arg("@-")
        .arg(url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("调用 curl 失败：{e}"))?;

    child
        .stdin
        .as_mut()
        .ok_or("无法写入 curl 标准输入")?
        .write_all(wire)
        .map_err(|e| format!("写入请求失败：{e}"))?;

    let mut out = Vec::new();
    child
        .stdout
        .as_mut()
        .ok_or("无法读取 curl 输出")?
        .read_to_end(&mut out)
        .ok();
    let status = child.wait().map_err(|e| format!("等待 curl 失败：{e}"))?;
    if !status.success() {
        return Err("DoH 请求失败（curl 非零退出）".to_string());
    }
    if out.is_empty() {
        return Err("DoH 返回空响应".to_string());
    }
    Ok(out)
}

#[cfg(windows)]
mod win {
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;

    type Handle = *mut c_void;

    #[link(name = "winhttp")]
    extern "system" {
        fn WinHttpOpen(
            agent: *const u16,
            access: u32,
            proxy: *const u16,
            bypass: *const u16,
            flags: u32,
        ) -> Handle;
        fn WinHttpConnect(session: Handle, host: *const u16, port: u16, reserved: u32) -> Handle;
        fn WinHttpOpenRequest(
            conn: Handle,
            verb: *const u16,
            path: *const u16,
            version: *const u16,
            referrer: *const u16,
            accept: *const *const u16,
            flags: u32,
        ) -> Handle;
        fn WinHttpSendRequest(
            req: Handle,
            headers: *const u16,
            headers_len: u32,
            body: *mut u8,
            body_len: u32,
            total: u32,
            ctx: usize,
        ) -> i32;
        fn WinHttpReceiveResponse(req: Handle, reserved: *mut c_void) -> i32;
        fn WinHttpReadData(req: Handle, buf: *mut u8, len: u32, read: *mut u32) -> i32;
        fn WinHttpQueryHeaders(
            req: Handle,
            info: u32,
            name: *const u16,
            buf: *mut c_void,
            len: *mut u32,
            index: *mut u32,
        ) -> i32;
        fn WinHttpSetTimeouts(session: Handle, resolve: i32, connect: i32, send: i32, recv: i32)
            -> i32;
        fn WinHttpCloseHandle(h: Handle) -> i32;
    }

    const FLAG_SECURE: u32 = 0x0080_0000;
    // WINHTTP_QUERY_STATUS_CODE(19) | WINHTTP_QUERY_FLAG_NUMBER(0x20000000)
    const QUERY_STATUS_CODE: u32 = 19 | 0x2000_0000;
    const DEFAULT_PROXY: u32 = 0;

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    struct Guard(Handle);
    impl Drop for Guard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    WinHttpCloseHandle(self.0);
                }
            }
        }
    }

    pub fn post(
        secure: bool,
        host: &str,
        port: u16,
        path: &str,
        wire: &[u8],
        timeout_ms: u32,
    ) -> Result<Vec<u8>, String> {
        let agent = wide("UURelayBridge/1.4");
        let session = unsafe { WinHttpOpen(agent.as_ptr(), DEFAULT_PROXY, std::ptr::null(), std::ptr::null(), 0) };
        if session.is_null() {
            return Err("WinHttpOpen 失败".to_string());
        }
        let session = Guard(session);
        let t = timeout_ms as i32;
        unsafe {
            WinHttpSetTimeouts(session.0, t, t, t, t);
        }

        let host_w = wide(host);
        let conn = unsafe { WinHttpConnect(session.0, host_w.as_ptr(), port, 0) };
        if conn.is_null() {
            return Err(format!("WinHttpConnect 失败：{host}:{port}"));
        }
        let conn = Guard(conn);

        let verb = wide("POST");
        let path_w = wide(path);
        let flags = if secure { FLAG_SECURE } else { 0 };
        let req = unsafe {
            WinHttpOpenRequest(
                conn.0,
                verb.as_ptr(),
                path_w.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                flags,
            )
        };
        if req.is_null() {
            return Err("WinHttpOpenRequest 失败".to_string());
        }
        let req = Guard(req);

        let headers = wide("Content-Type: application/dns-message\r\nAccept: application/dns-message\r\n");
        let mut body = wire.to_vec();
        let ok = unsafe {
            WinHttpSendRequest(
                req.0,
                headers.as_ptr(),
                u32::MAX, // -1：以 NUL 结尾
                body.as_mut_ptr(),
                body.len() as u32,
                body.len() as u32,
                0,
            )
        };
        if ok == 0 {
            return Err("WinHttpSendRequest 失败（连接被拒绝或超时）".to_string());
        }
        if unsafe { WinHttpReceiveResponse(req.0, std::ptr::null_mut()) } == 0 {
            return Err("WinHttpReceiveResponse 失败".to_string());
        }

        // 状态码
        let mut code: u32 = 0;
        let mut len = std::mem::size_of::<u32>() as u32;
        let ok = unsafe {
            WinHttpQueryHeaders(
                req.0,
                QUERY_STATUS_CODE,
                std::ptr::null(),
                &mut code as *mut u32 as *mut c_void,
                &mut len,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || code != 200 {
            return Err(format!("DoH 返回 HTTP {code}"));
        }

        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let mut read: u32 = 0;
            let ok = unsafe {
                WinHttpReadData(req.0, buf.as_mut_ptr(), buf.len() as u32, &mut read)
            };
            if ok == 0 || read == 0 {
                break;
            }
            out.extend_from_slice(&buf[..read as usize]);
            if out.len() > 64 * 1024 {
                break;
            }
        }
        if out.is_empty() {
            return Err("DoH 返回空响应".to_string());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_url() {
        let (secure, host, port, path) = split_url("https://dns.alidns.com/dns-query").unwrap();
        assert!(secure);
        assert_eq!(host, "dns.alidns.com");
        assert_eq!(port, 443);
        assert_eq!(path, "/dns-query");
    }

    #[test]
    fn parses_port_and_path() {
        let (secure, host, port, path) = split_url("http://127.0.0.1:8053/dns-query").unwrap();
        assert!(!secure);
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 8053);
        assert_eq!(path, "/dns-query");
    }

    #[test]
    fn defaults_path() {
        let (_s, _h, _p, path) = split_url("https://example.com").unwrap();
        assert_eq!(path, "/dns-query");
    }

    #[test]
    fn rejects_bad_url() {
        assert!(split_url("dns.alidns.com/dns-query").is_err());
        assert!(split_url("ftp://x/y").is_err());
    }
}
