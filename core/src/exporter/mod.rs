//! 导出流程：校验 → 编译 → 渲染 → 备份 → 原子替换 → manifest → 清理旧备份。
//!
//! 失败时保证：上一次成功生成的文件仍然完好（原子替换 + 先写临时文件）。

pub mod backup;
pub mod manifest;
pub mod writer;

use std::path::PathBuf;

use crate::adapters::{self, Adapter, RenderInput};
use crate::compiler::{self, Compiled};
use crate::model::Project;
use crate::validator::{self, has_errors, Issue};

pub use manifest::Manifest;

#[derive(Debug, Clone)]
pub struct ExportPaths {
    pub output_dir: PathBuf,
    pub script: PathBuf,
    pub manifest: PathBuf,
    pub backups_dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("校验未通过（{} 个错误），未写入任何文件", .0.len())]
    Validation(Vec<Issue>),
    #[error("目标内核不支持：{0}")]
    UnsupportedTarget(String),
    #[error("生成脚本失败：{0}")]
    Render(#[from] adapters::RenderError),
    #[error("创建目录失败：{path}：{source}")]
    CreateDir { path: String, source: std::io::Error },
    #[error("写入文件失败：{path}：{source}")]
    Write { path: String, source: std::io::Error },
    #[error("创建备份失败：{0}")]
    Backup(String),
    #[error("路径不可用：{0}")]
    BadPath(String),
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub script_path: PathBuf,
    pub manifest_path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
    pub rule_count: usize,
    pub generated_at: String,
    pub backup: Option<PathBuf>,
    pub enabled: bool,
    /// 按顺序记录的执行步骤，交给上层写进实时日志。
    pub steps: Vec<String>,
    pub warnings: Vec<Issue>,
}

fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// 只做预览，不写任何文件。
pub fn preview(
    project: &Project,
    app_version: &str,
    target_id: &str,
) -> Result<(String, Compiled, Vec<Issue>), ExportError> {
    let adapter = adapters::by_id(target_id)
        .ok_or_else(|| ExportError::UnsupportedTarget(target_id.to_string()))?;
    let issues = validator::validate(project);
    if has_errors(&issues) {
        return Err(ExportError::Validation(issues));
    }
    let compiled = compiler::compile(project);
    let text = adapter.render(&RenderInput {
        app_version,
        project_name: &project.name,
        enabled: true,
        compiled: &compiled,
        generated_at: &now_rfc3339(),
    })?;
    Ok((text, compiled, issues))
}

/// 生成并落盘。
pub fn export(
    project: &Project,
    paths: &ExportPaths,
    enabled: bool,
    app_version: &str,
    model_schema: u32,
    target_id: &str,
    backup_limit: usize,
) -> Result<Outcome, ExportError> {
    let adapter: Box<dyn Adapter> = adapters::by_id(target_id)
        .ok_or_else(|| ExportError::UnsupportedTarget(target_id.to_string()))?;

    let mut steps = Vec::new();
    let issues = validator::validate(project);
    let warnings: Vec<Issue> = issues
        .iter()
        .filter(|i| i.severity == crate::validator::Severity::Warning)
        .cloned()
        .collect();
    if has_errors(&issues) {
        return Err(ExportError::Validation(issues));
    }

    let compiled = compiler::compile(project);
    steps.push(format!(
        "编译规则：{} 条（跳过 {} 条无效规则）",
        compiled.rules.len(),
        compiled.skipped.len()
    ));
    for s in &compiled.skipped {
        steps.push(format!(
            "跳过规则 {}（{}）：{}",
            s.rule_name, s.rule_id, s.reason
        ));
    }

    let generated_at = now_rfc3339();
    let text = adapter.render(&RenderInput {
        app_version,
        project_name: &project.name,
        enabled,
        compiled: &compiled,
        generated_at: &generated_at,
    })?;
    let bytes = text.as_bytes().to_vec();
    let sha256 = writer::sha256_hex(&bytes);

    std::fs::create_dir_all(&paths.output_dir).map_err(|e| ExportError::CreateDir {
        path: paths.output_dir.display().to_string(),
        source: e,
    })?;

    // 1) 先备份旧文件（失败就中止，避免"悄悄丢历史"）
    let backup_path = match backup::backup_file(&paths.script, &paths.backups_dir) {
        Ok(p) => p,
        Err(e) => return Err(ExportError::Backup(e.to_string())),
    };
    if let Some(p) = &backup_path {
        steps.push(format!("已备份上一版：{}", p.display()));
    } else {
        steps.push("首次生成，没有旧文件需要备份".to_string());
    }

    // 2) 原子替换脚本
    writer::atomic_write_bytes(&paths.script, &bytes).map_err(|e| ExportError::Write {
        path: paths.script.display().to_string(),
        source: e,
    })?;
    steps.push(format!(
        "写入 {}（{} 字节，sha256 {}…）",
        paths.script.display(),
        bytes.len(),
        &sha256[..12.min(sha256.len())]
    ));

    // 3) manifest
    let file_name = paths
        .script
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| ExportError::BadPath(paths.script.display().to_string()))?
        .to_string();
    let mf = Manifest {
        app: "RelayBridge".to_string(),
        app_version: app_version.to_string(),
        model_schema,
        target: adapter.id().to_string(),
        target_version: adapter.target_version().to_string(),
        generated_at: generated_at.clone(),
        file_name,
        sha256: sha256.clone(),
        size: bytes.len() as u64,
        enabled,
        rule_count: compiled.rules.len(),
        project_name: project.name.clone(),
    };
    manifest::write(&paths.manifest, &mf).map_err(|e| ExportError::Write {
        path: paths.manifest.display().to_string(),
        source: e,
    })?;
    steps.push(format!("写入 {}", paths.manifest.display()));

    // 4) 清理旧备份（只删自己的，且保留数量有上限）
    match backup::prune(&paths.backups_dir, backup_limit) {
        Ok(n) if n > 0 => steps.push(format!(
            "清理旧备份 {} 份（保留最近 {} 份）",
            n, backup_limit
        )),
        Ok(_) => {}
        Err(e) => steps.push(format!("清理旧备份失败（不影响本次生成）：{}", e)),
    }

    Ok(Outcome {
        script_path: paths.script.clone(),
        manifest_path: paths.manifest.clone(),
        sha256,
        bytes: bytes.len() as u64,
        rule_count: compiled.rules.len(),
        generated_at,
        backup: backup_path,
        enabled,
        steps,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Action, Match, Rule};

    fn tmpdir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("rb-export-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn paths(root: &std::path::Path) -> ExportPaths {
        ExportPaths {
            output_dir: root.join("output/clash-verge-rev"),
            script: root.join("output/clash-verge-rev/relaybridge.js"),
            manifest: root.join("output/clash-verge-rev/manifest.json"),
            backups_dir: root.join("backups"),
        }
    }

    fn project() -> Project {
        Project {
            name: "Demo".into(),
            rules: vec![Rule {
                id: "r1".into(),
                name: "UU".into(),
                enabled: true,
                match_: Match {
                    domain: vec!["nrd.nie.163.com".into()],
                    ip_cidr: vec![],
                },
                action: Action::Proxy {
                    outbound: "Proxy".into(),
                },
            }],
            ..Default::default()
        }
    }

    #[test]
    fn first_export_then_regenerate_backs_up() {
        let d = tmpdir("first");
        let p = paths(&d);
        let o1 = export(&project(), &p, true, "1.1.0", 1, "clash-verge-rev", 5).unwrap();
        assert!(o1.backup.is_none());
        assert!(o1.script_path.exists());
        let text = std::fs::read_to_string(&o1.script_path).unwrap();
        assert!(text.contains("DOMAIN-SUFFIX,nrd.nie.163.com,Proxy"));

        let o2 = export(&project(), &p, false, "1.1.0", 1, "clash-verge-rev", 5).unwrap();
        assert!(o2.backup.is_some(), "第二次生成必须备份上一版");
        let text2 = std::fs::read_to_string(&o2.script_path).unwrap();
        assert!(text2.contains("var RB_ENABLED = false;"));
        let mf = manifest::read(&p.manifest).unwrap();
        assert!(!mf.enabled);
        assert_eq!(mf.sha256, o2.sha256);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn validation_error_writes_nothing() {
        let d = tmpdir("invalid");
        let p = paths(&d);
        let mut proj = project();
        proj.rules[0].match_.domain = vec!["bad domain".into()];
        let err = export(&proj, &p, true, "1.1.0", 1, "clash-verge-rev", 5).unwrap_err();
        assert!(matches!(err, ExportError::Validation(_)));
        assert!(!p.script.exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn unsupported_target_is_rejected() {
        let d = tmpdir("target");
        let p = paths(&d);
        let err = export(&project(), &p, true, "1.1.0", 1, "v2ray", 5).unwrap_err();
        assert!(matches!(err, ExportError::UnsupportedTarget(_)));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn determinism_except_timestamp() {
        let d = tmpdir("determinism");
        let p = paths(&d);
        let a = export(&project(), &p, true, "1.1.0", 1, "clash-verge-rev", 5).unwrap();
        let b = export(&project(), &p, true, "1.1.0", 1, "clash-verge-rev", 5).unwrap();
        let ta = std::fs::read_to_string(&a.script_path).unwrap();
        let tb = std::fs::read_to_string(&b.script_path).unwrap();
        let strip = |s: &str| {
            s.lines()
                .filter(|l| !l.contains("生成于") && !l.contains("状态:"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(strip(&ta), strip(&tb));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
