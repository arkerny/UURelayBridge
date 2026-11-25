//! 域名匹配：精确、后缀、通配符（`*` / `+`）。
//!
//! 规则写法：
//!   `nrd.nie.163.com`   精确域名（含所有子域）
//!   `*.nrd.nie.163.com` 任意子域（不含根域）
//!   `+.nrd.nie.163.com` 根域 + 任意子域（Clash 的 `+.` 语义）
//!   `*`                 除 IP 字面量外的全部域名
//!
//! 只做一层 `*` 展开，不做回溯正则，保证每次匹配都是 O(域名长度)。

/// 一条编译好的规则。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    /// 匹配所有非 IP 目标
    Any,
    /// 精确域名，含其子域（`a.com` → `a.com` / `x.a.com`）
    Suffix(String),
    /// 仅子域（`*.a.com` → `x.a.com`，不含 `a.com`）
    SubOnly(String),
    /// 根域 + 子域（`+.a.com`）
    RootAndSub(String),
    /// 中间通配（`a.*.c.com`），按前后缀判断
    Wildcard {
        prefix: String,
        suffix: String,
        min_len: usize,
    },
    /// 目标本身是 IP 字面量（精确匹配该 IP）
    IpLiteral(String),
}

impl Pattern {
    pub fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim().trim_end_matches('.').to_ascii_lowercase();
        if s.is_empty() {
            return None;
        }
        if s == "*" {
            return Some(Pattern::Any);
        }
        if let Some(rest) = s.strip_prefix("*.") {
            return Some(Pattern::SubOnly(rest.to_string()));
        }
        if let Some(rest) = s.strip_prefix("+.") {
            return Some(Pattern::RootAndSub(rest.to_string()));
        }
        if s.contains('*') {
            // 中间通配：a.*.c.com
            let mut parts = s.splitn(2, '*');
            let prefix = parts.next().unwrap_or("").to_string();
            let suffix = parts.next().unwrap_or("").to_string();
            let prefix = prefix.strip_suffix('.').map(|p| format!("{p}.")).unwrap_or(prefix);
            let suffix = suffix.strip_prefix('.').map(|p| format!(".{p}")).unwrap_or(suffix);
            return Some(Pattern::Wildcard {
                min_len: prefix.len() + suffix.len() + 1,
                prefix,
                suffix,
            });
        }
        if s.parse::<std::net::IpAddr>().is_ok() {
            return Some(Pattern::IpLiteral(s));
        }
        Some(Pattern::Suffix(s))
    }

    pub fn matches(&self, host: &str) -> bool {
        let h = host.trim_end_matches('.').to_ascii_lowercase();
        let is_ip = h.parse::<std::net::IpAddr>().is_ok();
        match self {
            // 只匹配完全相同的 IP
            Pattern::IpLiteral(ip) => h == *ip,
            Pattern::Any => !is_ip,
            Pattern::Suffix(base) => {
                if is_ip {
                    return false;
                }
                h == *base || h.ends_with(&format!(".{base}"))
            }
            Pattern::SubOnly(base) => !is_ip && h.ends_with(&format!(".{base}")),
            Pattern::RootAndSub(base) => {
                !is_ip && (h == *base || h.ends_with(&format!(".{base}")))
            }
            Pattern::Wildcard {
                prefix,
                suffix,
                min_len,
            } => {
                if is_ip || h.len() < *min_len {
                    return false;
                }
                h.starts_with(prefix.as_str()) && h.ends_with(suffix.as_str())
            }
        }
    }

    #[allow(dead_code)]
    pub fn as_str(&self) -> String {
        match self {
            Pattern::Any => "*".to_string(),
            Pattern::Suffix(b) => b.clone(),
            Pattern::SubOnly(b) => format!("*.{b}"),
            Pattern::RootAndSub(b) => format!("+.{b}"),
            Pattern::IpLiteral(ip) => ip.clone(),
            Pattern::Wildcard { prefix, suffix, .. } => {
                format!("{}*{}", prefix.trim_end_matches('.'), suffix)
            }
        }
    }
}

/// 一组规则。
#[derive(Debug, Clone, Default)]
pub struct Matcher {
    pub patterns: Vec<Pattern>,
}

impl Matcher {
    pub fn build(domains: &[String]) -> Self {
        let mut patterns = Vec::new();
        for d in domains {
            if let Some(p) = Pattern::parse(d) {
                if !patterns.contains(&p) {
                    patterns.push(p);
                }
            }
        }
        Self { patterns }
    }

    /// 命中任意一条即为"需要分片"。
    pub fn matches(&self, host: &str) -> bool {
        self.patterns.iter().any(|p| p.matches(host))
    }

    /// 是否包含全局通配（用于日志提示）。
    pub fn has_any(&self) -> bool {
        self.patterns.iter().any(|p| matches!(p, Pattern::Any))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(domains: &[&str]) -> Matcher {
        Matcher::build(&domains.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn suffix_matches_self_and_children() {
        let x = m(&["nrd.nie.163.com"]);
        assert!(x.matches("nrd.nie.163.com"));
        assert!(x.matches("api.nrd.nie.163.com"));
        assert!(x.matches("API.NRD.NIE.163.COM"));
        assert!(!x.matches("nie.163.com"));
        assert!(!x.matches("evil-nrd.nie.163.com"));
    }

    #[test]
    fn star_matches_children_only() {
        let x = m(&["*.nrd.nie.163.com"]);
        assert!(!x.matches("nrd.nie.163.com"));
        assert!(x.matches("api.nrd.nie.163.com"));
        assert!(x.matches("a.b.nrd.nie.163.com"));
    }

    #[test]
    fn plus_matches_root_and_children() {
        let x = m(&["+.nrd.nie.163.com"]);
        assert!(x.matches("nrd.nie.163.com"));
        assert!(x.matches("api.nrd.nie.163.com"));
    }

    #[test]
    fn bare_star_matches_all_hostnames_not_ips() {
        let x = m(&["*"]);
        assert!(x.matches("anything.example"));
        assert!(!x.matches("1.2.3.4"), "IP 字面量不在全局通配里");
        assert!(x.has_any());
    }

    #[test]
    fn middle_wildcard() {
        let x = m(&["api.*.nie.163.com"]);
        assert!(x.matches("api.us.nie.163.com"));
        assert!(!x.matches("api.nie.163.com"), "至少要有一段");
        assert!(!x.matches("other.us.nie.163.com"));
    }

    #[test]
    fn ip_literal_pattern() {
        let x = m(&["1.2.3.4"]);
        assert!(x.matches("1.2.3.4"));
        assert!(!x.matches("1.2.3.5"));
    }

    #[test]
    fn dedupes_patterns() {
        let x = m(&["a.com", "a.com", "*.a.com", "*.a.com"]);
        assert_eq!(x.patterns.len(), 2);
    }
}
