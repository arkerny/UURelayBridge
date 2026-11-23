//! 跨平台数据目录。唯一的路径来源，业务层不拼平台路径。

use crate::exporter::ExportPaths;
use std::io;
use std::path::{Path, PathBuf};

/// RelayBridge 自己的数据目录结构：
///   <root>/settings.json
///   <root>/project.json
///   <root>/output/<target>/<file>
///   <root>/backups/
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
}

impl AppPaths {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn settings(&self) -> PathBuf {
        self.root.join("settings.json")
    }
    pub fn project(&self) -> PathBuf {
        self.root.join("project.json")
    }
    pub fn output_root(&self) -> PathBuf {
        self.root.join("output")
    }
    pub fn output_dir(&self, target: &str) -> PathBuf {
        // target 只来自内部白名单，这里再做一次文件名字符清洗，防路径穿越
        let safe: String = target
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect();
        self.output_root().join(if safe.is_empty() { "unknown".to_string() } else { safe })
    }
    pub fn script(&self, target: &str, file_name: &str) -> PathBuf {
        self.output_dir(target).join(file_name)
    }
    pub fn manifest(&self, target: &str) -> PathBuf {
        self.output_dir(target).join("manifest.json")
    }
    pub fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }

    pub fn export_paths(&self, target: &str, file_name: &str) -> ExportPaths {
        ExportPaths {
            output_dir: self.output_dir(target),
            script: self.script(target, file_name),
            manifest: self.manifest(target),
            backups_dir: self.backups_dir(),
        }
    }

    /// 只建自己需要的目录；不触碰任何外部路径。
    pub fn ensure_dirs(&self, target: &str) -> io::Result<()> {
        std::fs::create_dir_all(self.output_dir(target))?;
        std::fs::create_dir_all(self.backups_dir())?;
        Ok(())
    }

    pub fn display(&self) -> String {
        self.root.display().to_string()
    }
}

/// 防路径穿越的自检：所有输出路径必须落在 root 内。
pub fn is_inside(root: &Path, candidate: &Path) -> bool {
    let Ok(root) = root.canonicalize() else { return false };
    let Ok(cand) = candidate.canonicalize() else { return false };
    cand.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_stable() {
        let p = AppPaths::new(PathBuf::from("/tmp/rb"));
        assert_eq!(p.settings(), PathBuf::from("/tmp/rb/settings.json"));
        assert_eq!(
            p.script("clash-verge-rev", "relaybridge.js"),
            PathBuf::from("/tmp/rb/output/clash-verge-rev/relaybridge.js")
        );
    }

    #[test]
    fn target_cannot_escape_root() {
        let p = AppPaths::new(PathBuf::from("/tmp/rb"));
        let d = p.output_dir("../../../etc");
        assert!(!d.to_string_lossy().contains(".."));
        assert!(d.starts_with(p.output_root()));
    }
}
