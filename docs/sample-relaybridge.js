/* ==========================================================================
 * RelayBridge — Clash Verge Rev 扩展脚本
 * 由 RelayBridge v1.2.0 生成于 2026-11-23T19:30:00+08:00
 * 项目: UU 远程
 * 状态: ON · 规则 3 条
 *
 * 手动接入（RelayBridge 不会自动导入，也不会改动 Clash Verge 的任何文件）：
 *   Clash Verge Rev → 订阅 → 选中配置 → 右键「编辑扩展脚本」→ 粘贴本文件 → 保存
 *
 * 本脚本只做一件事：按 ON/OFF 把自己的规则插到 config.rules 前面。
 * 不清空、不排序、不改写用户已有规则 / 代理组 / 节点。
 * ========================================================================== */

function main(config) {
  "use strict";

  var RB_ENABLED = true;
  var RB_RULES = [
    "DOMAIN-SUFFIX,nrd.nie.163.com,🔰 节点选择",
    "IP-CIDR,192.168.0.0/16,DIRECT",
    "IP-CIDR,10.0.0.0/8,DIRECT",
  ];

  if (!config || typeof config !== "object") {
    config = {};
  }
  var existing = Array.isArray(config.rules) ? config.rules.slice() : [];

  // 幂等：先摘掉上一次注入的完全相同的规则，避免重复加载时越加越多
  var kept = [];
  for (var i = 0; i < existing.length; i++) {
    if (RB_RULES.indexOf(existing[i]) === -1) {
      kept.push(existing[i]);
    }
  }

  // 只动 config.rules；用户其他字段一律不碰
  config.rules = RB_ENABLED ? RB_RULES.concat(kept) : kept;

  // 仅告警：策略组在本配置里不存在时提示（不修改任何东西）
  try {
    var groups = [];
    var pg = config["proxy-groups"];
    if (Array.isArray(pg)) {
      for (var g = 0; g < pg.length; g++) {
        if (pg[g] && pg[g].name) { groups.push(pg[g].name); }
      }
    }
    var missing = [];
    for (var k = 0; k < RB_RULES.length; k++) {
      var parts = RB_RULES[k].split(",");
      var target = parts[parts.length - 1];
      if (target === "DIRECT" || target === "REJECT" || target === "REJECT-DROP") { continue; }
      if (missing.indexOf(target) === -1 && groups.indexOf(target) === -1) { missing.push(target); }
    }
    if (missing.length > 0 && typeof console !== "undefined" && console.warn) {
      console.warn("[RelayBridge] 这些策略组在你的配置里不存在: " + missing.join(", "));
    }
  } catch (e) { /* 只是提示，绝不影响配置 */ }

  return config;
}
