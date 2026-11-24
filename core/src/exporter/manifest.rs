//! manifest.json：记录本次生成结果，便于用户核对"文件到底是不是新的"。

use serde::{Deserialize, Serialize};
use std::path::Path;

use super::writer;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub app: String,
    pub app_version: String,
    pub model_schema: u32,
    pub target: String,
    pub target_version: String,
    pub generated_at: String,
    pub file_name: String,
    pub sha256: String,
    pub size: u64,
    pub enabled: bool,
    pub rule_count: usize,
    pub project_name: String,
}

impl Manifest {
    pub fn to_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
}

pub fn write(path: &Path, manifest: &Manifest) -> std::io::Result<()> {
    writer::atomic_write(path, &manifest.to_pretty())
}

pub fn read(path: &Path) -> Option<Manifest> {
    let text = writer::read_optional(path).ok().flatten()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest {
            app: "RelayBridge".into(),
            app_version: "1.1.0".into(),
            model_schema: 1,
            target: "clash-verge-rev".into(),
            target_version: "2.x".into(),
            generated_at: "2026-01-01T00:00:00+08:00".into(),
            file_name: "relaybridge.js".into(),
            sha256: "abc".into(),
            size: 10,
            enabled: true,
            rule_count: 3,
            project_name: "Demo".into(),
        }
    }

    #[test]
    fn roundtrip() {
        let d = crate::test_tmp_dir("manifest");
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("manifest.json");
        write(&p, &sample()).unwrap();
        assert_eq!(read(&p).unwrap(), sample());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
