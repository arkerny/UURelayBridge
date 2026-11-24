# SNI 模块设计（v1.3 实现状态）

本文是 SNI 阻断处理模块的设计说明与**实现状态**。原始设计要求见任务书；
这里只记录"现在实际有什么、和文档要求差在哪"。

---

## 1. 目标与不承诺

按域名选择性处理 TLS ClientHello 分片，用于绕过"按明文 SNI 简单匹配"的中间设备。

**不承诺**：DNS 污染、IP 封锁、QUIC 封锁、目标服务器故障、能完整重组 TLS 的设备、
主动探测类封锁 —— 这些不是分片能解决的。

## 2. 当前实现（Windows 优先，代码同源）

| 能力 | 状态 | 位置 |
|---|---|---|
| TLS ClientHello 解析（取 SNI） | ✅ 已实现 + 单测 | `core/src/sni/tls.rs` |
| TLS Record 分片（1 → N 条记录） | ✅ 已实现 + 单测 | 同上 |
| TCP 分段（带间隔） | ✅ 已实现 + 单测 | 同上 |
| 本地 TLS 转发（透明、不终止 TLS） | ✅ 已实现 + 端到端验证 | `src-tauri/src/hostmode/mod.rs` |
| DNS A 查询（绕过 hosts）+ 并行取快 + 缓存 | ✅ 已实现 + 单测 | `core/src/sni/dns.rs` |
| hosts 改写（只动自己那一段、幂等、可移除） | ✅ 已实现 + 单测 | `core/src/sni/hosts.rs` |
| 提权（Windows UAC / macOS osascript） | ✅ 已实现 | `src-tauri/src/hostmode/elevate.rs` |
| 按域名选择性启用（不走分片就透传） | ✅ 已实现 | `hostmode` 的 `matches()` |
| **Clash 联动**（`dns.hosts` + `DIRECT` 交给本机转发器） | ✅ 已实现 + Node 行为测试 | `core/src/adapters/cvr/script.rs` |
| 诊断（DNS / TCP / TLS 分层报告） | ⬜ 未实现 | — |
| ECH 能力识别与配置 | ⬜ 未实现（不伪造） | — |
| sing-box 原生 `tls_record_fragment` 适配 | ⬜ 未实现（当前只有 CVR 脚本） | `core/src/adapters/` |
| Mihomo 原生分片 | ⬜ 不支持，也不会伪造字段 | — |

## 3. 数据模型（现状）

Host 模式配置（`hostmode.json`）：

```json
{
  "enabled": true,
  "listen_port": 443,
  "domains": ["nrd.nie.163.com"],
  "fragment_mode": "record",
  "fragment_parts": 4,
  "dns_server": "223.5.5.5"
}
```

- `fragment_mode`：`record`（TLS Record 分片）/ `segment`（TCP 分段）/ `off`（纯转发）。
- 域名按后缀匹配：写 `nrd.nie.163.com` 覆盖它自己和所有子域。
- 只监听 `127.0.0.1`，不监听 `0.0.0.0`。

设计文档里的 `sni_profiles.json` / 多 profile / ECH 配置等尚未落地，
当前是"单份配置 + 一份域名列表"，够用但不做多方案管理。

## 4. 与"配置所有权"原则的关系

设计文档要求不自动改 Hosts，但本版本按用户要求增加了 **Host 模式**：

- 改 hosts 只在用户点「写入 Hosts」时发生，且**必须确认 + 提权**；
- 只增删带 `# >>> UURelayBridge` 标记的一段，其它行原样保留；
- 提供「移除 Hosts」；程序退出不自动清理（UI 上有明确警告）；
- 不改系统代理、防火墙、DNS 设置，不注册服务 / 计划任务。

## 5. 安全边界（实现层面）

- 不终止 TLS、不替换证书、不记录 TLS 会话内容。
- 只记录：域名、目标 IP、分片段数、字节数、错误。不记录任何凭据。
- 非 TLS 首包直接断开；ClientHello 解析失败直接断开（不做猜测性处理）。
- 每条连接独立线程，异常即释放；监听线程可停止。

## 6. 后续路线

1. 诊断页（DNS / TCP / TLS 三段报告 + 处理前后对比）。
2. sing-box 1.12+ 适配：`tls_record_fragment` 规则片段（按官方 schema 校验）。
3. CVR / Mihomo：只生成域名规则 + 独立转发器配置，明确标注"无原生分片"。
4. ECH：仅做能力识别（目标是否支持），不做伪造。
5. 多 profile（不同域名不同参数）。
