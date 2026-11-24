//! 内置预置数据。
//!
//! 只放"确定会用到、且不会变来变去"的端点清单，不联网、不读配置文件。

/// UU 远程（GameViewer）的控制面 / 信令 / 中继端点。
///
/// 这六个都在 `nrd.nie.163.com` 后缀下；这里逐条列出，是为了让用户一眼看到
/// 具体在放行哪些主机，而不是只写一个后缀。
pub const UU_ENDPOINTS: &[&str] = &[
    "api.nrd.nie.163.com",
    "api-dcdn.nrd.nie.163.com",
    "api-event.nrd.nie.163.com",
    "api-ipv4.nrd.nie.163.com",
    "sig-3303-d.nrd.nie.163.com",
    "relay-mg-3303-d.nrd.nie.163.com",
];

/// 上述端点的公共后缀（用户只想写一行时可用）。
pub const UU_SUFFIX: &str = "nrd.nie.163.com";

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
}
