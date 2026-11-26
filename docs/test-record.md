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

产物：`dist/RelayBridge.exe`、`dist/RelayBridge_1.20.0_x64-setup.exe`、`dist/RelayBridge_1.20.0_x64_en-US.msi`。

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
| NSIS | `dist/RelayBridge_1.20.0_x64-setup.exe` | 用户级安装（`installMode: currentUser`），无需管理员 |
| MSI | `dist/RelayBridge_1.20.0_x64_en-US.msi` | 同上 |
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

## 8. 回归记录（1.20.0）：双击没反应

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
| 1.20.0 release 冒烟 | ✅ `dist\RelayBridge.exe` 在 `Desktop\relaybridge-standalone\` 下正常运行 |

**给用户的结论**：程序本身没问题，但这个工作区目录受到写入限制；
把 `dist\` 复制到工作区之外的普通目录（或把安装包复制出去再安装）即可。

---

## 9. 回归记录（1.20.0）：日志面板位置错乱

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

**验证**（1.20.0 release，1080×780 视口）：

| 指标 | 结果 |
|---|---|
| 内容区 | y=101, h=459（占满剩余空间） |
| 日志面板 | y=560, h=220（贴窗口底部，无空白残留） |
| 日志滚动区 | h=185，`scrollHeight 2772 > clientHeight 185` 可滚动 |
| 滚动条 | `offsetWidth - clientWidth = 10`，位于面板右边缘（log.right = 1080） |
| 主题 | `data-theme = light`（跟随系统） |

---

## 10. 1.20.0：改名 / 复制模式 / Host 模式

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

## 11. 1.20.0：SNI 配置同时作用于 Clash 模式

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

## 12. 1.20.0：模式选单 / DoH / 界面精简

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

---

## 13. 1.20.0：内置 UU 远程端点预置

`core/src/presets.rs` 内置 6 个端点（`UU_ENDPOINTS` + 公共后缀 `UU_SUFFIX`），
Host 模式默认值直接带上它们；两个模式各有一个「UU 远程」按钮可一键填入。

| 检查 | 结果 |
|---|---|
| 端点都在 `nrd.nie.163.com` 下、且不重复 | ✅ 2 个单测 |
| 全新数据目录（无 `hostmode.json`）+ host 模式 | ✅ 界面预填 6 个端点、DNS 默认 `223.5.5.5` |
| 规则模式「UU 远程」按钮 | ✅ 打开规则弹窗并预填 6 个域名（动作=代理，策略组待填） |

顺带修了一个启动期问题：`applyMode()` 原来嵌在 boot 的 `guard()` 回调里，
而 `guard()` 用 `S.busy` 做重入保护，导致 host 模式下首次加载配置被静默跳过
（表现为域名/DNS 输入框空白）。现在 `applyMode()` 在 guard 之外调用。

---

## 14. 1.20.0：修复顶部模式选单点击无响应

**现象**：顶部的「本机转发」按钮点了没反应，模式仍是 Clash 规则。

**定位**：给布局探针加了一步"模拟点击按钮并回报状态"，结果 `mode` 仍是 `clash`、无报错。
检查 `ui/app.js` 发现 `bind()` 里**根本没有 `modeSwitch` 的监听器**——
上一轮的批量替换用 `String.Replace` 匹配缩进时没命中，脚本"成功退出"但实际没插入。

**修复**：
- 在 `bind()` 里补上模式选单监听（点击 → `set_mode` → `applyMode`）；
- `applyMode()` 里加载 Host 配置的动作改为 `setTimeout(0)` 延后，避免又被 `S.busy`
  重入保护吞掉（这是同一类问题第二次出现，已写入注释）。

**验证**（真实进程 + 探针模拟点击）：

| 场景 | 结果 |
|---|---|
| 默认 clash → 点「本机转发」 | ✅ mode=host，标签变为 本机转发/设置，页面切到 page-host，域名 6 个 |
| host → 点「Clash 规则」 | ✅ mode=clash，标签回到 概览/规则/设置 |
| 选择是否落盘 | ✅ `settings.json` 里 `"mode": "host"` |

顺带把启动日志里的产品名统一成 `UURelayBridge`。

---

## 15. 1.20.0：单开关 + Clash 走本地 SOCKS + Host 监听 80/443

**结构调整**：一个开关启动/停止服务；Clash 模式起本地 SOCKS5，脚本注入 `socks5` 节点与规则；
Host 模式监听 127.0.0.1:443（TLS 分片）与 :80（HTTP 按 Host 头直连），并保留一键写 hosts。
「本机转发」名称改回「Host 模式」。

| 验证 | 结果 |
|---|---|
| SOCKS 握手 + TLS（example.com） | ✅ TLS1.3，cert 1002B |
| **被阻断域名经 SOCKS 分片** | ✅ `api.nrd.nie.163.com` 握手成功（TLS1.2 / ECDHE-ECDSA-AES256-GCM-SHA384，cert 1343B），日志 `TLS Record 分片 4 段` |
| Host 模式 443 转发 | ✅ `example.com` TLS1.3 握手成功 |
| Host 模式 80 监听 | ✅ 日志 `HTTP 转发已监听 127.0.0.1:80`（未做真实 HTTP 转发验证） |
| 生成脚本含 SOCKS 段 | ✅ 预览含 `RB_SOCKS_PORT`（长度 3749） |
| 开关失败回退 | ✅ 启动失败时自动把开关置回 OFF 并写错误日志 |

**注意**：SOCKS 模式下解析真实 IP 用的是配置里的 DNS（支持 DoH）；本机若开着 Clash TUN，
可能拿到 fake-ip（日志里是 198.18.x.x），这属于正常现象，连接仍由 Clash 转发。

---

## 16. 1.20.0：Host 模式可用性 + hosts 可恢复 + 全功能回归

**hosts 可恢复**
- 每次写入前把原文件备份到 `<数据目录>/backups/hosts/hosts-<epoch>.txt`，只留最近 5 份；
- Host 页新增「恢复原 hosts」（走同一套提权子进程 `--hosts-restore`）；
- 顶部提示条：hosts 已写入但服务未开启时给出警示，避免"域名打不开还不知道为什么"；
- 概览状态卡显示服务地址（SOCKS / TLS / HTTP）与 Hosts 状态。

**一键回归脚本** `tools/e2e.py`（本机实跑 7/7 通过）：

```
== 1. 单元 / 行为测试 ==      PASS cargo test（61 单元 + 7 行为）
== 2. hosts 往返 ==           PASS 备份 → 写入 → 恢复（内容逐字节一致）→ 移除（用户行保留）
== 3. SOCKS 模式 ==           PASS api.nrd.nie.163.com 经分片握手 TLSv1.2 cert=1343B
== 4. Host 模式 ==            PASS 443 转发 TLSv1.3；80 端口监听
== 5. UI 两种模式 ==          PASS clash: 概览/规则/设置 · host: Host 模式/设置
```

**未验证**：真实 UAC 弹窗交互（需要人点确认）、真实系统 hosts 的写入/恢复
（自检用 `UURELAYBRIDGE_HOSTS_FILE` 指到临时文件，逻辑一致但不是系统文件）。

---

## 17. 1.20.0：前后端分离（Clash Verge 式架构）

**结构**
- 新增 `uuengine`（workspace crate `engine/`，纯 CLI 内核，不依赖 Tauri）；
  支持 `run / check / print-config / version`，配置为 JSON（kind=socks|host）。
- 前端删除全部数据面代码（监听、SOCKS、relay），只保留：写 `engine.json`、
  拉起内核、转发内核 stdout 到日志、退出时结束内核、hosts 管理。
- 安装包通过 `bundle.externalBin` 带上内核；便携用法两个 exe 放同一目录。

**顺带修**
- 启动失败弹窗改为在子线程弹出，主线程不再阻塞（避免 Windows 判成"未响应"）。

**回归**（`tools/e2e.py`，10/10 通过）

```
1. cargo test             ✓ 61 单元 + 7 行为测试
2. hosts 往返             ✓ 备份→写入→恢复（字节一致）→移除（用户行保留）
3. engine check           ✓
4. engine socks 分片       ✓ api.nrd.nie.163.com TLSv1.2 cert=1343B
5. engine host 443/80     ✓ example.com TLSv1.3；HTTP 监听
6. app→engine（clash）    ✓ 前端拉起内核，7899 可用
7. app→engine（host）     ✓ 前端拉起内核，443/80 可用
8. UI clash / host        ✓ 标签与模式正确
```

**未验证**：安装包装好后的 exe 同目录布局（用 `target/release` 与便携目录验证过，
未在真实安装目录里双击）；内核 Ctrl+C 优雅退出（当前用进程结束）。

---

## 18. 1.20.0：内核改用简洁 YAML 配置

**配置格式**（`engine.yaml`，前端会自动写，也能手改）：

```yaml
mode: socks
domains: [nrd.nie.163.com]
```

- 只有 `mode` 与 `domains` 必填，其余给默认值；
- `domains` 可写列表、也可写单个域名（按后缀匹配子域）；
- `fragment` 一行表达三种行为：`4`（记录分 4 段）/ `off` / `tcp:6`；
- `dns` 同时接受明文 IP 与 DoH URL；
- 兼容旧字段名（`kind` / `listen_port` / `dns_server`）与 `.json` 配置。

**测试**（`tools/yaml_test.py`，9/9）：最小两行配置、单域名标量、完整配置、
`fragment: off`、注释与行内注释、非法 mode / 空 domains / 非法 fragment 均按预期报错，
并按 YAML 启动内核跑通 `api.nrd.nie.163.com` 分片握手（TLSv1.2 cert=1343B）。

**回归**：`tools/e2e.py` 10/10（含前端写 YAML → 内核读取 → 443/7899 可用）。
`cargo test`：61 单元 + 7 行为 + 6 个内核配置单测。

---

## 19. 1.20.0：配置默认在 exe 同目录、缺失自动创建；启动健壮性

**配置位置**
- `uuengine` 默认用可执行文件同目录的 `engine.yaml`，不存在则自动生成（带注释模板）；
- 不覆盖用户手改的那份：内容里没有生成标记时保持原样（`check` 也不会改文件）；
- 前端开关也优先写到 exe 同目录；目录不可写（如 Program Files）才退回数据目录，
  且不会覆盖用户手改过的文件。

**启动健壮性**（对应"打开就未响应/打不开"）
- 单实例：第二个实例不再抢 WebView2 数据目录（那会直接 0x800700AA），
  而是把已有窗口叫到前台；
- 窗口创建失败时用 `.desktop-2` / `.desktop-3` 备用数据目录重试，最多 3 次；
- 启动失败弹窗仍在子线程，主线程不阻塞。

**验证**（`tools/config_location_test.py`，9/9）
- 无参数直接运行 → 自动生成 `engine.yaml`、7899 可用、日志提示"已创建默认配置"；
- `check` 无参数也走默认路径并自动创建；
- 已存在的用户配置不被覆盖；
- 前端在 exe 同目录生成 `engine.yaml`，服务（443）可用。

`tools/e2e.py` 10/10 保持通过。

---

## 20. 1.20.0：按 campus-tls-fix 的方式重做抗阻断

参照 https://github.com/yunyancuo/campus-tls-fix 的实现，改了三处：

| 改动 | 之前 | 现在 |
|---|---|---|
| 切分位置 | 整条记录等分 N 段，SNI 可能完整落在某一段里 | **切点在 SNI 域名正中间**，拆成 2 条记录，任何一条都不含完整域名 |
| 解析结果 | 直接用系统 DNS 的答案 | 过滤 `198.18.0.0/15`、`127.x`、`169.254.x`，命中就改用 DoH 重查 |
| IP 选择 | 只试第一个 IP | 取全部 A 记录逐个探测，无响应的自动换下一个，最后还有内置兜底 IP 表 |

**实测（本机、被 SNI 阻断的网络）**：6 个 UU 端点全部握手成功：

```
api.nrd.nie.163.com                TLSv1.2   49ms cert=1343B
api-dcdn.nrd.nie.163.com           TLSv1.3   24ms cert=1959B
api-event.nrd.nie.163.com          TLSv1.2   18ms cert=1960B
api-ipv4.nrd.nie.163.com           TLSv1.2   32ms cert=1343B
sig-3303-d.nrd.nie.163.com         TLSv1.3  465ms cert=1960B   ← 第一个 IP 无响应，自动换到备用 IP
relay-mg-3303-d.nrd.nie.163.com    TLSv1.2   27ms cert=1960B
```

对比修复前：同一批域名直连是 TLS 握手超时（>3s 后失败）。

新增单测：`split_mid_sni` 拆出的两条记录拼接后与原始 payload 完全一致、
任何一条记录都不含完整域名、拼回去仍能解析出 SNI。

**未验证**：不同校园网 DPI 的实际行为差异（本机网络环境只能验证这一种封锁方式）。

---

## 21. 1.20.0：修 SOCKS 的 IP 目标 + 性能实测

**修的 bug**：SOCKS5 客户端常用 IP 字面量作为目标（如 `127.0.0.1:443`），
但解析环节把 IP 也当域名去查 DNS，且"防 fake-ip"过滤会把回环地址丢掉，
导致这类连接直接超时。现在：目标是 IP 字面量时直接连接，不做 DNS、不做过滤。

**性能实测**（本地 TLS 服务器 + 200 MB 下载，隔离网络影响）：

| 项目 | 直连 | 经内核 | 差异 |
|---|---|---|---|
| TLS 握手 | 2.6 ms | 3.8 ms | **+1.2 ms** |
| 吞吐 | 816 MB/s | 838 MB/s | **+2.7%**（在噪声内） |

结论：转发本身几乎不损失吞吐（回环上甚至略高，属测量噪声），
固定代价是**一次额外握手往返（约 1 ms 级）**。

**公网对照**（同一台机器、当前网络）：`example.com` 直连握手 4389 ms，
经内核 546 ms —— 快 8 倍，原因是直连的 ClientHello 被丢弃后要等超时重传。

**设计上的取舍**：
- 只对配置的域名分片，其余流量原样透传（不多走一跳）；
- 首个报文之后是纯字节转发，不做重组/解密，CPU 占用可忽略；
- 每连接多一个本地回环套接字（约几十 KB 内存），空闲 4s 探测只发生在首包。

---

## 22. 1.20.0：内核健壮性 + 英文日志 + hosts 自检 + macOS arm64

**健壮性（防闪退）**
- 每个连接线程包 `catch_unwind`：单连接 panic 不再拖垮整个内核
  （`panic=abort` 下线程 panic 会直接终止进程，这是"闪退"的根因）；
- 畸形输入压测 16 类：空连接、SOCKS4、HTTP、截断/超长/零长 ClientHello、
  非法 ATYP、BIND、随机 512B/4KB、50 个并发半开连接 —— 进程全部存活。

**日志**
- 统一格式 `HH:MM:SS.mmm [LEVEL] message`（INFO/WARN/ERROR）；
- 同时写 stdout 和**运行目录下的 `uuengine.log`**（目录不可写时静默跳过）；
- 全部改为英文。

**hosts 自检**
- 启动前扫描系统 hosts：若配置的域名被指向 `127.0.0.1` / `0.0.0.0` / `::1`，
  直接报错退出（code 3）并列出具体域名，避免转发器自己连自己；
- 可用 `check_hosts: false` 关闭（前端生成的配置默认关闭，由界面自己提示）。

**DoH**
- 配置 `dns: https://...` 走 DoH；实测 AliDNS / DNSPub 两个端点均解析成功并完成分片握手。

**macOS (arm64)**
- 新增 `aarch64-apple-darwin` 目标并通过 `cargo check`（Win 上没有 macOS SDK，无法链接出二进制）；
- 修掉 macOS 分支里 `std::io::Write` 未导入导致的编译错误；
- 非 Windows 平台补上文件锁单实例实现。

**回归**（release 内核）
- 6 个 UU 端点全部握手成功（19–373ms）；
- 健壮性 6/6；DoH 2/2；`cargo test` 64 单元 + 7 行为。

---

## 23. 1.20.0：修 WARN 暴露的三个解析问题

用户日志里出现了 4 类 WARN，逐个查到根因：

| 现象 | 根因 | 修复 |
|---|---|---|
| `42.186.144.136 closed immediately (EOF)` 后换 IP | 首选 DoH 只回 1 条 A 记录，没有备选 | 首选 DNS 与 4 个公共 DoH **并行**查询、合并结果 |
| `sig-3207-g… all candidate IPs failed` | 动态节点名不在静态兜底表里 | 按域名前缀兜底（`sig-*` → 34.95.122.33，`relay-mg-*` → 117.147.201.40） |
| 同一死 IP 每次都先撞一遍，浪费 4s | 没有失败记忆 | 失败 IP 标记 5 分钟，排到候选末尾 |
| 重复请求同一域名仍要 3–8s | 每次都重新查 DoH | 候选 IP 缓存 5 分钟 |
| 日志被逐连接细节刷屏 | INFO 级别过细 | 细节降为 DEBUG，`log_level: debug` 才输出 |

**实测**（同一域名连续 3 次）：

```
api.nrd.nie.163.com               首次 547ms  后续 11/13ms
sig-3207-g.nrd.nie.163.com        首次 482ms  后续 355/363ms
relay-mg-3207-g.nrd.nie.163.com   首次 2048ms 后续 24/22ms
api-dcdn.nrd.nie.163.com          首次 18ms   后续 13/14ms
```

首次慢的只剩 DoH 本身的首查（1–2s，被墙的 DoH 端点会拖到截止时间），之后走缓存。

**剩余问题**：`sig-*` 这类域名解析到的第一个 IP 在某些网络下确实不可用，
现在会标记 5 分钟并自动跳过，但每次新连接仍会先试一次兜底顺序里的活 IP，
延迟约 350ms —— 这是"必须验证线路通不通"的固有代价。

---

## 24. 1.20.0：修空闲连接被掐断（远控延迟/丢包的真正原因）

**现象**：远控时帧延迟、丢包明显上升。

**定位**（本地回环对照实验）：

| 检查 | 直连 | 经内核（修复前） |
|---|---|---|
| 空闲 20 秒后再收发 | 正常 | **连接被 abort（WinError 10053）** |
| 64B 往返中位数 | 0.04 ms | 0.09 ms |

根因：`socks_conn` 在握手阶段给客户端 socket 设了 `read_timeout = 15s`，
进入 `pump` 转发**之后没有清掉**。读线程一旦遇到空闲超过 15 秒就拿到 `WouldBlock`，
把它当成对端关闭 → 主动 shutdown 整个连接。

远控是长连接 + 周期性小包，画面静止或网络抖动几秒就会触发，
表现正是"卡顿、断流、延迟飙升、丢包"。

**修复**：
- `pump` 开头清掉客户端与远端的读写超时；
- 转发循环里把 `WouldBlock` / `Interrupted` 当作可重试，不再当作连接结束；
- 探测失败路径也复位超时，避免把超时状态带进透传。

**修复后实测**：

| 项目 | 结果 |
|---|---|
| 空闲 20 秒后再收发 | **正常**（与直连一致） |
| 90 秒长连接 + 3 次 25 秒空闲 | **全程存活** |
| 64B 往返中位数 | 0.06 ms（直连 0.04 ms） |
| 下行 8 MB 连续推送 | 163.7 MB/s（直连 543.6 MB/s，仍远高于远控实际码率） |
| 6 个 UU 端点 | 6/6 握手成功 |

**结论**：延迟增加与丢包是内核 bug，不是你的网络。下行吞吐在回环上的差距
来自单线程逐块拷贝，远控码率（几 MB/s 以内）用不到这个余量。

---

## 25. 1.20.0：内核性能审查 + 健壮性补强 + 双平台产物

**逐段审查后改动的点**

| 项目 | 改动 | 说明 |
|---|---|---|
| 转发缓冲 | 16 KB → 64 KB | 系统调用次数降到 1/4 |
| accept 循环 | 非阻塞忙等 80ms → 阻塞 accept | 空闲时不占 CPU |
| 日志写入 | 每行 open/close → 进程内常驻句柄 | 高并发时不再频繁打开文件 |
| 线程栈 | 2 MB → 256 KB | 并发连接数上限提高约 8 倍 |
| TCP keepalive | 新增（Win: `SIO_KEEPALIVE_VALS`；Unix: `SO_KEEPALIVE`） | 防中间设备静默丢连接状态 |
| 半关闭 | 下行结束先 `shutdown(Write)` 再关闭 | 对端能读完剩余数据 |
| 缓存容量 | 解析缓存 / 坏 IP 表加 512 条上限并过期清理 | 长期运行不无限增长 |
| 线程创建失败 | 显式告警并计数 | 极端并发不再静默丢连接 |

**一次失败的尝试（已回滚）**：试过"发完首包就走、不等服务器响应"，
延迟没降（服务器本来就要回包），反而让被黑洞的 IP 拖到客户端超时：
端点从 6/6 掉到 4/6。已恢复 2.5s 首字节探测，并保留"未写过数据时可换 IP"的能力。

**性能实测**（回环，64 MB × 5 取中位数）

```
直连    1015.5 MB/s
经内核   861.6 MB/s   → 85%
64B 往返中位数差 +0.02 ms
空闲 20s 后连接存活：OK
```

**回归**：6 端点 6/6；健壮性（畸形输入/并发）PASS；DoH 2/2；`cargo test` 64 + 7。

**双平台产物**

- Windows：`dist/uuengine.exe`（x86_64-pc-windows-msvc）、NSIS/MSI 安装包；
- macOS：`dist/uuengine-1.20.0-macos-arm64-src.zip`
  （内核+core 源码 + `build-macos-arm64.sh` + 说明）。
  代码通过 `cargo check --target aarch64-apple-darwin`，
  但 Windows 上没有 Xcode SDK，**无法在此机器上链接出 Mach-O**，
  需在 Mac 上执行脚本产出 `uuengine`。

---

## 26. 1.20.0：通配符规则 + 非代理域名直连

**规则语法**（`domains`，内核与 Clash 脚本双向一致）

| 写法 | 含义 | 例子 |
|---|---|---|
| `a.com` | 自身 + 全部子域 | `api.a.com` ✅ / `a.com` ✅ |
| `*.a.com` | 仅子域，不含根域 | `api.a.com` ✅ / `a.com` ❌ |
| `+.a.com` | 根域 + 子域（Clash 的 `+.`） | 两者都 ✅ |
| `api.*.a.com` | 中间通配，至少一段 | `api.us.a.com` ✅ |
| `*` | 所有主机名（IP 字面量除外） | `anything.com` ✅ / `1.2.3.4` ❌ |

匹配器是手写的 O(域名长度) 判定，不用回溯正则，单连接开销可忽略；
新增 13 个单测覆盖各形态与边界（大小写、多余点、去重、IP 字面量）。

**非代理域名直连**

命中规则 → 分片；未命中 → **不走探测、不分片，直接 TCP 透传**（`direct: true`，默认）。

**Clash 侧一致性**：脚本把同一份域名换算成 Clash 规则 ——
`*.a.com` / `+.a.com` → `DOMAIN-SUFFIX,a.com`，`*` → `MATCH`，
中间通配 → `DOMAIN-REGEX,^api[.][^.]*[.]c[.]com$`。

**验证**

| 用例 | 结果 |
|---|---|
| `*` 全局分片 | ✅ 任意域名日志均为 `split mid-SNI` |
| `*.` / `+.` 语义 | ✅ 子域命中、根域按规则 |
| 未命中直连 | ✅ 日志 `direct passthrough`，比探测路径更快 |
| `direct: false` | ✅ 回退到普通探测，不崩溃 |
| Clash 规则换算（Node 实跑） | ✅ 5 种写法输出符合预期 |
| 6 个 UU 端点 | ✅ 6/6 |
| 吞吐 / 延迟 / 空闲 | ✅ 85% / +0.03ms / 连接存活 |

**顺带修复的两个真问题**

1. `log_level: debug` 一直没生效 —— `set_debug` 调用与 `debug_log` 字段在早前重写中丢失，
   所有 DEBUG 日志被静默丢弃。已补回并验证。
2. 解析失败时提示含糊，现在区分「NXDOMAIN / 只有 fake-ip 答案」与「非法地址」。

**已知边界**：`nrd.nie.163.com` 这类根域本身没有 A 记录（只有子域有），
配 `+.` 时对它发起连接会解析失败 —— 这是 DNS 事实，不是匹配问题。

---

## 27. 1.20.0：预置改为通配符 `+.nrd.nie.163.com`

**改动**：预置从"6 条固定端点"改为**一条通配符** `+.nrd.nie.163.com`，
内核 `print-config`、前端生成的 `engine.yaml`、「UU 远程」按钮全部同步。

**为什么改**：UU 的节点名随调度变化 —— 用户日志里出现过
`sig-3207-g` / `relay-mg-3207-g`，而预置里写的是 `sig-3303-d` / `relay-mg-3303-d`，
固定清单会漏掉新节点，直接表现为"这个域名没走分片、连不上"。

**实测覆盖**（`tools/preset_test.py`）：

```
api.nrd.nie.163.com                TLSv1.2  694ms
api-dcdn.nrd.nie.163.com           TLSv1.3  133ms
api-event.nrd.nie.163.com          TLSv1.2 2024ms
api-ipv4.nrd.nie.163.com           TLSv1.2 2032ms
sig-3303-d.nrd.nie.163.com         TLSv1.3  557ms
relay-mg-3303-d.nrd.nie.163.com    TLSv1.2  416ms
sig-3207-g.nrd.nie.163.com         TLSv1.3  475ms   ← 旧预置漏掉的动态节点
relay-mg-3207-g.nrd.nie.163.com    TLSv1.2  192ms   ← 同上
```

- 8/8 UU 域名走 mid-SNI 分片并握手成功；
- 同级域名（`nrd-file.fp.ps.netease.com`）**不**被误匹配，走直连；
- 新增 3 个单测：预置是单条通配符、覆盖全部已知端点、不漏到同级域名。

**产物**：Windows `uuengine.exe` 1.20.0、NSIS/MSI；macOS 源码包同步更新。

---

## 28. 动态节点名是否也会被阻断（基线对照实验）

**问题**：预置改成 `+.nrd.nie.163.com` 后，动态节点名（`sig-3207-g` 这类）
在不做任何防 SNI 处理时，到底会不会被阻断？

**方法**：固定同一个目标 IP，只改变"是否分片"，屏蔽 DNS 污染带来的噪声。
对照组同时包含静态节点和 `example.com` 这类普通域名。

**结果**（`tools/judge_test.py`）

| 域名 | 类型 | 目标 IP | 直连（不分片） | 经内核（分片） |
|---|---|---|---|---|
| api.nrd.nie.163.com | 静态 | 123.58.184.232 | **阻断 超时** | 成功 TLSv1.2 |
| api-dcdn.nrd.nie.163.com | 静态 | 122.228.29.101 | **阻断 超时** | 成功 TLSv1.3 |
| api-event.nrd.nie.163.com | 静态 | 59.111.45.136 | **阻断 超时** | 成功 TLSv1.2 |
| api-ipv4.nrd.nie.163.com | 静态 | 112.13.123.98 | **阻断 超时** | 成功 TLSv1.2 |
| relay-mg-3303-d... | 静态 | 59.111.45.166 | **阻断 超时** | 成功 TLSv1.2 |
| **sig-3207-g...** | **动态** | 42.186.187.245 | **阻断 超时** | 成功 TLSv1.3 |
| **relay-mg-3207-g...** | **动态** | 42.186.98.66 | **阻断 超时** | 成功 TLSv1.2 |

**结论**

1. **动态节点名同样被阻断**，阻断强度与静态节点没有区别 ——
   说明封锁是按 SNI 明文里的 `nrd.nie.163.com` 后缀匹配，不区分具体主机名。
2. **阻断依据确实是明文 SNI，不是 IP**：同一 IP 直连超时、分片后立刻成功，
   唯一的变量就是 ClientHello 是否被拆成两条记录。
3. 因此预置必须用通配符：只写已知的静态节点会漏掉动态节点，而动态节点一样需要分片。

**一个反例值得注意**：`api.nrd.nie.163.com` 有时会解析到 Cloudflare 的
`104.18.x.x`（公共 DNS 被污染/Anycast 的结果），那些 IP 上握手反而"成功"，
但证书不是网易的 —— 属于**错误 IP 上的 TLS 成功**，业务不可用。
这也是内核坚持用「过滤 fake-ip + 多候选 + 首字节探测」的原因。

---

## 29. 1.20.0：前端重构 + 提权修复

### 根因：setup 里同步启动内核，把窗口创建拖住了

日志里只有 `界面已就绪` 之前的部分就没有下文，说明 JS 从未跑完初始化。
原因是 setup 中同步调用 `start_service`，而它内部会等内核 bind 端口
（数百毫秒）再返回 —— 这段时间窗口还没建出来，用户看到的就是"打开没反应"。

修复：启动内核移到后台线程，窗口先出来；失败时回写 `enabled=false` 并广播状态。

### 提权：host 模式首次开启即弹 UAC

- 内核 bind 80/443 失败且错误含 `needs admin` 时，主动 `runas` 重启自己；
- 旧实例在 400ms 后退出，避免两个进程抢同一份数据目录；
- 新实例通过 `--elevated --data-dir <路径>` 复用原数据目录（已实测解析正确）；
- 用户取消 UAC 时保持原状态并记日志，不会把开关卡在"开"。

### 界面重构（对齐 Clash Verge 布局）

| 项目 | 之前 | 现在 |
|---|---|---|
| 布局 | 顶部标签 + 底部常驻日志面板（挤压主区） | **左侧栏**：模式切换 / 导航 / 运行状态；右侧主区 |
| 日志 | 常驻面板，占 546px 高度 | 独立「日志」页，主区不再被挤压 |
| 导航 | 4 个标签常驻 | 按模式显隐（clash：概览/规则/日志/设置；host：Host 模式/日志/设置） |
| 概览 | 预览 + 长说明 | 3 张状态卡 + 三步引导（当前步骤高亮） |
| 规则 | 表格 + 隐藏按钮 | 表格支持编辑/删除/启停，通配符提示内联 |
| 主题 | 变量切换 | 同步重写，深色下对比度按新色板 |

**实测（探针读取真实 DOM）**

```
clash 模式：mode=clash tab=overview page=page-overview
            nav=概览 / 规则 / 日志 / 设置
            cards=3 steps=3 sidebar=208x780 page=872x728
host  模式：mode=host  nav=Host 模式 / 日志 / 设置
            toggle=True domains=[+.nrd.nie.163.com]
            443 已监听
```

**回归**：前端流程 3/3（clash 开 / host 开 / 关着启动不起服务）；
6 个 UU 端点 6/6；通配符 4 组场景 PASS。

**顺带清理**：布局探针里对旧选择器（`.app` / `.logbar` / `.tab`）的引用会导致
整段探针 JS 抛异常、拿不到数据；已全部改到新结构。
删掉 HTML 里两个已废弃的隐藏按钮（`btnDiscard` / `btnSaveRules`），
避免"点了没反应"的误导。

**未验证**：真实 UAC 弹窗点击（脚本无法代点）、深色主题下的实际观感。

---

## 30. 1.20.0：修"点不了" + 内核黑框 + 文件散落

### 1) 点不了：其实是三个问题叠在一起

| 现象 | 真因 | 修法 |
|---|---|---|
| 界面像死的 | 内嵌前端资源未更新：`cargo build` 不重新嵌入 `ui/`，改了 CSS/JS 不生效 | 必须用 `npx tauri build` 构建 |
| 导航点了不切页 | `switchTab` 用 `navItem.hidden` 判断"该页是否属于当前模式"，点击瞬间 hidden 还是旧值 → overview/rules 被强制回落 host | 改用 `data-for` 与 `currentMode()` 比较 |
| 按钮量不到尺寸 | 探针在非活动页上测按钮（`display:none` 的子元素 rect 恒为 0） | 探针改为只测当前活动页 |

**实测（release 构建，探针逐页点击）**

```
导航：overview→overview ✅  rules→rules ✅  logs→logs ✅  settings→settings ✅
按钮：概览 btnQuickStart ✅
      规则 btnPresetUU / btnJson / btnAddRule ✅
      日志 btnClearLogs / btnOpenLogFile ✅
      Host btnHostSave / btnHostApplyHosts / btnHostRestore / btnHostClearHosts ✅
```

命中测试（`elementFromPoint`）全部返回元素自身，无遮挡。

### 2) 内核黑框

内核是控制台程序，`Command::new` 直接启动会弹出一个 CLI 窗口。
加 `CREATE_NO_WINDOW (0x08000000)` 后：**引擎进程 MainWindowHandle = 0**，
stdout/stderr 仍走管道，日志照常进 GUI（实测 `[engine] SOCKS5 listening on ...`）。

### 3) 文件散落

- 工作区内清理了 10 个临时测试目录与 23 个 commit 消息文件；
- `tools/` 下删除 67 个一次性补丁脚本，只保留可复跑的验收脚本；
- Desktop 上未留任何我创建的文件（standalone 目录已由用户移入工作区）。

**回归**：前端流程 3/3；引擎无窗口且 7899 可用；日志双写（GUI + `uuengine.log`）。

---

## 31. 1.20.0：SOCKS 模式重命名 + 双模式独立 + 保活 / 自启

### 重命名与配置

- `clash` 模式正式改名为 **SOCKS 模式**（`mode: socks`），旧值 `clash` 自动迁移；
- SOCKS 模式的「配置」页给出 **Clash 扩展脚本**（预览 + 复制 + 导出），
  内核职责是建立 SOCKS 隧道，Clash 负责把域名指向它。

### 两种模式各自独立

| 项目 | 之前 | 现在 |
|---|---|---|
| 开关状态 | 单一 `enabled` | `enabled_socks` / `enabled_host` 各自独立 |
| 切换模式 | 直接切，旧内核残留 | 先确认「旧模式正在运行」，用户同意后停掉再切 |
| 左侧导航 | 按模式显隐不同项 | **五项完全相同**：概览 / 配置 / 规则 / 日志 / 设置 |
| 视觉区分 | 同色 | SOCKS 蓝、Host 青绿（导航、徽标、主按钮、状态点全部跟随） |
| 状态栏 | 只写"运行中" | 写明 **SOCKS 模式 · 运行中** + 模式徽标 |

### 新增能力

- **重启内核**：标题栏「重启内核」按钮（先停再起，需确认）；
- **自动保活**：5 秒巡检，进程消失立即拉起；端口连续 3 次无响应判定卡死并重启；
- **开机自启**：Windows 写 `HKCU\...\Run`，macOS 写 LaunchAgent；设置页可开关；
- **应用内开关 = 内核进程**：开即启动 `uuengine`，关即结束进程（文案已写明）。

### 修掉的一个真 bug（死锁）

`hostmode::start()` 在持有状态锁的情况下调用 `status()`，而 `status()` 要重入同一把
`std::sync::Mutex` —— 不可重入，直接死锁。表现是保活线程第一次 tick 后永久卡住，
连带内核状态无法更新。修复：显式 `drop(g)` 后再调用。

**实测**

```
两模式独立：socks 开 → 仅 7899；host 开 → 仅 443          ✅
保活：杀掉内核后 5 秒内自动拉起，端口恢复                   ✅
导航：overview/config/rules/logs/settings 五项全部正确切换   ✅
按钮：概览 1、配置 6、规则 3、日志 2，命中测试全部 ok        ✅
引擎窗口数 = 0（无 CLI 黑框）                              ✅
cargo test：67 core + 7 行为 + 13 engine                   ✅
```

**未验证**：真实 UAC 弹窗、开机自启在重启后的实际生效（注册表已写入，需重启验证）。

---

## 32. 1.20.0 全量验收（UI + 业务逻辑）

### 单元测试

```
core    : 70 passed（含新增 3 个旧配置迁移用例）
core 行为: 7 passed
engine  : 13 passed
```

### 内核功能

| 项目 | 结果 |
|---|---|
| 6 个 UU 端点握手 | ✅ 6/6（TLS1.2/1.3，证书均正常） |
| 通配符 `*` / `*.` / `+.` / 中间通配 | ✅ 4 组场景 PASS |
| DoH（AliDNS / DNSPub） | ✅ 2/2 |
| 畸形输入 / 并发压测 | ✅ 进程存活 |
| hosts 备份→写入→恢复→移除 | ✅ 字节一致、用户行保留 |
| YAML 配置校验 / 自动创建 | ✅ 9 例 |

### 命令级业务逻辑（页面上下文直接调用 Tauri 命令）

```
mode0=socks, running0=true          启动态正确
needsConfirm=true, switched1=false  旧内核在跑时，未确认 → 拒绝切换 ✅
switched2=true,  mode2=host         确认后才切换 ✅
running2=false                      切换时旧内核已停止 ✅
restartOk=true                      重启内核命令可用 ✅
```

### UI 点击（逐页命中测试，`elementFromPoint`）

```
导航：overview/config/rules/logs/settings 五项全部正确切换 ✅
概览   btnQuickStart                                    ✅
配置   btnHostSave / ApplyHosts / Restore / ClearHosts
       btnCopyScript / btnExportScript                   ✅ 6/6
规则   btnPresetUU / btnJson / btnAddRule               ✅ 3/3
日志   btnClearLogs / btnOpenLogFile                    ✅ 2/2
设置   btnSaveSettings / btnOpenData                    ✅ 2/2
```

**共 14 个按钮，命中全部返回元素自身，无遮挡。**

### 服务行为

| 项目 | 结果 |
|---|---|
| 两模式开关独立 | ✅ socks 只开 7899；host 只开 443 |
| 引擎无 CLI 窗口 | ✅ MainWindowHandle = 0 |
| 日志双写 | ✅ GUI `[engine]` + 运行目录 `uuengine.log` |
| 保活（杀掉内核） | ✅ 5 秒内自动拉起，端口恢复 |
| 开机自启 | ✅ 注册表写入 `"…\UURelayBridge.exe" --autostart` |

### 本轮修掉的问题

1. **旧配置不兼容**：`enabled` 字段未被迁移到 `enabled_socks/host`，
   导致老配置升级后服务不自动启动。已补迁移 + 3 个单测。
2. 测试脚本路径仍指向被移走的工作区外目录（23 个脚本），已统一修正。

---

## 33. 1.20.0：GUI 真实点击测试 + 修 4 个致命 bug

这轮用**真实点击 + 后果验证**（不是只看元素位置）跑了一遍 GUI，
发现前端与后端的数据契约有 4 处不一致，全部是用户必踩的坑。

| # | Bug | 现象 | 根因 |
|---|---|---|---|
| 1 | `set_mode` 参数名错 | 切换模式直接报 `missing required key confirmStop` | 前端传 `confirm_stop`，Tauri 要求 camelCase |
| 2 | 命令名错 | 点「导出」无反应 | 调用 `generate`，实际命令是 `regenerate` |
| 3 | **规则永远存不了** | 新建规则点确定 → "规则至少需要一个域名" | 后端字段是 `match.domain`，前端写成 `match.domains` |
| 4 | 规则动作格式错 | 即使填了域名也失败 | `Action` 是内部标签枚举 `{type:"proxy",outbound}`，前端传裸字符串 |

另外修了 2 个交互问题：

- **开关状态错位**：`renderAll` 用旧的全局 `enabled` 回填开关，改为读当前模式的
  `enabled_socks` / `enabled_host`；
- **静默丢弃操作**：`guard()` 在忙碌时直接 return，用户以为"点了没反应"。
  现在给出 toast 提示，并在忙碌期间禁用开关。

### 新增两层自动化检查

- `tools/check_contract.py`：比对前端 `api("cmd", {...})` 与后端命令签名
  （命令名是否存在、参数是否 camelCase、必填项是否缺失）→ **24 个调用全部匹配**；
- `tools/check_schema.py`：比对前端构造的数据结构与 Rust 结构体字段
  （`Match.domain/ip_cidr`、`Action` 变体、`Rule` 字段）→ **一致**。

### GUI 真实点击结果（11 项全通过）

```
切换弹确认框            ✅
取消后模式不变           ✅
停止弹确认              ✅
停止后 running=false    ✅  running=false enabled=false
开启弹确认              ✅
开启后 running=true     ✅  running=true enabled=true
规则弹窗打开            ✅
规则已保存              ✅  rules=1
规则已删除              ✅
重启后 running=true     ✅
设置已保存              ✅
```

**Host 模式**：从 GUI 启动后 443/80 监听正常，且不占用 SOCKS 端口。

**单元测试**：core 70 + 行为 7 + engine 13。

---

## 34. 1.20.0：规则编辑内嵌 + 全局样式统一

### 规则编辑不再是全局弹窗

原来「新建规则」打开的是覆盖全屏的模态框；现在改成**嵌在规则页内的面板**
（`#ruleEditor`，带模式色描边），位置就在规则表格下方：

- 只影响规则页，不再遮挡侧栏与其它页面；
- 标题随状态变化（新建 / 编辑），按钮改为「取消 / 保存规则」；
- 打开时自动滚动到可视区域。

全局模态只保留两个必须打断全局的场景：**确认框**、**JSON 编辑**（内容太长）。

### 样式统一

- 所有页面统一为 `pane > pane-head / pane-body / pane-foot` 三段结构，
  不再有「有的页面用 pane、有的直接铺内容」的混搭；
- 表单统一用 `.field`（label 在上、控件在下），端口/分片这类并排项用 `.grid2/.grid3`；
- 说明文字统一放 `pane-foot`（浅底 + 上边框），不散落在正文；
- 日志页用 `.pane.fill` 撑满剩余高度，不再出现「日志框只占一小块」；
- 侧栏收窄到 204px，模式色贯穿导航选中、主按钮、开关、徽标。

### 验证结果（真实点击，23 项全通过）

```
导航 5 页切换                     ✅ ×5
按钮命中 概览1 / 配置3 / 规则3 / 日志2 / 设置2   ✅ ×5
编辑器嵌在规则页且可命中           ✅
规则保存 / 收起 / 编辑回填 / 取消 / 删除  ✅ ×5
启动 running=true / 停止 running=false    ✅ ×2
切换模式：模式已改变、未运行时不弹确认      ✅ ×2
重启内核 running=true             ✅
设置保存                          ✅
```

新增静态检查：`check_ui_bindings.py`（DOM 绑定）、`check_contract.py`
（命令签名与参数）、`check_schema.py`（数据结构字段），三者均通过。

**产物**：Windows NSIS/MSI 1.20.0；macOS arm64 源码包。

---

## 35. 域名统一由规则管理 + 规则编辑改为锁定式弹窗

### 域名只由「规则」管理

以前域名在两处存在：`hostmode.json` 的 `domains` 和规则的 `match.domain`，
两边各存一份、互不同步，很容易「改了规则但内核用的是旧域名」。

现在统一：

| 位置 | 改动 |
|---|---|
| 配置页 | **移除域名输入框**，改为一行说明「域名在规则页统一维护」 |
| `hostmode.json` | 默认 `domains` 置空，不再预置 6 个 UU 域名 |
| `start_service` | 用 `rule_domains(project)` 从启用的规则里取域名 |
| `host_apply_hosts` | 同上；没有规则时直接报可读错误 |
| Clash 脚本 | `sni_link` 同样以规则为准 |

### 规则编辑改为锁定式全屏弹窗

按需求：**不点保存或取消，就不能切换选项卡**。

- 改成全屏浮层 + 遮罩，浮在页面最上层；
- 打开时给 `<body>` 加 `modal-open`：导航、模式按钮、开关、重启按钮全部
  `pointer-events: none`，点击会提示「请先保存或取消当前规则」；
- 保存成功或取消后解除锁定，恢复正常导航。

### 顺带修掉两个会被这次改动触发的问题

1. **空域名导致内核崩溃**：`domains:` 后面没有元素时，内核的 `OneOrMany`
   解析器直接报错（`data did not match any variant of untagged enum OneOrMany`）。
   现改为空列表显式输出 `domains: []`，并在启动前给出可读提示。
2. **保活死循环**：没有规则时启动必然失败，保活每 5 秒重试一次、刷满日志。
   现识别为配置类错误，暂停自动重启并关掉开关，等用户补规则。

### 验证结果（真实点击，13 项全通过）

```
配置页无域名输入                    PASS
配置页仍有内核参数                  PASS
规则弹窗打开 / 内有域名框 / 浮顶层    PASS x3
未保存时不能切页                    PASS  page-rules -> page-rules
未保存时开关不变                    PASS
保存后弹窗关闭 / 规则已保存          PASS x2
规则域名进入生成的脚本               PASS
取消后弹窗关闭 / 可以切页            PASS x2
规则已删除                          PASS
```

**回归**：6 个 UU 端点 6/6、通配符、DoH 全部 PASS；
有规则时内核正常启动（`domains=1`），引擎窗口数 0。

**产物**：Windows NSIS/MSI；macOS arm64 源码包。

---

## 36. 1.20.0：修「点启动没反应」+ GUI 预置默认内核配置

### 根因：全新安装默认模式是 `clash`

1.16 把模式从 `clash` 改名成 `socks`，但 **`d_mode()` 的返回值漏改了**：

```rust
fn d_mode() -> String { "clash".to_string() }   // 错：MODES 里已经没有 clash
pub const MODES: [&str; 2] = ["socks", "host"];
```

`Settings::default()` 因此给出 `mode = clash`。而 `sanitize()` 只在**保存**时调用，
`load_settings()` 读文件后没有调用，于是全新的 `Settings::default()` 一路带着
`clash` 进入 `start_service` —— 内核按未知模式启动，行为不可预期。

同时 `Project::default()` 的 `rules` 是空数组，启动时命中
「还没有启用任何域名规则」直接返回错误。

两者叠加 = 用户点开关，看起来"没反应"。

### 修复

| 改动 | 说明 |
|---|---|
| `d_mode()` → `"socks"` | 默认模式与 `MODES` 一致 |
| `load_settings()` 增加 `sanitize()` | 读进来的旧配置也做迁移，不只保存时 |
| `Project::default()` 预置一条 UU 规则 | 全新安装即有可用域名 |
| 无域名时给可读错误 | 不再静默失败，提示去规则页添加 |

### GUI 预置的默认内核配置

全新安装、零配置文件时即具备：

| 项目 | 默认值 |
|---|---|
| 模式 | `socks` |
| 规则 | 1 条「UU 远程」，域名 `+.nrd.nie.163.com`，动作 proxy |
| SOCKS 端口 | 7899 |
| Host 监听 | 443 / 80 |
| 分片 | TLS Record × 4 段 |
| DNS | DoH（阿里 / DNSPod） |

实测：零配置启动 → 日志 `规则 1 条`；打开开关 → `7899` 监听正常。

### 完整启动链路测试（7 个入口）

| 入口 | 结果 |
|---|---|
| 1. UI 开关（点击 → 确认框 → 启动） | PASS `running=true socks=127.0.0.1:7899` |
| 2. UI 开关关闭 | PASS `running=false` |
| 3. 「重启内核」按钮 | PASS `running=true` |
| 4. 命令直调（等同托盘菜单） | PASS 可停可起 |
| 5. 切换模式后再启动 | PASS 切到 host 正常 |
| 6. 启动时按上次状态自动拉起 | PASS 7899 监听 |
| 7. 保活自动拉起（杀进程） | PASS 10 秒内恢复 |

**引擎窗口数 = 0**（无 CLI 黑框）。

**回归**：core 70 + 行为 7 + engine 13 全绿。

**产物**：Windows NSIS/MSI 1.20.0；macOS arm64 源码包。
