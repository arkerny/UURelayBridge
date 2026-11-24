//! 适配器接口。新增内核只加实现，不改核心模型。

pub mod cvr;
pub mod mihomo;
pub mod singbox;

use crate::compiler::Compiled;

/// SNI 处理信息：与 Host 模式共用同一份域名列表。
///
/// Clash / Mihomo 没有原生的 TLS 分片能力，所以这里只做"把域名交给本机转发器"：
/// 用 `dns.hosts` 把域名指向 127.0.0.1，再用 `DIRECT` 规则让内核连本机 443，
/// 真正的分片由 UURelayBridge 的 Host 模式完成。
#[derive(Debug, Clone, Default)]
pub struct SniLink {
    pub enabled: bool,
    pub domains: Vec<String>,
    pub mode: String,
    pub parts: u8,
}

/// 渲染所需的全部输入（全部由调用方算好，适配器不做 IO）。
pub struct RenderInput<'a> {
    pub app_version: &'a str,
    pub project_name: &'a str,
    pub enabled: bool,
    pub compiled: &'a Compiled,
    /// RFC3339 时间戳，仅用于注释，不参与规则内容。
    pub generated_at: &'a str,
    /// SNI 联动（None = 不生成 SNI 段）
    pub sni: Option<&'a SniLink>,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("该适配器尚未实现（{0}）")]
    NotImplemented(&'static str),
}

/// 一个目标内核的配置生成器。
pub trait Adapter {
    /// 稳定标识，写进 settings.json / manifest.json。
    fn id(&self) -> &'static str;
    /// 界面上显示的名字。
    fn display_name(&self) -> &'static str;
    /// 声明的目标软件版本范围，写进 manifest。
    fn target_version(&self) -> &'static str;
    /// 生成的文件名（不含目录）。
    fn file_name(&self) -> &'static str;
    /// 生成文件内容。
    fn render(&self, input: &RenderInput<'_>) -> Result<String, RenderError>;
}

/// 按 settings.target 取适配器。
pub fn by_id(id: &str) -> Option<Box<dyn Adapter>> {
    match id {
        "clash-verge-rev" => Some(Box::new(cvr::ClashVergeRev::default())),
        "mihomo" => Some(Box::new(mihomo::Mihomo)),
        "sing-box" => Some(Box::new(singbox::SingBox)),
        _ => None,
    }
}

pub const SUPPORTED_TARGETS: [&str; 3] = ["clash-verge-rev", "mihomo", "sing-box"];
