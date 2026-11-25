//! 内核配置：前端写 JSON，内核只读。

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
pub struct Config {
    /// "host" | "socks"
    #[serde(default = "d_kind")]
    pub kind: String,
    /// host：TLS 监听端口
    #[serde(default = "d_port")]
    pub listen_port: u16,
    /// host：HTTP 监听端口（0 = 不监听）
    #[serde(default = "d_http_port")]
    pub http_port: u16,
    /// socks：SOCKS5 端口
    #[serde(default = "d_socks_port")]
    pub socks_port: u16,
    #[serde(default)]
    pub domains: Vec<String>,
    /// record | segment | off
    #[serde(default = "d_mode")]
    pub fragment_mode: String,
    #[serde(default = "d_parts")]
    pub fragment_parts: u8,
    #[serde(default = "d_dns")]
    pub dns_server: String,
    #[serde(default = "d_true")]
    pub log_connections: bool,
}

fn d_kind() -> String {
    "host".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            kind: d_kind(),
            listen_port: d_port(),
            http_port: d_http_port(),
            socks_port: d_socks_port(),
            domains: Vec::new(),
            fragment_mode: d_mode(),
            fragment_parts: d_parts(),
            dns_server: d_dns(),
            log_connections: true,
        }
    }
}

impl Config {
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

    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.kind != "host" && self.kind != "socks" {
            errors.push(format!("kind 只能是 host 或 socks，当前：{}", self.kind));
        }
        if self.domains.is_empty() {
            errors.push("domains 不能为空".to_string());
        }
        if self.dns_server.trim().is_empty() {
            errors.push("dns_server 不能为空".to_string());
        }
        if self.kind == "socks" && self.socks_port == 0 {
            errors.push("socks_port 不能为 0".to_string());
        }
        if self.kind == "host" && self.listen_port == 0 {
            errors.push("listen_port 不能为 0".to_string());
        }
        errors
    }
}
