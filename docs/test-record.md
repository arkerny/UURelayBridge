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

产物：`dist/RelayBridge.exe`、`dist/RelayBridge_1.5.0_x64-setup.exe`、`dist/RelayBridge_1.5.0_x64_en-US.msi`。

## 2. 自动化测试

```
cargo test -p relaybridge-core
```

| 套件 | 数量 | 结果 |
|---|---|---|
| 单元测试（模型 / 规则语法 / 编译 / 校验 / 原子写 / 备份 / 设置 / 主题 / 路径穿越 / 日志） | 43 | ✅ |
| `cvr_script_semantics`（真实 Node 执行生成的 JS） | 4 | ✅ |
| doc-test | 0 | — |

测试的临时目录写在 `target/test-tmp/`（`relaybridge_core::test_tmp_dir`），
不依赖 `%TEMP%`——受限环境（沙箱 / 临时目录只读）里也能跑。

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
| NSIS | `dist/RelayBridge_1.5.0_x64-setup.exe` | 用户级安装（`installMode: currentUser`），无需管理员 |
| MSI | `dist/RelayBridge_1.5.0_x64_en-US.msi` | 同上 |
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

---

## 8. 回归记录（1.5.0）：双击没反应

**现象**：用户双击 `dist\RelayBridge.exe` 后没有任何窗口，进程立即消失。
事件日志里有两条 `Application Error`：`Exception code: 0xc0000409`（abort），模块为 `RelayBridge.exe`。

**定位过程**

1. 事件日志 → `0xC0000409`（`FAST_FAIL_FATAL_APP_EXIT`，即 Rust `panic!` + `panic = "abort"`）。
2. release 是 GUI 子系统、没有控制台，panic 文本无处输出，所以表现为"静默不启动"。
3. 重定向 stderr 复现，拿到 panic：`Failed to setup app: ... Access is denied. (os error 5)`。
4. 加 boot 日志定位到 `WebviewWindowBuilder::build()`；再记录 `create_dir_all` 结果：

   ```
   app_local_data_dir = Ok("C:\\Users\\arkerny\\AppData\\Local\\com.relaybridge.desktop")
   create_dir_all(...\com.relaybridge.desktop) => Err(PermissionDenied, "Access is denied.")
   create_dir_all(...\Temp\RelayBridge-webview2) => Err(PermissionDenied, "Access is denied.")
   ```

5. 交叉验证：`cmd.exe` / `python.exe` / `node.exe` 都能在同一位置建目录，
   **同一个 exe 复制到 `%TEMP%` 或 `%LOCALAPPDATA%\Programs\` 就能正常启动**
   （包括通过 Task Scheduler 启动也一样失败，说明与父进程无关，取决于程序所在目录）。

**根因（两层）**

- 产品缺陷：Tauri 把 `%LOCALAPPDATA%\<identifier>` 当 WebView2 用户数据目录，而 WebView2
  **不会**创建它；目录不存在时直接返回 `Access denied (0x5)`。首次启动必然踩中。
- 环境限制：本机把 `Desktop\hdu-uu-fix\` 这个目录里的进程限制为只能写自身目录，
  于是"先建目录"的兜底也会失败 —— 这属于运行位置问题，不是程序逻辑问题。

**修复**

1. 创建窗口前先显式 `create_dir_all` 并 `data_directory(...)`，顺序为：
   应用本地数据目录 → `%APPDATA%\RelayBridge\webview2` → 系统临时目录。
2. 窗口创建失败不再走 `panic`：写启动错误日志（`RELAYBRIDGE_LOG_FILE` 或 `%TEMP%`），
   弹 `MessageBox` 说明原因与处理建议，然后以退出码 1 结束。

**验证**

| 场景 | 结果 |
|---|---|
| 全新状态（删除 `%LOCALAPPDATA%\com.relaybridge.desktop`）+ 可写目录运行 | ✅ 窗口正常，自动创建 `EBWebView` |
| 从受限目录运行 | ✅ 不再静默崩溃，弹出可读的错误对话框并写日志 |
| 1.5.0 release 冒烟 | ✅ `dist\RelayBridge.exe` 在 `Desktop\relaybridge-standalone\` 下正常运行 |

**给用户的结论**：程序本身没问题，但这个工作区目录受到写入限制；
把 `dist\` 复制到工作区之外的普通目录（或把安装包复制出去再安装）即可。

---

## 9. 回归记录（1.5.0）：日志面板位置错乱

**现象**：界面里内容区被压成一条（约 44px），日志框被撑到 415px 并停在页面中部，
窗口底部空出约 220px 的空白 —— 也就是"日志框位置不对"。

**定位**：在 WebView2 页面里执行 `getBoundingClientRect()` 把真实布局回写到日志
（`RELAYBRIDGE_DUMP_LAYOUT=1`），拿到：

```
content  y=101 h=44
logbar   y=145 h=415
appKids  [topbar 56, banner 0(display:none), tabs 45, content 44, logbar 415]
```

**根因**：`.app` 用的是 `grid-template-rows: 56 auto 44 1fr 220`（5 行对应 5 个元素）。
提示横幅 `.subtle-banner` 平时是 `display:none` —— 它不再是 grid item，
后面的元素于是整体**上移一行**：`content` 落进了 44px 的标签行，
`logbar` 抢到 `1fr`，最后那行 220px 空着。

**修复**：`.app` 改为纵向 flex（`topbar` / `banner` / `tabs` 固定，`content: 1fr`，
`logbar: 220px`），日志面板内部同样改为 flex；横幅显示与否都不再影响其他元素位置。

**验证**（1.5.0 release，1080×780 视口）：

| 指标 | 结果 |
|---|---|
| 内容区 | y=101, h=459（占满剩余空间） |
| 日志面板 | y=560, h=220（贴窗口底部，无空白残留） |
| 日志滚动区 | h=185，`scrollHeight 2772 > clientHeight 185` 可滚动 |
| 滚动条 | `offsetWidth - clientWidth = 10`，位于面板右边缘（log.right = 1080） |
| 主题 | `data-theme = light`（跟随系统） |

---

## 10. 1.5.0：改名 / 复制模式 / Host 模式

**改名**：crate `uurelaybridge(-core)`、产品名 `UURelayBridge`、identifier
`com.uurelaybridge.desktop`、数据目录 `%APPDATA%\UURelayBridge`、窗口标题同步；
环境变量新增 `UURELAYBRIDGE_*` 并兼容旧的 `RELAYBRIDGE_*`。

**改为"复制即用"**：开关与保存规则不再写文件，只在界面里生成内容；
`导出到文件` 才落盘（旧版自动备份）。托盘菜单相应改为「导出配置到文件」。

**Host 模式**（本地 TLS 转发 + 分片 + hosts）：

| 检查 | 结果 |
|---|---|
| TLS ClientHello 解析 / Record 分片 / TCP 分段 | 单元测试通过（`core/src/sni/tls.rs`） |
| DNS 查询（绕过 hosts） | 单元测试 + 实跑（`example.com` 在 1ms 内返回） |
| hosts 改写（幂等、保留其它行、可移除） | 单元测试通过（串行执行避免环境变量竞争） |
| 端到端：配置域名走分片 | ✅ `tools/host_selftest.py`：`example.com` 经本地转发完成 TLS1.3 握手，日志为「TLS Record 分片 4 段，共 517 字节」 |
| 端到端：未配置域名直接透传 | ✅ `www.iana.org` 同样握手成功（未分片） |
| 转发开销 | 直连基线 4333ms，经转发 4338ms（本机网络本身就要 4.3s） |
| 日志栏可拖拽 | ✅ 探针模拟拖拽：`--logbar-h` 220px → 340px，实际高度同步 |
| Host 页存在 | ✅ 5 个表单字段 |

**未验证**：UAC 提权的真实交互（需要人点确认）、macOS 的 root 监听与 osascript 提权、
写入真实系统 hosts（仅用 `UURELAYBRIDGE_HOSTS_FILE` 指向临时文件做了单测）。

**已知限制**：Host 模式停掉转发或退出程序后，被写入 hosts 的域名会不可用（界面已提示）；
443 在 macOS 上需要 root。

---

## 11. 1.5.0：SNI 配置同时作用于 Clash 模式

**做法**：Host 模式里的域名/分片参数是唯一一份，生成 Clash 脚本时同步使用：

- `dns.hosts[<域名>] = "127.0.0.1"`（只改这几个键，用户其它 hosts 原样保留）；
- 生成 `DOMAIN-SUFFIX,<域名>,DIRECT` 规则并排在最前面，让内核把流量交给本机转发器；
- OFF 版本会把自己写进 `dns.hosts` 的 `127.0.0.1` 删掉。

**测试**（Node 执行真实脚本，`core/tests/cvr_script_semantics.rs`）：

| 用例 | 结果 |
|---|---|
| `sni_link_points_domains_to_localhost_and_adds_direct_rule` | ✅ hosts 指向 127.0.0.1、用户原有 hosts 保留、DIRECT 规则在最前、`MATCH` 仍在最后 |
| `sni_link_is_idempotent` | ✅ 连跑 3 次，DIRECT 规则仍只有 1 条 |
| `off_removes_sni_hosts_entry` | ✅ OFF 后自己写的 hosts 条目被删除，用户条目保留 |

**应用内验证**：预置 `hostmode.json`（域名 `nrd.nie.163.com`、`link_clash=true`）后启动，
布局探针读到预览文本包含 `RB_SNI_DOMAINS` 与 `hosts[d]`（长度 2878），
Host 页 6 个表单字段、日志栏拖拽 340→460px 均正常。

**已知语义**：同一域名若另有代理规则，会被 SNI 的 DIRECT 规则覆盖（分片优先），
脚本头部与 README 均写明；不想联动可取消 Host 页的勾选。

---

## 12. 1.5.0：模式选单 / DoH / 界面精简

**模式选单**：顶部「Clash 规则 / 本机转发」二选一，存进 `settings.json` 的 `mode`；
切换后标签页跟着变（clash：概览/规则/设置；host：本机转发/设置）。

| 检查 | 结果 |
|---|---|
| clash 模式 | mode=clash，标签 = 概览/规则/设置，复制按钮可见 |
| host 模式 | mode=host，标签 = 本机转发/设置，注入开关与复制按钮隐藏 |
| 默认值 / 非法值 | 默认 `clash`；非法值回落 `clash`（单测 `mode_defaults_to_clash_and_rejects_unknown`） |

**DoH**：`dns_server` 支持 `https://...`（RFC 8484 POST）。Windows 走系统 WinHTTP，
其它平台走 curl；选择 DoH 时不再用明文 DNS 兜底。

| 检查 | 结果 |
|---|---|
| URL 解析（https/端口/默认路径/非法） | ✅ 4 个单测 |
| 端到端（本地假 DoH 服务器） | ✅ `tools/fake_doh.py` + `cargo run --example doh_probe`：`test.local -> 203.0.113.7` |
| 真实公网 DoH | ⚠ 本机 Schannel 在受限环境下不可用，未做公网联调；用户环境应正常 |

**界面精简**：去掉大段说明与多余入口，概览只留「复制 + 导出 + 打开目录」，
Host 页改为「域名 / 端口 / 分片 / DNS」四项 + 状态侧栏；DoH 预设用 datalist 提供。
顺带修复：`settings.json` 带 UTF-8 BOM 时解析失败（记事本保存过会遇到），现在读取时自动去 BOM。

**测试**：`cargo test -p uurelaybridge-core` → 56 单元 + 7 个 Node 行为测试全绿。
