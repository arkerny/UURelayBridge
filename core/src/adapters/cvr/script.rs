//! 生成 relaybridge.js 的文本。纯字符串拼接，便于快照测试。

use crate::adapters::RenderInput;

/// 把用户可控文本压成单行，避免破坏注释块。
fn one_line(s: &str) -> String {
    s.chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect::<String>()
        .replace("*/", "*\\/")
        .trim()
        .to_string()
}

fn js_string(s: &str) -> String {
    // JSON 字符串字面量与 JS 字符串字面量在这里完全兼容
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

pub fn render(input: &RenderInput<'_>) -> String {
    let mut out = String::with_capacity(1024 + input.compiled.rules.len() * 64);

    let state = if input.enabled { "ON" } else { "OFF" };
    out.push_str("/* ==========================================================================\n");
    out.push_str(" * RelayBridge — Clash Verge Rev 扩展脚本\n");
    out.push_str(&format!(
        " * 由 RelayBridge v{} 生成于 {}\n",
        one_line(input.app_version),
        one_line(input.generated_at)
    ));
    out.push_str(&format!(" * 项目: {}\n", one_line(input.project_name)));
    out.push_str(&format!(
        " * 状态: {} · 规则 {} 条\n",
        state,
        input.compiled.rules.len()
    ));
    out.push_str(" *\n");
    out.push_str(" * 手动接入（UURelayBridge 不会自动导入，也不会改动 Clash Verge 的任何文件）：\n");
    out.push_str(" *   Clash Verge Rev → 订阅 → 选中配置 → 右键「编辑扩展脚本」→ 粘贴本文件 → 保存\n");
    out.push_str(" *\n");
    out.push_str(" * 本脚本只做两件事：\n");
    out.push_str(" *   1) 按 ON/OFF 把自己的规则插到 config.rules 前面；\n");
    out.push_str(" *   2) 若启用了 SNI 联动，把敏感域名指到本机转发器（dns.hosts + DIRECT）。\n");
    out.push_str(" * 不清空、不排序、不改写用户已有规则 / 代理组 / 节点。\n");

    let sni = input.sni.filter(|s| s.enabled && !s.domains.is_empty());
    if let Some(s) = sni {
        out.push_str(" *\n");
        out.push_str(&format!(
            " * SNI 处理：{} 个域名 · {} · {} 段\n",
            s.domains.len(),
            match s.mode.as_str() {
                "segment" => "TCP 分段",
                "record" => "TLS Record 分片",
                _ => "纯转发",
            },
            s.parts
        ));
        if let Some(port) = s.socks_port {
            out.push_str(&format!(
                " * 走本机 SOCKS5 127.0.0.1:{port}（UURelayBridge 负责分片），需保持 UURelayBridge 运行。\n"
            ));
        } else {
            out.push_str(" * 域名会被指到 127.0.0.1:443，由 UURelayBridge 的 Host 模式做分片转发。\n");
        }
    }
    out.push_str(" * ========================================================================== */\n\n");

    out.push_str("function main(config) {\n");
    out.push_str("  \"use strict\";\n\n");
    out.push_str(&format!("  var RB_ENABLED = {};\n", input.enabled));
    out.push_str("  var RB_RULES = [\n");
    for r in &input.compiled.rules {
        out.push_str("    ");
        out.push_str(&js_string(r));
        out.push_str(",\n");
    }
    out.push_str("  ];\n\n");

    // SNI 联动：域名列表 + 目标（本机 SOCKS5 或 本机 443）
    let sni_domains: Vec<&String> = sni.map(|s| s.domains.iter().collect()).unwrap_or_default();
    let socks_port = sni.and_then(|s| s.socks_port);
    let socks_name = sni
        .map(|s| s.socks_name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "UURelayBridge-SNI".to_string());

    out.push_str(&format!(
        "  var RB_SNI_ENABLED = {};\n",
        !sni_domains.is_empty()
    ));
    out.push_str("  var RB_SNI_DOMAINS = [\n");
    for d in &sni_domains {
        out.push_str("    ");
        out.push_str(&js_string(d));
        out.push_str(",\n");
    }
    out.push_str("  ];\n");
    out.push_str(&format!("  var RB_SOCKS_PORT = {};\n", match socks_port {
        Some(p) => p.to_string(),
        None => "0".to_string(),
    }));
    out.push_str(&format!("  var RB_SOCKS_NAME = {};\n", js_string(&socks_name)));
    out.push_str("  var RB_SNI_RULES = RB_SNI_DOMAINS.map(function (d) {\n");
    out.push_str("    var target = RB_SOCKS_PORT ? RB_SOCKS_NAME : \"DIRECT\";\n");
    out.push_str("    // 通配符按 Clash 语义换算（星号、星号点子域、加号点子域）\n");
    out.push_str("    if (d === \"*\") { return \"MATCH,\" + target; }\n");
    out.push_str("    if (d.indexOf(\"*.\") === 0) { return \"DOMAIN-SUFFIX,\" + d.slice(2) + \",\" + target; }\n");
    out.push_str("    if (d.indexOf(\"+.\") === 0) { return \"DOMAIN-SUFFIX,\" + d.slice(2) + \",\" + target; }\n");
    out.push_str("    if (d.indexOf(\"*\") >= 0) {\n");
    out.push_str("      var rx = d.split(\"*\").map(function (s) { return s.replace(/[.]/g, \"[.]\"); }).join(\"[^.]*\");\n");
    out.push_str("      return \"DOMAIN-REGEX,^\" + rx + \"$,\" + target;\n");
    out.push_str("    }\n");
    out.push_str("    return \"DOMAIN-SUFFIX,\" + d + \",\" + target;\n  });\n\n");

    out.push_str("  if (!config || typeof config !== \"object\") {\n    config = {};\n  }\n");
    out.push_str("  var existing = Array.isArray(config.rules) ? config.rules.slice() : [];\n\n");

    out.push_str("  // 幂等：先摘掉上一次注入的完全相同的规则，避免重复加载时越加越多\n");
    out.push_str("  var ours = RB_SNI_RULES.concat(RB_RULES);\n");
    out.push_str("  var kept = [];\n");
    out.push_str("  for (var i = 0; i < existing.length; i++) {\n");
    out.push_str("    if (ours.indexOf(existing[i]) === -1) {\n      kept.push(existing[i]);\n    }\n  }\n\n");

    out.push_str("  // 只动 config.rules；用户其他字段一律不碰\n");
    out.push_str("  config.rules = RB_ENABLED ? ours.concat(kept) : kept;\n\n");

    out.push_str("  // SNI 联动：SOCKS 模式下注入本机代理；否则把域名指到本机 443\n");
    out.push_str("  try {\n");
    out.push_str("    var wantSni = RB_ENABLED && RB_SNI_ENABLED;\n");
    out.push_str("    if (RB_SOCKS_PORT > 0) {\n");
    out.push_str("      var list = Array.isArray(config.proxies) ? config.proxies.slice() : [];\n");
    out.push_str("      list = list.filter(function (p) { return !p || p.name !== RB_SOCKS_NAME; });\n");
    out.push_str("      if (wantSni) {\n");
    out.push_str("        list.push({ name: RB_SOCKS_NAME, type: \"socks5\", server: \"127.0.0.1\", port: RB_SOCKS_PORT, udp: false });\n");
    out.push_str("      }\n");
    out.push_str("      config.proxies = list;\n");
    out.push_str("    }\n");
    out.push_str("    // 清理旧方案（dns.hosts → 127.0.0.1）留下的记录\n");
    out.push_str("    var dns = config.dns;\n");
    out.push_str("    if (dns && typeof dns === \"object\") {\n");
    out.push_str("      var hosts = (dns.hosts && typeof dns.hosts === \"object\") ? dns.hosts : null;\n");
    out.push_str("      var legacy = RB_SOCKS_PORT === 0 && wantSni;\n");
    out.push_str("      if (hosts) {\n");
    out.push_str("        for (var k = 0; k < RB_SNI_DOMAINS.length; k++) {\n");
    out.push_str("          var d = RB_SNI_DOMAINS[k];\n");
    out.push_str("          if (legacy) { hosts[d] = \"127.0.0.1\"; }\n");
    out.push_str("          else if (hosts[d] === \"127.0.0.1\") { delete hosts[d]; }\n");
    out.push_str("        }\n");
    out.push_str("        dns.hosts = hosts;\n");
    out.push_str("        config.dns = dns;\n");
    out.push_str("      } else if (legacy && typeof console !== \"undefined\") {\n");
    out.push_str("        console.warn(\"[UURelayBridge] 没有 dns 段，无法把域名指到本机 443；建议改用 SOCKS 模式。\");\n");
    out.push_str("      }\n");
    out.push_str("    }\n");
    out.push_str("  } catch (e) { /* 只影响 SNI 联动，不动其它配置 */ }\n\n");

    out.push_str("  // 仅告警：策略组在本配置里不存在时提示（不修改任何东西）\n");
    out.push_str("  try {\n");
    out.push_str("    var groups = [];\n");
    out.push_str("    var pg = config[\"proxy-groups\"];\n");
    out.push_str("    if (Array.isArray(pg)) {\n");
    out.push_str("      for (var g = 0; g < pg.length; g++) {\n");
    out.push_str("        if (pg[g] && pg[g].name) { groups.push(pg[g].name); }\n      }\n    }\n");
    out.push_str("    var missing = [];\n");
    out.push_str("    for (var k = 0; k < RB_RULES.length; k++) {\n");
    out.push_str("      var parts = RB_RULES[k].split(\",\");\n");
    out.push_str("      var target = parts[parts.length - 1];\n");
    out.push_str("      if (target === \"DIRECT\" || target === \"REJECT\" || target === \"REJECT-DROP\") { continue; }\n");
    out.push_str("      if (missing.indexOf(target) === -1 && groups.indexOf(target) === -1) { missing.push(target); }\n");
    out.push_str("    }\n");
    out.push_str("    if (missing.length > 0 && typeof console !== \"undefined\" && console.warn) {\n");
    out.push_str("      console.warn(\"[RelayBridge] 这些策略组在你的配置里不存在: \" + missing.join(\", \"));\n");
    out.push_str("    }\n");
    out.push_str("  } catch (e) { /* 只是提示，绝不影响配置 */ }\n\n");

    out.push_str("  return config;\n");
    out.push_str("}\n");

    if input.compiled.rules.is_empty() {
        out.push_str("\n// 注：当前没有可注入的规则（OFF 状态或规则全部无效）。\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::Compiled;

    fn input(enabled: bool, rules: Vec<&str>) -> (Compiled, bool) {
        (
            Compiled {
                rules: rules.into_iter().map(String::from).collect(),
                ..Default::default()
            },
            enabled,
        )
    }

    fn render_with(enabled: bool, rules: Vec<&str>) -> String {
        let (compiled, en) = input(enabled, rules);
        let i = RenderInput {
            app_version: "1.1.0",
            project_name: "Demo",
            enabled: en,
            compiled: &compiled,
            generated_at: "2026-01-01T00:00:00+08:00",
            sni: None,
        };
        render(&i)
    }

    #[test]
    fn contains_main_and_rules() {
        let s = render_with(true, vec!["DOMAIN-SUFFIX,nrd.nie.163.com,Proxy"]);
        assert!(s.contains("function main(config)"));
        assert!(s.contains("var RB_ENABLED = true;"));
        assert!(s.contains("\"DOMAIN-SUFFIX,nrd.nie.163.com,Proxy\""));
        assert!(s.contains("return config;"));
    }

    #[test]
    fn off_keeps_rules_list_for_cleanup() {
        let s = render_with(false, vec!["DOMAIN,a.com,DIRECT"]);
        assert!(s.contains("var RB_ENABLED = false;"));
        // OFF 时仍要列出规则，才能在下次加载时把它们摘掉
        assert!(s.contains("\"DOMAIN,a.com,DIRECT\""));
    }

    #[test]
    fn empty_rules_render_valid_script() {
        let s = render_with(true, vec![]);
        assert!(s.contains("var RB_RULES = ["));
        assert!(s.contains("return config;"));
        assert!(s.contains("当前没有可注入的规则"));
    }

    #[test]
    fn comment_text_cannot_break_out() {
        let compiled = Compiled::default();
        let i = RenderInput {
            app_version: "1.1.0",
            project_name: "evil */ project\nname",
            enabled: true,
            compiled: &compiled,
            generated_at: "2026-01-01T00:00:00+08:00",
            sni: None,
        };
        let s = render(&i);
        // 模板自身有 3 处注释结束符（头部 + 两处行内 JS 注释）；用户文本里的 */ 必须被转义
        assert_eq!(s.matches("*/").count(), 3, "用户文本不得制造新的注释结束符");
        assert!(!s.contains("project\nname"));
        assert!(!s.contains("evil */ project"));
    }
}
