//! 统一配置模型。与任何具体代理内核格式解耦。

use serde::{Deserialize, Serialize};

/// 配置模型版本，随字段语义变化递增。
pub const MODEL_SCHEMA_VERSION: u32 = 1;

fn default_schema() -> u32 {
    MODEL_SCHEMA_VERSION
}

fn default_project_name() -> String {
    "Default Project".to_string()
}

fn default_true() -> bool {
    true
}

/// 一个项目 = 一组用户规则。对应 project.json。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    #[serde(default = "default_schema")]
    pub schema_version: u32,
    #[serde(default = "default_project_name")]
    pub name: String,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            schema_version: MODEL_SCHEMA_VERSION,
            name: default_project_name(),
            // 预置一条 UU 规则：全新安装后打开开关即可工作
            rules: vec![Rule {
                id: "uu-default".to_string(),
                name: "UU 远程".to_string(),
                enabled: true,
                action: Action::Proxy {
                    outbound: "Proxy".to_string(),
                },
                match_: Match {
                    domain: crate::presets::uu_domains(),
                    ip_cidr: Vec::new(),
                },
            }],
        }
    }
}

/// 单条规则。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, rename = "match")]
    pub match_: Match,
    pub action: Action,
}

/// 匹配条件。域名与 IP/CIDR 同时存在时，命中任意一个即生效。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Match {
    #[serde(default)]
    pub domain: Vec<String>,
    #[serde(default)]
    pub ip_cidr: Vec<String>,
}

impl Match {
    pub fn is_empty(&self) -> bool {
        self.domain.iter().all(|d| d.trim().is_empty())
            && self.ip_cidr.iter().all(|c| c.trim().is_empty())
    }
}

/// 命中后的动作。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Action {
    /// 交给指定策略组（该名称必须存在于用户自己的配置里）。
    Proxy { outbound: String },
    /// 直连。
    Direct,
    /// 拒绝。
    Reject,
}

impl Action {
    /// 生成规则串时使用的目标字段。
    pub fn target(&self) -> &str {
        match self {
            Action::Proxy { outbound } => outbound,
            Action::Direct => "DIRECT",
            Action::Reject => "REJECT",
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Action::Proxy { .. } => "proxy",
            Action::Direct => "direct",
            Action::Reject => "reject",
        }
    }

    pub fn is_valid(&self) -> bool {
        match self {
            Action::Proxy { outbound } => {
                let t = outbound.trim();
                !t.is_empty() && !t.contains(',') && !t.contains('\n') && !t.contains('\r')
            }
            _ => true,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Action::Proxy { outbound } => format!("代理 → {}", outbound.trim()),
            Action::Direct => "直连 DIRECT".to_string(),
            Action::Reject => "拒绝 REJECT".to_string(),
        }
    }
}

/// 生成一个稳定可读的规则 id（时间 + 序号，避免依赖随机数）。
pub fn new_rule_id(existing: &[Rule]) -> String {
    let mut n = existing.len() + 1;
    loop {
        let id = format!("rule-{:03}", n);
        if !existing.iter().any(|r| r.id == id) {
            return id;
        }
        n += 1;
    }
}
