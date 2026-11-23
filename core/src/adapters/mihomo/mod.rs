//! 占位适配器：只声明接口与目标格式，不实现任何生成逻辑。

use super::{Adapter, RenderError, RenderInput};

/// 独立 Mihomo 内核（YAML）。首期不实现。
#[derive(Debug, Default)]
pub struct Mihomo;

impl Adapter for Mihomo {
    fn id(&self) -> &'static str {
        "mihomo"
    }

    fn display_name(&self) -> &'static str {
        "Mihomo（规划中）"
    }

    fn target_version(&self) -> &'static str {
        "Mihomo 1.18+（YAML 片段）"
    }

    fn file_name(&self) -> &'static str {
        "relaybridge.mihomo.yaml"
    }

    fn render(&self, _input: &RenderInput<'_>) -> Result<String, RenderError> {
        Err(RenderError::NotImplemented(
            "Mihomo 适配器将在后续版本提供，当前请使用 Clash Verge Rev",
        ))
    }
}
