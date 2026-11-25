//! 内置预置数据。
//!
//! 只放"确定会用到、且不会变来变去"的端点清单，不联网、不读配置文件。

/// UU 远程（GameViewer）的控制面 / 信令 / 中继端点。
///
/// 这些主机都在 `nrd.nie.163.com` 下，但节点名会随调度变化
/// （例如 `sig-3303-d` → `sig-3207-g`），所以预置**不用固定清单**，
/// 直接用根域 + 子域的通配符 `+.nrd.nie.163.com`：
///
/// - `*.nrd.nie.163.com` 的子域全部命中（含未来新增的节点）；
/// - 根域 `nrd.nie.163.com` 本身没有 A 记录，命中也不会有连接失败问题。
///
/// 下面这份清单只用于**文档展示**（README / 界面提示"放行了哪些主机"），
/// 不参与实际匹配。
pub const UU_ENDPOINTS: &[&str] = &[
    "api.nrd.nie.163.com",
    "api-dcdn.nrd.nie.163.com",
    "api-event.nrd.nie.163.com",
    "api-ipv4.nrd.nie.163.com",
    "sig-3303-d.nrd.nie.163.com",
    "relay-mg-3303-d.nrd.nie.163.com",
];

/// UU 的根域。
pub const UU_SUFFIX: &str = "nrd.nie.163.com";

/// 预置规则：根域 + 任意子域的通配符。
pub const UU_PATTERN: &str = "+.nrd.nie.163.com";

/// 实际使用的预置域名列表（单条通配符，覆盖全部 UU 节点）。
pub fn uu_domains() -> Vec<String> {
    vec![UU_PATTERN.to_string()]
}

/// 展开后的端点清单（仅用于展示，不参与匹配）。
pub fn uu_endpoints() -> Vec<String> {
    UU_ENDPOINTS.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_share_the_suffix() {
        for e in UU_ENDPOINTS {
            assert!(e.ends_with(UU_SUFFIX), "{e} 不在 {UU_SUFFIX} 下");
        }
        assert_eq!(UU_ENDPOINTS.len(), 6);
    }

    #[test]
    fn endpoints_are_unique() {
        let mut v = uu_endpoints();
        v.sort();
        v.dedup();
        assert_eq!(v.len(), UU_ENDPOINTS.len());
    }

    #[test]
    fn preset_is_a_single_wildcard() {
        let d = uu_domains();
        assert_eq!(d.len(), 1, "预置应该只有一条通配符");
        assert_eq!(d[0], "+.nrd.nie.163.com");
    }

    #[test]
    fn preset_covers_every_known_endpoint() {
        // 用引擎同一套匹配逻辑验证：预置通配符必须覆盖全部已知端点
        let pat = UU_PATTERN.trim_start_matches("+.");
        for e in UU_ENDPOINTS {
            assert!(
                e.ends_with(&format!(".{pat}")) || *e == pat,
                "预置 {UU_PATTERN} 没覆盖 {e}"
            );
        }
    }

    #[test]
    fn preset_does_not_leak_to_sibling_domains() {
        let pat = UU_PATTERN.trim_start_matches("+.");
        assert!(!format!("evil{pat}").ends_with(&format!(".{pat}")));
        assert!(!"notnrd.nie.163.com".ends_with(&format!(".{pat}")));
    }
}
