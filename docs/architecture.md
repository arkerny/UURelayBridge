# 架构与数据流

## 1. 分层

```
┌──────────────────────────── UI（WebView，无构建步骤） ────────────────────────────┐
│  index.html / app.js / style.css    （只做展示 + 收集"确认"，不含业务规则）        │
└───────────────────────────────────┬──────────────────────────────────────────────┘
                                    │ invoke / event（Tauri IPC）
┌───────────────────────────────────▼──────────────────────────────────────────────┐
│  src-tauri/src/commands.rs   命令层：授权门控 + 串行化写 + 日志 + 事件广播         │
│  src-tauri/src/tray/         托盘 / 菜单栏（Windows 托盘、macOS 菜单栏同一套代码） │
│  src-tauri/src/state/        运行期状态（RwLock 内存态，只在用户授权后落盘）       │
│  src-tauri/src/platform/     windows.rs / macos.rs（只放"打开目录"+图标选择）      │
└───────────────────────────────────┬──────────────────────────────────────────────┘
                                    │ 函数调用（无 IO 依赖注入）
┌───────────────────────────────────▼──────────────────────────────────────────────┐
│  core/（relaybridge-core，纯 Rust，不依赖 Tauri）                                 │
│    model.rs      统一模型 Project / Rule / Match / Action                         │
│    rules.rs      域名、IP/CIDR 的规范化和语法校验（纯函数）                       │
│    compiler.rs   模型 → Clash 规则串（保序、去重、跳过非法项）                    │
│    validator.rs  生成前的错误/警告清单                                            │
│    adapters/     Adapter trait + cvr（实现）+ mihomo / singbox（占位）            │
│    exporter/     原子写 + 备份 + manifest + 导出编排                              │
│    paths.rs      数据目录结构（唯一路径来源）                                     │
│    settings.rs   settings.json / project.json 读写与损坏恢复                      │
└──────────────────────────────────────────────────────────────────────────────────┘
```

依赖方向单一：`ui → commands → core`，**core 不认识 Tauri**。
新增内核 = 加一个 adapter，不改 core、不改 commands。

## 2. 一次"切到 ON"的完整链路

```
用户点开关
  └─ UI: 弹出确认框（说明会写哪个文件、不会碰 Clash Verge）
      └─ invoke("set_enabled", { approved: true, enabled: true })
          └─ commands::set_enabled
              ├─ 校验 approved（false 直接返回 needs_approval，前端绕过也无效）
              ├─ lock write_lock（串行化，避免并发覆盖）
              ├─ 内存 settings.enabled = true
              ├─ generate_locked()
              │    ├─ validator::validate(project)      → 有 error 就中止，不写任何文件
              │    ├─ compiler::compile(project)        → Vec<规则串>
              │    ├─ adapter.render(RenderInput)       → relaybridge.js 文本
              │    ├─ backup::backup_file(旧文件)      → backups/relaybridge-*.js
              │    ├─ writer::atomic_write_bytes()      → 同目录临时文件 + fsync + rename
              │    ├─ manifest::write()                 → 含 sha256 / 时间 / 规则数
              │    └─ backup::prune(keep=N)             → 只删自己的旧备份
              ├─ 更新 last 结果（成功/失败都记录）
              ├─ 持久化 settings.json（enabled + 时间戳 + 是否成功）
              ├─ tray::refresh()  → 勾选状态 / 菜单文案 / 托盘图标 / tooltip
              └─ emit("rb://state", snapshot)  → UI 全量刷新
          └─ 返回 Snapshot（UI 用返回值和事件双保险）
```

失败时：**上一次成功生成的文件保持完好**（原子替换保证），`last.ok=false`，
托盘显示"⚠ 上次生成失败"，界面出现一致性横幅，用户可重试。

## 3. 授权模型（硬约束）

- 前端：每个写操作前必须走 `confirmDialog`，用户点「确认」才调用。
- 后端：命令签名带 `approved: bool`，为 `false` 直接返回 `needs_approval` 错误。
- 托盘：用户点击菜单项本身就是授权动作（`apply_enabled(..., true, "tray")`）。
- 启动：只读。`settings.json` / `project.json` 缺失或损坏时**不自动修复、不落盘**，
  只在界面提示；等用户点保存时才把损坏文件备份到 `backups/` 再写新的。

## 4. 路径与跨平台

| 项 | Windows | macOS |
|---|---|---|
| 数据目录 | `%APPDATA%\RelayBridge` | `~/Library/Application Support/RelayBridge` |
| 路径获取 | `app.path().data_dir()`（Tauri Path API） | 同上 |
| 打开目录 | `explorer.exe <dir>` | `open <dir>` |
| 托盘 | 系统托盘 + 右键菜单，左键开窗 | 菜单栏图标 + 下拉菜单，模板图标 |
| 关闭窗口 | 拦截 `CloseRequested` → `hide()` | 同左 |
| 退出 | 托盘「退出」→ `app.exit(0)` | 菜单「Quit RelayBridge」/ Cmd+Q |

业务代码里没有 `C:\`、`/Users/`、`/home/` 之类的硬编码；`paths.rs` 是唯一路径来源，
`output/<target>` 还会对 target 名做字符过滤，防止路径穿越。

## 5. 扩展脚本的幂等设计

`relaybridge.js` 里的 `main(config)`：

```js
var RB_RULES = [ ... ];                    // 本次应注入的规则（OFF 时也保留，用于清理）
var kept = existing.filter(r => !ours.includes(r));   // 先摘掉上次注入的"完全相同"的字符串
config.rules = RB_ENABLED ? RB_RULES.concat(kept) : kept;
```

- 只按**完全相同的规则串**去重 → 重复加载不会叠加，也不会误删用户自己的同名规则（除非字符串一模一样）。
- 永远插在最前面 → 优先于 `MATCH` 等终结规则。
- 不碰 `proxy-groups` / `proxies` / 任何其他字段；仅在策略组缺失时 `console.warn`。

## 6. 测试策略

| 层 | 手段 | 说明 |
|---|---|---|
| core 单元 | `cargo test -p relaybridge-core` | 模型/校验/编译/导出/备份/设置/路径穿越 |
| 脚本行为 | `core/tests/cvr_script_semantics.rs` | 生成真实 JS，交给 **Node** 跑，断言注入顺序、幂等、OFF 清理、空规则 |
| 应用整链 | `relaybridge --selftest` | 真实进程内跑授权→保存→生成→备份→OFF，输出 PASS/FAIL |
| GUI | 手工 + 本机实测记录 | 沙箱环境无法创建 WebView2，详见 test-record |
