//! 把统一模型编译成 Clash 规则串。纯函数，不做 IO。

use crate::model::{Action, Project};
use crate::rules::compile_entry;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub struct Skipped {
    pub rule_id: String,
    pub rule_name: String,
    pub reason: String,
}

#[derive(Debug, Default, Clone)]
pub struct Compiled {
    /// 已经按模型顺序展开成 Clash 规则串。
    pub rules: Vec<String>,
    /// 被跳过的启用规则（内容非法）。
    pub skipped: Vec<Skipped>,
    /// 参与编译的规则 id（启用且有效）。
    pub used_rule_ids: Vec<String>,
}

impl Compiled {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// 编译项目。只有 enabled 且能通过语法检查的规则才会产出规则串。
pub fn compile(project: &Project) -> Compiled {
    let mut out = Compiled::default();
    let mut seen: HashSet<String> = HashSet::new();

    for rule in project.rules.iter().filter(|r| r.enabled) {
        if !rule.action.is_valid() {
            out.skipped.push(Skipped {
                rule_id: rule.id.clone(),
                rule_name: display_name(rule),
                reason: match &rule.action {
                    Action::Proxy { outbound } if outbound.trim().is_empty() => {
                        "动作是代理，但没有填写策略组名称".to_string()
                    }
                    _ => "策略组名称不能包含逗号或换行".to_string(),
                },
            });
            continue;
        }
        if rule.match_.is_empty() {
            out.skipped.push(Skipped {
                rule_id: rule.id.clone(),
                rule_name: display_name(rule),
                reason: "规则没有任何匹配条件".to_string(),
            });
            continue;
        }

        let domain = rule.match_.domain.join("\n");
        let cidr = rule.match_.ip_cidr.join("\n");
        match compile_entry(&domain, &cidr, rule.action.target()) {
            Ok(list) => {
                let mut added = false;
                for r in list {
                    if seen.insert(r.clone()) {
                        out.rules.push(r);
                        added = true;
                    }
                }
                if added {
                    out.used_rule_ids.push(rule.id.clone());
                }
            }
            Err(e) => out.skipped.push(Skipped {
                rule_id: rule.id.clone(),
                rule_name: display_name(rule),
                reason: e,
            }),
        }
    }
    out
}

fn display_name(rule: &crate::model::Rule) -> String {
    if rule.name.trim().is_empty() {
        rule.id.clone()
    } else {
        rule.name.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Action, Match, Rule};

    fn rule(id: &str, domains: &[&str], outbound: &str, enabled: bool) -> Rule {
        Rule {
            id: id.into(),
            name: id.into(),
            enabled,
            match_: Match {
                domain: domains.iter().map(|s| s.to_string()).collect(),
                ip_cidr: vec![],
            },
            action: Action::Proxy {
                outbound: outbound.into(),
            },
        }
    }

    #[test]
    fn empty_project_compiles_to_nothing() {
        let c = compile(&Project::default());
        assert!(c.is_empty());
        assert!(c.skipped.is_empty());
    }

    #[test]
    fn disabled_rules_are_ignored() {
        let p = Project {
            rules: vec![rule("a", &["a.com"], "P", false)],
            ..Default::default()
        };
        assert!(compile(&p).is_empty());
    }

    #[test]
    fn order_and_dedup_follow_model_order() {
        let p = Project {
            rules: vec![
                rule("a", &["a.com", "b.com"], "P", true),
                rule("b", &["b.com"], "P", true),
            ],
            ..Default::default()
        };
        let c = compile(&p);
        assert_eq!(c.rules, vec!["DOMAIN-SUFFIX,a.com,P", "DOMAIN-SUFFIX,b.com,P"]);
        // 规则 b 的条目和规则 a 完全重复，没有产出新规则串，因此不计入 used_rule_ids
        assert_eq!(c.used_rule_ids, vec!["a"]);
    }

    #[test]
    fn every_contributing_rule_is_recorded() {
        let p = Project {
            rules: vec![
                rule("a", &["a.com"], "P", true),
                rule("b", &["b.com"], "P", true),
            ],
            ..Default::default()
        };
        assert_eq!(compile(&p).used_rule_ids, vec!["a", "b"]);
    }

    #[test]
    fn invalid_rule_is_skipped_not_fatal() {
        let p = Project {
            rules: vec![rule("bad", &["ht!tp://x"], "P", true), rule("ok", &["ok.com"], "P", true)],
            ..Default::default()
        };
        let c = compile(&p);
        assert_eq!(c.rules.len(), 1);
        assert_eq!(c.skipped.len(), 1);
        assert_eq!(c.skipped[0].rule_id, "bad");
    }

    #[test]
    fn proxy_without_group_is_skipped() {
        let p = Project {
            rules: vec![rule("a", &["a.com"], "  ", true)],
            ..Default::default()
        };
        let c = compile(&p);
        assert!(c.is_empty());
        assert_eq!(c.skipped.len(), 1);
    }
}
