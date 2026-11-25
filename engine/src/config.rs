//! 内核配置：YAML，尽量少写。
//!
//! 最小可用：
//! ```yaml
//! mode: socks
//! domains: [nrd.nie.163.com]
//! ```
//!
//! 全部字段（除 mode / domains 外都可省略）：
//! ```yaml
//! mode: socks        # socks | host
//! domains: [a.example.com, b.example.com]
//! socks: 7899        # mode=socks 的端口
//! listen: 443        # mode=host 的 TLS 端口
//! http: 80           # mode=host 的 HTTP 端口，0 关闭
//! fragment: 4        # 4 = TLS 记录分 4 段；off = 不分片；tcp[:N] = TCP 分段
//! dns: 223.5.5.5     # 也可写 https://dns.alidns.com/dns-query
//! ```
//!
//! 旧的 JSON 配置仍然能读（按扩展名判断）。

use serde::Deserialize;

fn d_port() -> u16 {
    443
}
fn d_http() -> u16 {
    80
}
fn d_socks() -> u16 {
    7899
}
fn d_dns() -> String {
    "223.5.5.5".to_string()
}

/// 分片设置：数字 = 记录分片段数；off；tcp 或 tcp:N。
#[derive(Debug, Clone, PartialEq)]
pub struct Fragment {
    pub mode: String, // record | segment | off
    pub parts: u8,
}

impl Default for Fragment {
    fn default() -> Self {
        Self {
            mode: "record".to_string(),
            parts: 4,
        }
    }
}

impl Fragment {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let s = raw.trim().to_ascii_lowercase();
        if s.is_empty() || s == "off" || s == "none" || s == "false" || s == "0" {
            return Ok(Self {
                mode: "off".to_string(),
                parts: 4,
            });
        }
        if let Some(rest) = s.strip_prefix("tcp") {
            let parts = rest
                .trim_start_matches(':')
                .trim()
                .parse::<u8>()
                .unwrap_or(4);
            return Ok(Self {
                mode: "segment".to_string(),
                parts,
            });
        }
        match s.parse::<u8>() {
            Ok(n) => Ok(Self {
                mode: "record".to_string(),
                parts: n,
            }),
            Err(_) => Err(format!(
                "fragment 只能是数字、off 或 tcp[:N]，当前：{raw}"
            )),
        }
    }

    pub fn as_yaml(&self) -> String {
        match self.mode.as_str() {
            "off" => "off".to_string(),
            "segment" => format!("tcp:{}", self.parts),
            _ => self.parts.to_string(),
        }
    }
}

fn de_fragment<'de, D>(d: D) -> Result<Fragment, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let s = String::deserialize(d)?;
    Fragment::parse(&s).map_err(D::Error::custom)
}

fn de_domains<'de, D>(d: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

#[derive(Debug, Deserialize)]
pub struct FileConfig {
    #[serde(default)]
    pub mode: Option<String>,
    /// 兼容旧字段名
    #[serde(default, alias = "kind")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "de_domains")]
    pub domains: Vec<String>,
    #[serde(default)]
    pub socks: Option<u16>,
    #[serde(default, alias = "listen_port")]
    pub listen: Option<u16>,
    #[serde(default, alias = "http_port")]
    pub http: Option<u16>,
    #[serde(default, deserialize_with = "de_fragment")]
    pub fragment: Fragment,
    #[serde(default, alias = "dns_server", alias = "doh")]
    pub dns: Option<String>,
}

/// 运行时配置。
#[derive(Debug, Clone)]
pub struct Config {
    pub kind: String,
    pub listen_port: u16,
    pub http_port: u16,
    pub socks_port: u16,
    pub domains: Vec<String>,
    pub fragment: Fragment,
    pub dns_server: String,
    pub log_connections: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            kind: "host".to_string(),
            listen_port: d_port(),
            http_port: d_http(),
            socks_port: d_socks(),
            domains: Vec::new(),
            fragment: Fragment::default(),
            dns_server: d_dns(),
            log_connections: true,
        }
    }
}

impl Config {
    pub fn from_file(fc: FileConfig) -> Result<Self, String> {
        let kind = fc
            .mode
            .or(fc.kind)
            .unwrap_or_else(|| "host".to_string())
            .trim()
            .to_ascii_lowercase();
        let kind = match kind.as_str() {
            "socks" | "socks5" | "clash" => "socks".to_string(),
            "host" | "hosts" | "local" => "host".to_string(),
            other => {
                return Err(format!("mode 只能是 socks 或 host，当前：{other}"));
            }
        };
        let mut cfg = Config {
            kind,
            listen_port: fc.listen.unwrap_or_else(d_port),
            http_port: fc.http.unwrap_or_else(d_http),
            socks_port: fc.socks.unwrap_or_else(d_socks),
            domains: fc.domains,
            fragment: fc.fragment,
            dns_server: fc.dns.unwrap_or_else(d_dns),
            log_connections: true,
        };
        cfg.sanitize();
        Ok(cfg)
    }

    pub fn sanitize(&mut self) {
        if self.fragment.parts < 2 {
            self.fragment.parts = 2;
        }
        if self.fragment.parts > 12 {
            self.fragment.parts = 12;
        }
        if self.socks_port == 0 {
            self.socks_port = d_socks();
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

    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.domains.is_empty() {
            errors.push("domains 不能为空".to_string());
        }
        if self.kind == "host" && self.listen_port == 0 {
            errors.push("listen 不能为 0".to_string());
        }
        errors
    }

    /// 生成简洁 YAML（带注释），`uuengine print-config` 用。
    pub fn to_yaml(&self) -> String {
        let mut s = String::new();
        s.push_str("# uuengine 配置\n");
        s.push_str(&format!("mode: {}\n", self.kind));
        s.push_str("domains:\n");
        for d in &self.domains {
            s.push_str(&format!("  - {d}\n"));
        }
        s.push_str(&format!("socks: {}\n", self.socks_port));
        s.push_str(&format!("listen: {}\n", self.listen_port));
        s.push_str(&format!("http: {}\n", self.http_port));
        s.push_str(&format!("fragment: {}\n", self.fragment.as_yaml()));
        s.push_str(&format!("dns: {}\n", self.dns_server));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_config() {
        let fc: FileConfig = serde_yaml::from_str("mode: socks\ndomains: [a.example.com]\n").unwrap();
        let c = Config::from_file(fc).unwrap();
        assert_eq!(c.kind, "socks");
        assert_eq!(c.socks_port, 7899);
        assert_eq!(c.fragment.parts, 4);
        assert_eq!(c.domains, vec!["a.example.com"]);
    }

    #[test]
    fn single_domain_string() {
        let fc: FileConfig = serde_yaml::from_str("mode: host\ndomains: nrd.nie.163.com\n").unwrap();
        let c = Config::from_file(fc).unwrap();
        assert_eq!(c.domains, vec!["nrd.nie.163.com"]);
    }

    #[test]
    fn fragment_forms() {
        for (raw, mode, parts) in [
            ("4", "record", 4),
            ("off", "off", 4),
            ("tcp", "segment", 4),
            ("tcp:6", "segment", 6),
            ("0", "off", 4),
        ] {
            let f = Fragment::parse(raw).unwrap();
            assert_eq!((f.mode.as_str(), f.parts), (mode, parts), "raw={raw}");
        }
        assert!(Fragment::parse("abc").is_err());
    }

    #[test]
    fn aliases_still_work() {
        let fc: FileConfig =
            serde_yaml::from_str("kind: socks\ndns_server: 1.1.1.1\nlisten_port: 8443\n").unwrap();
        let c = Config::from_file(fc).unwrap();
        assert_eq!(c.kind, "socks");
        assert_eq!(c.dns_server, "1.1.1.1");
        assert_eq!(c.listen_port, 8443);
    }

    #[test]
    fn yaml_roundtrip_is_readable() {
        let mut c = Config {
            kind: "socks".to_string(),
            domains: vec!["a.example.com".to_string()],
            fragment: Fragment { mode: "segment".to_string(), parts: 6 },
            ..Default::default()
        };
        c.sanitize();
        let text = c.to_yaml();
        let back: FileConfig = serde_yaml::from_str(&text).unwrap();
        let c2 = Config::from_file(back).unwrap();
        assert_eq!(c2.kind, "socks");
        assert_eq!(c2.fragment.mode, "segment");
        assert_eq!(c2.fragment.parts, 6);
        assert_eq!(c2.domains, c.domains);
    }

    #[test]
    fn bad_mode_is_rejected() {
        let fc: FileConfig = serde_yaml::from_str("mode: nope\ndomains: [a]\n").unwrap();
        assert!(Config::from_file(fc).is_err());
    }
}
