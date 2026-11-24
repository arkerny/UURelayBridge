//! RelayBridge 核心库。
//!
//! 这一层不依赖 Tauri、不认识窗口/托盘/文件管理器，只做：
//! 模型 → 校验 → 编译 → 渲染 → 原子写盘 / 备份 / manifest。
//!
//! Windows / macOS 共用同一份实现；平台差异全部留在 `src-tauri/src/platform`。

pub mod adapters;
pub mod compiler;
pub mod exporter;
pub mod model;
pub mod paths;
pub mod presets;
pub mod rules;
pub mod settings;
pub mod sni;
pub mod validator;

/// 测试用临时目录。
///
/// 默认放在 `target/test-tmp/` 下：受限环境（沙箱 / 只读临时目录）里
/// `%TEMP%` 往往不可写，导致和业务无关的测试失败。
/// 只有连仓库目录都写不了时才退回系统临时目录。
#[doc(hidden)]
pub fn test_tmp_dir(tag: &str) -> std::path::PathBuf {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-tmp");
    let dir = base.join(format!("{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    if std::fs::create_dir_all(&dir).is_ok() {
        return dir;
    }
    let fallback = std::env::temp_dir().join(format!("relaybridge-test-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&fallback);
    let _ = std::fs::create_dir_all(&fallback);
    fallback
}
