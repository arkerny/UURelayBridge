# 测试与验收记录

**机器**：Windows 11（10.0.26200），x86_64，Rust 1.93.0（stable-x86_64-pc-windows-msvc），
WebView2 157.0.4341.60　**日期**：2026-11-23

> 本记录只写实际执行过的结果。没执行过的项目明确标为「未验证」，不用静态阅读代替真机结论。

---

## 1. 构建

| 项目 | 命令 | 结果 |
|---|---|---|
| debug 构建 | `cargo build` | ✅ 成功，无 warning |
| release 构建 | `cargo build --release` | ✅ 成功（1m41s，LTO + strip + panic=abort） |
| 安装包 | `npx tauri build` | ✅ NSIS 1.28 MB + MSI 1.84 MB |
| 依赖 | crates.io 直连 | ✅ 436 个 crate |
| 图标 | `python tools/gen_icons.py` | ✅ `icon.ico`（16–256 多尺寸）、`icon.icns`（PNG 载荷）、托盘 on/off、macOS 模板图 |

产物：`dist/RelayBridge.exe`、`dist/RelayBridge_1.1.0_x64-setup.exe`、`dist/RelayBridge_1.1.0_x64_en-US.msi`。

## 2. 自动化测试

```
cargo test -p relaybridge-core
```

| 套件 | 数量 | 结果 |
|---|---|---|
| 单元测试（模型 / 规则语法 / 编译 / 校验 / 原子写 / 备份 / 设置 / 主题 / 路径穿越 / 日志） | 43 | ✅ |
| `cvr_script_semantics`（真实 Node 执行生成的 JS） | 4 | ✅ |
| doc-test | 0 | — |

脚本行为测试覆盖：

- ON 时注入的规则在最前，用户 3 条规则与其它字段原样保留，`MATCH` 仍在最后；
- 同一份配置连跑两次 `main()` 规则数不变（幂等）；
- 先 ON 再 OFF 后，规则列表精确恢复成用户原始 3 条；
- 空规则项目不改动 `rules`。

## 3. 应用内端到端自检（真实进程）

```
dist\RelayBridge.exe --selftest
```

debug 与 release 两个产物都跑过，输出一致：

```
[selftest] PASS save-project
[selftest] PASS authorization-gate 无授权时状态不变
[selftest] PASS enabled RB_ENABLED=true
[selftest] PASS injected-rule 两条规则都在
[selftest] PASS manifest-written manifest.json 存在
[selftest] PASS backup-created 备份 1 份
[selftest] PASS disabled RB_ENABLED=false
[selftest] PASS cleanup-list-kept OFF 版本保留规则清单用于清理
[selftest] PASS last-state-consistent ...
[selftest] PASS theme-persisted settings.json 写入 light
[selftest] PASS theme-applied-without-error 窗口主题调用无错误
[selftest] SELFTEST OK
```

同时生成的 `app.log` 含每一步的编译条数、备份文件、写入字节数与 sha256 前缀。

## 4. GUI 真机验证

| 检查 | 结果 | 证据 |
|---|---|---|
| 进程启动 | ✅ | `relaybridge.exe` 常驻 |
| 主窗口 | ✅ | `MainWindowTitle = RelayBridge` |
| 托盘初始化 | ✅ | 日志 `托盘/菜单栏已就绪` |
| **前端真正加载并连上后端** | ✅ | 日志 `[ui] 界面已就绪，前后端 IPC 正常`（由前端调用 `bootstrap` 触发的后端日志） |
| 启动阶段不写盘 | ✅ | 启动后数据目录保持空，直到用户操作 |
| 关闭窗口 → 隐藏 | ✅ | `WM_CLOSE` 后进程存活，日志 `主窗口已隐藏到托盘，程序继续常驻` |
| 便携/覆盖模式 | ✅ | `RELAYBRIDGE_DATA_DIR` / `RELAYBRIDGE_WEBVIEW_DIR` / `RELAYBRIDGE_LOG_FILE` 均生效 |

前端静态检查（`node --check ui/app.js` + `tools/check_ui_bindings.py`）：
语法通过；`app.js` 引用的 67 个 DOM id 在 `index.html` 中全部存在（0 缺失）。
另有 `tools/check_theme_vars.py` 校验深/浅两套主题的 CSS 变量完全对齐（51 / 51，无未定义、无冗余）。

| 界面变更（v1.2） | 结果 | 依据 |
|---|---|---|
| 日志面板滚动条固定在面板右边缘 | ✅ | `.log` 加 `min-height:0` + `scrollbar-gutter: stable`，`.logbar` 加 `overflow:hidden`（结构静态核对 + 运行截图外的人工确认项见第 7 节） |
| 浅色主题 | ✅ | 51 个变量两套齐全，切换 `html[data-theme]` |
| 主题跟随系统 | ✅ | `theme-boot.js` 首帧按 `prefers-color-scheme` 定调；`app.js` 监听系统变化，`system` 模式下实时跟随 |
| 主题持久化 + 原生标题栏 | ✅ | selftest `theme-persisted` / `theme-applied-without-error` |

## 5. 未验证项（如实标注）

| 项目 | 原因 | 建议 |
|---|---|---|
| macOS 全部运行时行为 | 本机是 Windows，无法交叉编译 / 运行 `.app` | 按 `docs/build-macos.md` 第 8 节清单在 Mac 上过一遍 |
| macOS 菜单栏图标观感 / 模板反色 | 同上 | 真机确认 |
| 托盘菜单逐项点击（开关、重新生成、设置、退出） | 需要真实交互，自动化不可靠 | 按第 7 节清单手工过 |
| 安装包安装 / 卸载、用户数据保留 | 未在本机执行安装（避免污染当前系统） | 建议在干净虚拟机验证一次 |
| 高 DPI / 多显示器窗口还原 | 需要人工观察 | 125% / 150% 各试一次 |

## 6. 安装包

| 产物 | 路径 | 说明 |
|---|---|---|
| NSIS | `dist/RelayBridge_1.1.0_x64-setup.exe` | 用户级安装（`installMode: currentUser`），无需管理员 |
| MSI | `dist/RelayBridge_1.1.0_x64_en-US.msi` | 同上 |
| 免安装 | `dist/RelayBridge.exe` | 双击即用 |

卸载不删除 `%APPDATA%\RelayBridge`（用户数据与生成结果保留）。

## 7. 手工验收清单（Windows）

- [ ] 双击 `dist\RelayBridge.exe`，托盘出现图标，主窗口打开
- [ ] 关闭窗口 → 托盘图标仍在；点击托盘图标 → 窗口回来
- [ ] 开关 ON → 确认框 → 「概览」出现生成时间、sha256、预览内容
- [ ] `%APPDATA%\RelayBridge\output\clash-verge-rev\relaybridge.js` 与预览一致
- [ ] 再点「重新生成」→ `backups/` 多一份，主文件更新
- [ ] 开关 OFF → 预览里 `var RB_ENABLED = false;`
- [ ] 「规则」页填非法域名（如 `bad domain`）→ 保存被拒并列出错误
- [ ] JSON 编辑器改坏语法 → 保存被拒
- [ ] 设置里关掉「关闭窗口时隐藏到托盘」→ 关窗口后进程仍存活（Windows 常驻策略）
- [ ] 托盘「退出」→ 进程消失，数据目录与输出文件都还在
- [ ] 重新启动 → 状态与规则恢复
- [ ] 把脚本贴进 Clash Verge Rev 扩展脚本并重载，规则出现在最前面
