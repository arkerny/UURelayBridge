//! Windows 专属：用资源管理器打开目录。不做任何注册表 / 服务 / 系统代理操作。

use std::path::Path;
use std::process::Command;

pub fn open(path: &Path) -> std::io::Result<()> {
    // explorer.exe 对目录路径直接打开；不隐藏窗口，便于用户看到定位结果
    Command::new("explorer.exe").arg(path).spawn().map(|_| ())
}
