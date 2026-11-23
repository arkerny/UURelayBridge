//! macOS 专属：用 Finder 打开目录。不安装任何系统扩展 / 网络扩展。

use std::path::Path;
use std::process::Command;

pub fn open(path: &Path) -> std::io::Result<()> {
    Command::new("open").arg(path).spawn().map(|_| ())
}
