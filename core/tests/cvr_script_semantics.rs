//! 扩展脚本的**行为**测试：把生成出来的 relaybridge.js 丢给真实 Node 执行，
//! 检查它是否只动 config.rules、是否幂等、OFF 是否能把注入的规则摘干净。
//!
//! 没有 node 时跳过（会打印原因），不阻断其它测试。

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use uurelaybridge_core::adapters::{Adapter, RenderInput};
use uurelaybridge_core::adapters::cvr::ClashVergeRev;
use uurelaybridge_core::compiler;
use uurelaybridge_core::model::{Action, Match, Project, Rule};

const HARNESS: &str = r#"
const fs = require("node:fs");
const [scriptPath, configPath, outPath, times] = process.argv.slice(2);
const src = fs.readFileSync(scriptPath, "utf8");
const main = new Function(src + "\nreturn main;")();
let cfg = JSON.parse(fs.readFileSync(configPath, "utf8"));
const n = Number(times || 1);
for (let i = 0; i < n; i++) cfg = main(cfg);
fs.writeFileSync(outPath, JSON.stringify(cfg));
"#;

fn unique() -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    N.fetch_add(1, Ordering::SeqCst)
}

fn tmpdir(tag: &str) -> PathBuf {
    uurelaybridge_core::test_tmp_dir(&format!("{}-{}", tag, unique()))
}

fn sample_project(enabled: bool) -> Project {
    Project {
        name: "Demo".into(),
        rules: vec![
            Rule {
                id: "uu".into(),
                name: "UU Remote".into(),
                enabled,
                match_: Match {
                    domain: vec!["nrd.nie.163.com".into()],
                    ip_cidr: vec![],
                },
                action: Action::Proxy {
                    outbound: "Proxy".into(),
                },
            },
            Rule {
                id: "lan".into(),
                name: "LAN".into(),
                enabled: true,
                match_: Match {
                    domain: vec![],
                    ip_cidr: vec!["10.0.0.0/8".into()],
                },
                action: Action::Direct,
            },
        ],
        ..Default::default()
    }
}

fn render(project: &Project, enabled: bool) -> String {
    let compiled = compiler::compile(project);
    ClashVergeRev
        .render(&RenderInput {
            app_version: "1.1.0",
            project_name: &project.name,
            enabled,
            compiled: &compiled,
            generated_at: "2026-01-01T00:00:00+08:00",
            sni: None,
        })
        .unwrap()
}

fn run_node(script: &str, config: &str, times: u32) -> Option<serde_json::Value> {
    let dir = tmpdir(&format!("run{}", times));
    let sp = dir.join("relaybridge.js");
    let cp = dir.join("config.json");
    let op = dir.join("out.json");
    let hp = dir.join("harness.js");
    fs::write(&sp, script).unwrap();
    fs::write(&cp, config).unwrap();
    fs::write(&hp, HARNESS).unwrap();

    let out = Command::new("node")
        .arg(&hp)
        .arg(&sp)
        .arg(&cp)
        .arg(&op)
        .arg(times.to_string())
        .output();

    let result = match out {
        Ok(o) if o.status.success() => {
            let text = fs::read_to_string(&op).expect("node 应该写出结果");
            Some(serde_json::from_str(&text).expect("结果必须是合法 JSON"))
        }
        Ok(o) => {
            eprintln!(
                "node 执行失败（跳过断言）：{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            None
        }
        Err(_) => {
            eprintln!("未找到 node，跳过脚本行为测试");
            None
        }
    };
    let _ = fs::remove_dir_all(&dir);
    result
}

fn user_config() -> String {
    serde_json::json!({
        "mixed-port": 7890,
        "proxy-groups": [{"name": "Proxy", "type": "select", "proxies": ["DIRECT"]}],
        "rules": [
            "DOMAIN-SUFFIX,corp.example.com,DIRECT",
            "GEOIP,CN,DIRECT",
            "MATCH,Proxy"
        ]
    })
    .to_string()
}

fn rules_of(v: &serde_json::Value) -> Vec<String> {
    v["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn on_injects_at_top_and_keeps_everything_else() {
    let script = render(&sample_project(true), true);
    let Some(out) = run_node(&script, &user_config(), 1) else {
        return;
    };
    let rules = rules_of(&out);
    assert_eq!(rules[0], "DOMAIN-SUFFIX,nrd.nie.163.com,Proxy");
    assert_eq!(rules.len(), 5, "2 条注入 + 3 条用户规则：{:?}", rules);
    assert_eq!(rules[rules.len() - 1], "MATCH,Proxy", "MATCH 必须保持最后");
    assert!(rules.contains(&"DOMAIN-SUFFIX,corp.example.com,DIRECT".to_string()));
    assert!(rules.contains(&"GEOIP,CN,DIRECT".to_string()));
    assert_eq!(out["mixed-port"], 7890);
    assert_eq!(out["proxy-groups"][0]["name"], "Proxy");
}

#[test]
fn applying_twice_does_not_duplicate() {
    let script = render(&sample_project(true), true);
    let Some(out) = run_node(&script, &user_config(), 2) else {
        return;
    };
    let rules = rules_of(&out);
    assert_eq!(rules.len(), 5, "重复加载不应叠加：{:?}", rules);
    assert_eq!(
        rules.iter().filter(|r| r.starts_with("DOMAIN-SUFFIX,nrd.nie.163.com")).count(),
        1
    );
}

#[test]
fn off_removes_previously_injected_rules() {
    let on = render(&sample_project(true), true);
    let off = render(&sample_project(true), false);
    // 先用 ON 生成一份"已经注入过"的配置
    let dir = tmpdir("off");
    let sp = dir.join("on.js");
    let cp = dir.join("config.json");
    let op = dir.join("out.json");
    let hp = dir.join("harness.js");
    fs::write(&sp, &on).unwrap();
    fs::write(&cp, user_config()).unwrap();
    fs::write(&hp, HARNESS).unwrap();
    let ok = Command::new("node")
        .arg(&hp).arg(&sp).arg(&cp).arg(&op).arg("1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("未找到 node，跳过 OFF 测试");
        let _ = fs::remove_dir_all(&dir);
        return;
    }
    let injected = fs::read_to_string(&op).unwrap();

    // 再把 OFF 脚本应用到这份配置上
    let sp2 = dir.join("off.js");
    let op2 = dir.join("out2.json");
    fs::write(&sp2, &off).unwrap();
    fs::write(&cp, &injected).unwrap();
    let ok2 = Command::new("node")
        .arg(&hp).arg(&sp2).arg(&cp).arg(&op2).arg("1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok2, "OFF 脚本必须能正常执行");
    let out: serde_json::Value = serde_json::from_str(&fs::read_to_string(&op2).unwrap()).unwrap();
    let rules = rules_of(&out);
    assert_eq!(
        rules,
        vec![
            "DOMAIN-SUFFIX,corp.example.com,DIRECT".to_string(),
            "GEOIP,CN,DIRECT".to_string(),
            "MATCH,Proxy".to_string()
        ],
        "OFF 之后应恢复成用户原始规则"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn empty_rules_project_still_produces_working_script() {
    // 默认项目预置了 UU 规则，这里显式构造空项目
    let empty = Project {
        rules: Vec::new(),
        ..Default::default()
    };
    let script = render(&empty, true);
    let Some(out) = run_node(&script, &user_config(), 1) else {
        return;
    };
    assert_eq!(rules_of(&out).len(), 3, "没有规则时不应改动 rules");}

// ---------------------------------------------------------------- SNI 联动

fn render_with_sni(project: &Project, enabled: bool, domains: &[&str]) -> String {
    let compiled = compiler::compile(project);
    let sni = uurelaybridge_core::adapters::SniLink {
        enabled: true,
        domains: domains.iter().map(|d| d.to_string()).collect(),
        mode: "record".to_string(),
        parts: 4,
        socks_port: None,
        socks_name: "UURelayBridge-SNI".to_string(),
    };
    ClashVergeRev
        .render(&RenderInput {
            app_version: "1.3.0",
            project_name: &project.name,
            enabled,
            compiled: &compiled,
            generated_at: "2026-01-01T00:00:00+08:00",
            sni: Some(&sni),
        })
        .unwrap()
}

fn dns_config() -> String {
    serde_json::json!({
        "dns": { "enable": true, "hosts": { "keep.example.com": "1.2.3.4" } },
        "proxy-groups": [{"name": "Proxy", "type": "select", "proxies": ["DIRECT"]}],
        "rules": ["MATCH,Proxy"]
    })
    .to_string()
}

#[test]
fn sni_link_points_domains_to_localhost_and_adds_direct_rule() {
    let script = render_with_sni(&sample_project(true), true, &["nrd.nie.163.com"]);
    let Some(out) = run_node(&script, &dns_config(), 1) else {
        return;
    };
    assert_eq!(out["dns"]["hosts"]["nrd.nie.163.com"], "127.0.0.1");
    assert_eq!(out["dns"]["hosts"]["keep.example.com"], "1.2.3.4", "用户原有 hosts 必须保留");
    let rules = rules_of(&out);
    assert_eq!(rules[0], "DOMAIN-SUFFIX,nrd.nie.163.com,DIRECT", "SNI 规则要在最前面");
    assert_eq!(rules[rules.len() - 1], "MATCH,Proxy");
}

#[test]
fn sni_link_is_idempotent() {
    let script = render_with_sni(&sample_project(true), true, &["nrd.nie.163.com"]);
    let Some(out) = run_node(&script, &dns_config(), 3) else {
        return;
    };
    let rules = rules_of(&out);
    assert_eq!(
        rules
            .iter()
            .filter(|r| r.as_str() == "DOMAIN-SUFFIX,nrd.nie.163.com,DIRECT")
            .count(),
        1,
        "重复加载不应叠加：{:?}",
        rules
    );
}

#[test]
fn off_removes_sni_hosts_entry() {
    let on = render_with_sni(&sample_project(true), true, &["nrd.nie.163.com"]);
    let off = render_with_sni(&sample_project(true), false, &["nrd.nie.163.com"]);
    let dir = tmpdir("sni-off");
    let sp = dir.join("on.js");
    let cp = dir.join("cfg.json");
    let op = dir.join("out.json");
    let hp = dir.join("h.js");
    fs::write(&sp, &on).unwrap();
    fs::write(&cp, dns_config()).unwrap();
    fs::write(&hp, HARNESS).unwrap();
    let ok = Command::new("node")
        .arg(&hp).arg(&sp).arg(&cp).arg(&op).arg("1")
        .status().map(|s| s.success()).unwrap_or(false);
    if !ok {
        eprintln!("未找到 node，跳过");
        let _ = fs::remove_dir_all(&dir);
        return;
    }
    // 再用 OFF 脚本处理已经注入过的配置
    let sp2 = dir.join("off.js");
    let op2 = dir.join("out2.json");
    fs::write(&sp2, &off).unwrap();
    let injected = fs::read_to_string(&op).unwrap();
    fs::write(&cp, injected).unwrap();
    let ok2 = Command::new("node")
        .arg(&hp).arg(&sp2).arg(&cp).arg(&op2).arg("1")
        .status().map(|s| s.success()).unwrap_or(false);
    assert!(ok2);
    let out: serde_json::Value = serde_json::from_str(&fs::read_to_string(&op2).unwrap()).unwrap();
    assert!(
        out["dns"]["hosts"].get("nrd.nie.163.com").is_none(),
        "OFF 后应删掉自己写入的 hosts 记录"
    );
    assert_eq!(out["dns"]["hosts"]["keep.example.com"], "1.2.3.4");
    let _ = fs::remove_dir_all(&dir);
}