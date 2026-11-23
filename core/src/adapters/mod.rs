//! 适配器接口。新增内核只加实现，不改核心模型。

pub mod cvr;
pub mod mihomo;
pub mod singbox;

use crate::compiler::Compiled;

/// 渲染所需的全部输入（全部由调用方算好，适配器不做 IO）。
pub struct RenderInput<'a> {
    pub app_version: &'a str,
    pub project_name: &'a str,
    pub enabled: bool,
    pub compiled: &'a Compiled,
    /// RFC3339 时间戳，仅用于注释，不参与规则内容。
    pub generated_at: &'a str,
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
