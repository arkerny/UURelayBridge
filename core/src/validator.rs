//! 生成前校验：给出面向用户的错误/警告清单，规则本身不被修改。

use crate::model::{Action, Project};
use crate::rules::{normalize_cidr, validate_domain};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Issue {
    pub severity: Severity,
    pub rule_id: Option<String>,
    pub field: String,
    pub message: String,
}

impl Issue {
    fn err(rule_id: Option<&str>, field: &str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            rule_id: rule_id.map(|s| s.to_string()),
            field: field.to_string(),
            message: message.into(),
        }
    }
    fn warn(rule_id: Option<&str>, field: &str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            rule_id: rule_id.map(|s| s.to_string()),
            field: field.to_string(),
            message: message.into(),
        }
    }
}

pub fn has_errors(issues: &[Issue]) -> bool {
    issues.iter().any(|i| i.severity == Severity::Error)
}

/// 校验整个项目。
pub fn validate(project: &Project) -> Vec<Issue> {
    let mut issues = Vec::new();

    if project.schema_version > crate::model::MODEL_SCHEMA_VERSION {
        issues.push(Issue::err(
            None,
            "schema_version",
            format!(
                "project.json 版本 {} 高于本程序支持的 {}，请升级 RelayBridge",
                project.schema_version,
                crate::model::MODEL_SCHEMA_VERSION
            ),
        ));
    }
    if project.rules.is_empty() {
        issues.push(Issue::warn(None, "rules", "还没有任何规则，生成的脚本不会注入内容"));
    }

    let mut ids = std::collections::HashSet::new();
    let mut targets = std::collections::HashSet::new();
    let mut enabled = 0usize;

    for rule in &project.rules {
        let rid = rule.id.trim();
        if rid.is_empty() {
            issues.push(Issue::err(None, "id", "存在没有 id 的规则"));
        } else if !ids.insert(rid.to_string()) {
            issues.push(Issue::err(Some(rid), "id", "规则 id 重复"));
        }
        if rule.name.trim().is_empty() {
            issues.push(Issue::warn(Some(rid), "name", "规则没有名称，界面里会显示 id"));
        }
        if rule.enabled {
            enabled += 1;
        }
        if rule.match_.is_empty() {
            issues.push(Issue::err(
                Some(rid),
                "match",
                "规则至少需要一个域名或一个 IP/CIDR",
            ));
            continue;
        }
        for d in rule.match_.domain.iter().filter(|s| !s.trim().is_empty()) {
            if let Err(e) = validate_domain(d) {
                issues.push(Issue::err(Some(rid), "match.domain", format!("`{}`：{}", d.trim(), e)));
            }
        }
        for c in rule.match_.ip_cidr.iter().filter(|s| !s.trim().is_empty()) {
            if let Err(e) = normalize_cidr(c) {
                issues.push(Issue::err(Some(rid), "match.ip_cidr", format!("`{}`：{}", c.trim(), e)));
            }
        }
        if let Action::Proxy { outbound } = &rule.action {
            if outbound.trim().is_empty() {
                issues.push(Issue::err(Some(rid), "action.outbound", "代理动作必须填写策略组名称"));
            } else if outbound.contains(',') {
                issues.push(Issue::err(Some(rid), "action.outbound", "策略组名称不能包含逗号"));
            }
        }
        if rule.enabled {
            let key = format!("{}|{}", rule.match_.domain.join(","), rule.match_.ip_cidr.join(","));
            let _ = key;
            if !targets.insert(format!("{}", rule.action.describe())) {
                // 只是提示，不算错误
            }
        }
    }

    if !project.rules.is_empty() && enabled == 0 {
        issues.push(Issue::warn(None, "rules", "所有规则都被禁用了"));
    }
    if project.rules.len() > 2000 {
        issues.push(Issue::warn(None, "rules", "规则数量很大，生成的脚本会变长"));
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Action, Match, Rule};

    fn r(id: &str, domains: Vec<&str>, action: Action) -> Rule {
        Rule {
            id: id.into(),
            name: id.into(),
            enabled: true,
            match_: Match {
                domain: domains.into_iter().map(String::from).collect(),
                ip_cidr: vec![],
            },
            action,
        }
    }

    #[test]
    fn valid_project_has_no_errors() {
        let p = Project {
            rules: vec![
                r("a", vec!["a.com", "*.b.com"], Action::Proxy { outbound: "P".into() }),
                r("b", vec!["c.com"], Action::Direct),
            ],
            ..Default::default()
        };
        let issues = validate(&p);
        assert!(!has_errors(&issues), "{:?}", issues);
    }

    #[test]
    fn catches_bad_domain_and_empty_outbound() {
        let p = Project {
            rules: vec![
                r("a", vec!["bad domain"], Action::Direct),
                r("b", vec!["ok.com"], Action::Proxy { outbound: " ".into() }),
            ],
            ..Default::default()
        };
        let issues = validate(&p);
        assert!(has_errors(&issues));
        assert_eq!(issues.iter().filter(|i| i.severity == Severity::Error).count(), 2);
    }

    #[test]
    fn empty_match_is_error() {
        let p = Project {
            rules: vec![r("a", vec![], Action::Direct)],
            ..Default::default()
        };
        assert!(has_errors(&validate(&p)));
    }

    #[test]
    fn wrong_schema_is_error() {
        let p = Project {
            schema_version: 99,
            ..Default::default()
        };
        assert!(has_errors(&validate(&p)));
    }
}
