//! 规则条目的语法校验与规范化（纯函数，便于单测）。

/// 域名匹配类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainKind {
    /// 精确域名：`a.example.com`
    Exact,
    /// 含自身及所有子域：`example.com`
    Suffix,
    /// 仅子域：`*.example.com`
    SubOnly,
    /// 根域 + 子域：`+.example.com`
    RootAndSub,
}

/// 规范化域名：去空白、转小写、去末尾点、去掉 `*.` 前缀。
/// 返回 (规范化后的域名, 是否为通配写法)。
pub fn normalize_domain(raw: &str) -> (String, bool) {
    let mut s = raw.trim().to_ascii_lowercase();
    if let Some(stripped) = s.strip_suffix('.') {
        s = stripped.to_string();
    }
    // 内核 matcher 支持 *. 与 +. 两种前缀
    let wildcard = s.starts_with("*.") || s.starts_with("+.");
    if let Some(rest) = s.strip_prefix("*.") {
        s = rest.to_string();
    } else if let Some(rest) = s.strip_prefix("+.") {
        s = rest.to_string();
    }
    (s, wildcard)
}

/// 判断域名写法是否合法。返回规范化结果或人类可读的错误。
pub fn validate_domain(raw: &str) -> Result<(String, DomainKind), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("域名不能为空".into());
    }
    if trimmed.contains("://") {
        return Err("只填域名，不要带 http:// 或 https://".into());
    }
    if trimmed.contains('/') {
        return Err("域名中不能包含 /（IP 网段请填到 IP/CIDR 一栏）".into());
    }
    if trimmed.chars().any(|c| c.is_whitespace()) {
        return Err("域名中不能包含空格".into());
    }
    if !trimmed.is_ascii() {
        return Err("暂不支持非 ASCII 域名，请改用 punycode 形式（xn-- 开头）".into());
    }

    let (host, wildcard) = normalize_domain(trimmed);
    if host.is_empty() {
        return Err("域名不能为空".into());
    }
    if host.contains('*') || host.contains('+') {
        return Err("通配符只支持开头的 `*.` / `+.` 写法".into());
    }
    if host.len() > 253 {
        return Err("域名长度超过 253 个字符".into());
    }
    if !host.contains('.') {
        return Err("域名至少需要包含一个点，例如 example.com".into());
    }
    for label in host.split('.') {
        if label.is_empty() {
            return Err("域名中存在空标签（连续的点）".into());
        }
        if label.len() > 63 {
            return Err("域名中存在超过 63 字符的标签".into());
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(format!("标签 `{}` 不能以 - 开头或结尾", label));
        }
        if !label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!("标签 `{}` 含有非法字符", label));
        }
    }

    // 裸域名（无 *. 前缀）按后缀匹配，覆盖自身与所有子域；
    // 带 *. 前缀的写法语义相同，这里统一成后缀。
    let lowered = raw.trim().to_ascii_lowercase();
    let kind = if lowered.starts_with("+.") {
        DomainKind::RootAndSub
    } else if lowered.starts_with("*.") {
        DomainKind::SubOnly
    } else if host.matches('.').count() >= 1 {
        DomainKind::Suffix
    } else {
        DomainKind::Exact
    };
    Ok((host, kind))
}

/// 规范化并校验 IP 或 CIDR，返回 `ip/prefix` 形式。
pub fn normalize_cidr(raw: &str) -> Result<String, String> {
    use std::net::IpAddr;
    let s = raw.trim();
    if s.is_empty() {
        return Err("IP/CIDR 不能为空".into());
    }
    let (ip_part, prefix_part) = match s.split_once('/') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    };
    let ip: IpAddr = ip_part
        .parse()
        .map_err(|_| format!("`{}` 不是合法的 IP 地址", ip_part))?;
    let max = if ip.is_ipv4() { 32 } else { 128 };
    let prefix = match prefix_part {
        None => max,
        Some(p) => {
            let v: u32 = p
                .trim()
                .parse()
                .map_err(|_| format!("`{}` 不是合法的前缀长度", p))?;
            if v > max {
                return Err(format!("前缀长度不能大于 {}（这是 {}）", max,
                    if ip.is_ipv4() { "IPv4" } else { "IPv6" }));
            }
            v
        }
    };
    Ok(format!("{}/{}", ip, prefix))
}

/// 把一条规则条目编译成 Clash 规则串的列表（不含策略组校验）。
pub fn compile_entry(
    domain: &str,
    ip_cidr: &str,
    target: &str,
) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for d in domain
        .split(['\n', ',', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let (host, kind) = validate_domain(d)?;
        let rule = match kind {
            DomainKind::Suffix => format!("DOMAIN-SUFFIX,{},{}", host, target),
            DomainKind::Exact => format!("DOMAIN,{},{}", host, target),
            // 仅在子域写法在 Clash 中没有独立语义，退化为后缀匹配
            DomainKind::SubOnly => format!("DOMAIN-SUFFIX,{},{}", host, target),
            DomainKind::RootAndSub => format!("DOMAIN-SUFFIX,{},{}", host, target),
        };
        out.push(rule);
    }
    for c in ip_cidr
        .split(['\n', ',', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let cidr = normalize_cidr(c)?;
        out.push(format!("IP-CIDR,{},{}", cidr, target));
    }
    if out.is_empty() {
        return Err("规则至少需要一个域名或一个 IP/CIDR".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_suffix_from_bare_name() {
        let (host, kind) = validate_domain("Example.COM").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(kind, DomainKind::Suffix);
    }

    #[test]
    fn star_wildcard_is_sub_only() {
        let (host, kind) = validate_domain("*.nrd.nie.163.com").unwrap();
        assert_eq!(host, "nrd.nie.163.com");
        assert_eq!(kind, DomainKind::SubOnly);
    }

    #[test]
    fn rejects_scheme_and_slash() {
        assert!(validate_domain("https://a.com").is_err());
        assert!(validate_domain("a.com/path").is_err());
        assert!(validate_domain("a b.com").is_err());
    }

    #[test]
    fn plus_wildcard_is_root_and_sub() {
        let (host, kind) = validate_domain("+.nrd.nie.163.com").unwrap();
        assert_eq!(host, "nrd.nie.163.com");
        assert_eq!(kind, DomainKind::RootAndSub);
    }

    #[test]
    fn plus_and_star_both_accepted() {
        // 预置默认值用的就是 +. 写法，必须能通过校验
        assert!(validate_domain("+.nrd.nie.163.com").is_ok());
        assert!(validate_domain("*.nrd.nie.163.com").is_ok());
        assert!(validate_domain("nrd.nie.163.com").is_ok());
    }

    #[test]
    fn rejects_bad_labels() {
        assert!(validate_domain("a..com").is_err());
        assert!(validate_domain("-a.com").is_err());
        assert!(validate_domain("a!b.com").is_err());
        assert!(validate_domain("nodot").is_err());
    }

    #[test]
    fn cidr_forms() {
        assert_eq!(normalize_cidr("1.2.3.4").unwrap(), "1.2.3.4/32");
        assert_eq!(normalize_cidr("10.0.0.0/8").unwrap(), "10.0.0.0/8");
        assert_eq!(normalize_cidr("2001:db8::1").unwrap(), "2001:db8::1/128");
        assert!(normalize_cidr("10.0.0.0/33").is_err());
        assert!(normalize_cidr("999.1.1.1").is_err());
    }

    #[test]
    fn compile_entry_builds_rules() {
        let rules = compile_entry("a.com\n*.b.com", "10.0.0.0/8", "PROXY").unwrap();
        assert_eq!(
            rules,
            vec![
                "DOMAIN-SUFFIX,a.com,PROXY",
                "DOMAIN-SUFFIX,b.com,PROXY",
                "IP-CIDR,10.0.0.0/8,PROXY",
            ]
        );
    }
}
