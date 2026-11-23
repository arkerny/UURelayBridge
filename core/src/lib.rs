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
pub mod rules;
pub mod settings;
pub mod validator;
