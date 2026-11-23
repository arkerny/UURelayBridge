//! 占位适配器：只声明接口与目标格式，不实现任何生成逻辑。

use super::{Adapter, RenderError, RenderInput};

/// sing-box 内核（JSON）。首期不实现。
#[derive(Debug, Default)]
pub struct SingBox;

impl Adapter for SingBox {
    fn id(&self) -> &'static str {
        "sing-box"
    }

    fn display_name(&self) -> &'static str {
        "sing-box（规划中）"
    }

    fn target_version(&self) -> &'static str {
        "sing-box 1.11+（JSON 片段）"
    }

    fn file_name(&self) -> &'static str {
        "relaybridge.singbox.json"
    }

    fn render(&self, _input: &RenderInput<'_>) -> Result<String, RenderError> {
        Err(RenderError::NotImplemented(
            "sing-box 适配器将在后续版本提供，当前请使用 Clash Verge Rev",
        ))
    }
}
