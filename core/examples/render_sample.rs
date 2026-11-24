//! 生成一份示例扩展脚本：
//!   cargo run -p relaybridge-core --example render_sample > docs/sample-relaybridge.js
//!
//! 用真实的 UU 远程控制面域名做例子，方便用户对照自己生成的结果。

use uurelaybridge_core::adapters::cvr::ClashVergeRev;
use uurelaybridge_core::adapters::{Adapter, RenderInput};
use uurelaybridge_core::compiler;
use uurelaybridge_core::model::{Action, Match, Project, Rule};

fn main() {
    let project = Project {
        schema_version: 1,
        name: "UU 远程".into(),
        rules: vec![
            Rule {
                id: "rule-001".into(),
                name: "UU 远程控制面".into(),
                enabled: true,
                match_: Match {
                    domain: vec!["nrd.nie.163.com".into()],
                    ip_cidr: vec![],
                },
                action: Action::Proxy {
                    outbound: "🔰 节点选择".into(),
                },
            },
            Rule {
                id: "rule-002".into(),
                name: "局域网直连".into(),
                enabled: true,
                match_: Match {
                    domain: vec![],
                    ip_cidr: vec!["192.168.0.0/16".into(), "10.0.0.0/8".into()],
                },
                action: Action::Direct,
            },
        ],
    };

    let compiled = compiler::compile(&project);
    let script = ClashVergeRev
        .render(&RenderInput {
            app_version: env!("CARGO_PKG_VERSION"),
            project_name: &project.name,
            enabled: true,
            compiled: &compiled,
            generated_at: "2026-11-23T19:30:00+08:00",
            sni: None,
        })
        .expect("render");
    print!("{script}");
}
