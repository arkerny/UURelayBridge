//! Clash Verge Rev 扩展脚本适配器。
//!
//! 目标接口：Clash Verge Rev 的「扩展脚本」，脚本里导出 `main(config)`，
//! 接收合并后的配置对象并返回修改后的对象。我们只动 `config.rules`：
//!   ON  —— 把 RelayBridge 的规则插到最前面（在 MATCH 之类终结规则之前）。
//!   OFF —— 只移除上一次注入的完全相同规则，其余原样保留。
//! 幂等：按"完全相同的规则串"去重，重复加载不会叠加。

pub mod script;

use super::{Adapter, RenderError, RenderInput};

#[derive(Debug, Default)]
pub struct ClashVergeRev;

impl Adapter for ClashVergeRev {
    fn id(&self) -> &'static str {
        "clash-verge-rev"
    }

    fn display_name(&self) -> &'static str {
        "Clash Verge Rev"
    }

    fn target_version(&self) -> &'static str {
        "Clash Verge Rev 2.x（扩展脚本 main(config)）"
    }

    fn file_name(&self) -> &'static str {
        "relaybridge.js"
    }

    fn render(&self, input: &RenderInput<'_>) -> Result<String, RenderError> {
        Ok(script::render(input))
    }
}
