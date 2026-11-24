//! settings.json / project.json 的读写、默认值与损坏恢复。
//!
//! 关键原则：启动阶段**只读不写**。文件缺失或损坏都不落盘，
//! 只有用户在界面里点了保存，才会写文件（此时才做"损坏文件备份"）。

use crate::model::{Project, MODEL_SCHEMA_VERSION};
use crate::exporter::writer;
use serde::{Deserialize, Serialize};
use std::path::Path;

fn d_true() -> bool {
    true
}
fn d_target() -> String {
    "clash-verge-rev".to_string()
}
fn d_backups() -> usize {
    20
}
fn d_theme() -> String {
    "system".to_string()
}

/// 允许的主题值：system（跟随系统）/ light / dark
pub const THEME_MODES: [&str; 3] = ["system", "light", "dark"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "schema")]
    pub schema_version: u32,
    /// RelayBridge 自己的开关：是否把规则注入生成的脚本。
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub start_minimized: bool,
    #[serde(default = "d_true")]
    pub close_to_tray: bool,
    #[serde(default = "d_true")]
    pub show_notifications: bool,
    #[serde(default = "d_target")]
    pub target: String,
    #[serde(default = "d_backups")]
    pub backup_limit: usize,
    /// 界面主题：system / light / dark
    #[serde(default = "d_theme")]
    pub theme: String,
    #[serde(default)]
    pub last_generated_at: Option<String>,
    #[serde(default)]
    pub last_generation_ok: Option<bool>,
}

fn schema() -> u32 {
    MODEL_SCHEMA_VERSION
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: MODEL_SCHEMA_VERSION,
            // 默认关闭：用户没点 ON 之前不注入任何规则
            enabled: false,
            start_minimized: false,
            close_to_tray: true,
            show_notifications: true,
            target: d_target(),
            backup_limit: d_backups(),
            theme: d_theme(),
            last_generated_at: None,
            last_generation_ok: None,
        }
    }
}

impl Settings {
    /// 夹紧非法值，避免用户手改坏 JSON 后行为异常。
    pub fn sanitize(&mut self) {
        if !crate::adapters::SUPPORTED_TARGETS.contains(&self.target.as_str()) {
            self.target = "clash-verge-rev".to_string();
        }
        if self.backup_limit == 0 {
            self.backup_limit = 1;
        }
        if self.backup_limit > 200 {
            self.backup_limit = 200;
        }
        if !THEME_MODES.contains(&self.theme.as_str()) {
            self.theme = d_theme();
        }
        self.schema_version = MODEL_SCHEMA_VERSION;
    }
}

/// 读取结果：值 + 是否曾经存在 + 读取时的问题（损坏/版本不符）。
#[derive(Debug, Clone)]
pub struct Loaded<T> {
    pub value: T,
    pub existed: bool,
    pub issue: Option<String>,
}

pub fn load_settings(path: &Path) -> Loaded<Settings> {
    match writer::read_optional(path) {
        Ok(None) => Loaded {
            value: Settings::default(),
            existed: false,
            issue: None,
        },
        Ok(Some(text)) => match serde_json::from_str::<Settings>(&text) {
            Ok(mut s) => {
                let mut issue = None;
                if s.schema_version > MODEL_SCHEMA_VERSION {
                    issue = Some(format!(
                        "settings.json 版本 {} 高于程序支持的 {}，已按默认值处理",
                        s.schema_version, MODEL_SCHEMA_VERSION
                    ));
                    s = Settings::default();
                }
                s.sanitize();
                Loaded {
                    value: s,
                    existed: true,
                    issue,
                }
            }
            Err(e) => Loaded {
                value: Settings::default(),
                existed: true,
                issue: Some(format!("settings.json 解析失败（{}），本次使用默认设置", e)),
            },
        },
        Err(e) => Loaded {
            value: Settings::default(),
            existed: true,
            issue: Some(format!("settings.json 读取失败：{}", e)),
        },
    }
}

pub fn load_project(path: &Path) -> Loaded<Project> {
    match writer::read_optional(path) {
        Ok(None) => Loaded {
            value: Project::default(),
            existed: false,
            issue: None,
        },
        Ok(Some(text)) => match serde_json::from_str::<Project>(&text) {
            Ok(mut p) => {
                let mut issue = None;
                if p.schema_version > MODEL_SCHEMA_VERSION {
                    issue = Some(format!(
                        "project.json 版本 {} 高于程序支持的 {}",
                        p.schema_version, MODEL_SCHEMA_VERSION
                    ));
                    p.schema_version = MODEL_SCHEMA_VERSION;
                }
                Loaded {
                    value: p,
                    existed: true,
                    issue,
                }
            }
            Err(e) => Loaded {
                value: Project::default(),
                existed: true,
                issue: Some(format!("project.json 解析失败（{}），本次使用空规则", e)),
            },
        },
        Err(e) => Loaded {
            value: Project::default(),
            existed: true,
            issue: Some(format!("project.json 读取失败：{}", e)),
        },
    }
}

/// 写入前，如果原文件已损坏，先把它挪进 backups/，不静默丢弃。
pub fn preserve_broken(path: &Path, backups_dir: &Path) -> std::io::Result<Option<std::path::PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    crate::exporter::backup::backup_file(path, backups_dir)
}

pub fn save_settings(path: &Path, s: &Settings) -> std::io::Result<()> {
    writer::atomic_write(path, &serde_json::to_string_pretty(s).unwrap_or_default())
}

pub fn save_project(path: &Path, p: &Project) -> std::io::Result<()> {
    writer::atomic_write(path, &serde_json::to_string_pretty(p).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmpdir(tag: &str) -> PathBuf {
        crate::test_tmp_dir(tag)
    }

    #[test]
    fn missing_files_give_defaults_without_writing() {
        let d = tmpdir("missing");
        let f = d.join("settings.json");
        let l = load_settings(&f);
        assert!(!l.existed);
        assert!(l.issue.is_none());
        assert!(!l.value.enabled);
        assert!(!f.exists(), "启动阶段不允许写文件");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn corrupt_file_is_reported_and_replaced_by_defaults() {
        let d = tmpdir("corrupt");
        let f = d.join("settings.json");
        std::fs::write(&f, "{not json").unwrap();
        let l = load_settings(&f);
        assert!(l.existed);
        assert!(l.issue.is_some());
        assert_eq!(l.value, Settings::default());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let d = tmpdir("roundtrip");
        let f = d.join("settings.json");
        let mut s = Settings::default();
        s.enabled = true;
        s.close_to_tray = false;
        save_settings(&f, &s).unwrap();
        let l = load_settings(&f);
        assert!(l.value.enabled);
        assert!(!l.value.close_to_tray);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn sanitize_clamps_bad_values() {
        let mut s = Settings::default();
        s.target = "not-a-kernel".into();
        s.backup_limit = 0;
        s.sanitize();
        assert_eq!(s.target, "clash-verge-rev");
        assert_eq!(s.backup_limit, 1);
    }

    #[test]
    fn theme_defaults_to_system_and_rejects_unknown() {
        let mut s = Settings::default();
        assert_eq!(s.theme, "system");
        s.theme = "neon".into();
        s.sanitize();
        assert_eq!(s.theme, "system");
        s.theme = "light".into();
        s.sanitize();
        assert_eq!(s.theme, "light");
    }

    #[test]
    fn old_settings_without_theme_still_load() {
        let d = tmpdir("legacy-theme");
        let f = d.join("settings.json");
        // 1.0 时代的文件：没有 theme 字段
        std::fs::write(
            &f,
            r#"{"schema_version":1,"enabled":true,"start_minimized":false,"close_to_tray":true,"show_notifications":true,"target":"clash-verge-rev"}"#,
        )
        .unwrap();
        let l = load_settings(&f);
        assert!(l.issue.is_none());
        assert_eq!(l.value.theme, "system");
        assert!(l.value.enabled);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn project_defaults_are_empty() {
        let d = tmpdir("proj");
        let l = load_project(&d.join("project.json"));
        assert!(l.value.rules.is_empty());
        assert!(!l.existed);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn broken_file_is_preserved_on_save() {
        let d = tmpdir("preserve");
        let f = d.join("project.json");
        std::fs::write(&f, "garbage").unwrap();
        let b = d.join("backups");
        let saved = preserve_broken(&f, &b).unwrap();
        assert!(saved.is_some());
        assert!(saved.unwrap().exists());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
