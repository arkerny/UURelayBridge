# RelayBridge v1.2

轻量级 **Clash Verge Rev 扩展配置生成器**。常驻托盘 / 菜单栏，把你定义的域名 / IP 规则生成一段
`main(config)` 扩展脚本，由你手动粘贴进 Clash Verge Rev。

> RelayBridge **不是**代理客户端：不跑内核、不接管代理、不改订阅、不碰 Clash Verge 的任何文件。

---

## 一、它做什么 / 不做什么

**做**

- 维护一份自己的规则模型（`project.json`）。
- 按 ON/OFF 生成 `relaybridge.js`（Clash Verge Rev 扩展脚本），并保留历史备份 + `manifest.json`（SHA-256）。
- 托盘 / 菜单栏常驻：关闭窗口不退出，可随时唤回。
- 实时日志、生成预览、单条规则启停、JSON 高级编辑。
- 深色 / 浅色主题，默认**跟随系统**；切换后标题栏与界面同时生效，选择会存进 `settings.json`。
- 原子写入：写临时文件 → fsync → 替换；失败时旧文件保持完好。

**不做（硬性约束）**

- 不修改 Clash Verge 的订阅、配置、数据库、安装目录。
- 不自动导入扩展脚本，不自动重载配置，不调用 Controller API，不探测 Clash 状态。
- 不改系统代理、Hosts、防火墙，不装服务 / 系统扩展 / 网络扩展。
- 不需要管理员权限。

**所有写操作都要经过用户授权**：界面每次落盘前弹确认框，后端命令再校验一次
`approved=true`（前端绕过也无效）；托盘上的点击本身即视为授权。**启动阶段只读不写。**

---

## 二、界面

| 页 | 内容 |
|---|---|
| 概览 | 状态 / 目标内核 / 最近生成 / 输出文件（含 sha256）、脚本预览、接入步骤、校验提示 |
| 规则 | 表格增删改、单条启停、域名 / IP-CIDR、动作（代理到策略组 / 直连 / 拒绝）、JSON 编辑器 |
| 设置 | 主题（跟随系统 / 浅色 / 深色）、启动隐藏、关闭到托盘、操作提示、目标内核、备份份数、目录入口 |
| 底部 | 实时日志（分级着色、自动滚动可关、可清空、可复制） |

规则改动先进「草稿」，点 **保存规则** 才落盘并重新生成 —— 这样"改"和"写"是两件事。

---

## 三、工程结构（workspace）

```
RelayBridge/
├── Cargo.toml                  # workspace（core + src-tauri）
├── core/                       # relaybridge-core：纯逻辑，不依赖 Tauri，可单测
│   └── src/{model,rules,compiler,validator,paths,settings}.rs
│       ├── adapters/           # Adapter trait + cvr（实现）/ mihomo / singbox（占位）
│       └── exporter/           # writer(原子写) / backup / manifest
├── src-tauri/                  # relaybridge：Tauri 2 应用
│   ├── src/{lib,main,commands,selftest}.rs
│   ├── src/state/              # 运行期状态（RwLock 内存态）
│   ├── src/tray/               # 托盘 / 菜单栏
│   └── src/platform/           # windows.rs / macos.rs（仅系统调用）
├── ui/                         # 原生 HTML/CSS/JS，无构建步骤
├── icons/                      # icon.ico / icon.icns / 托盘 on-off / macOS 模板图
├── docs/                       # 接入指南、macOS 构建、架构、测试记录、示例脚本
└── tools/                      # 图标生成器、UI 绑定检查等开发脚本
```

**为什么拆两层**：核心逻辑不链接 Tauri，才能在任意环境跑 `cargo test`；
新增内核只需要在 `core/src/adapters/` 加一个实现，UI 和命令层都不用改。

---

## 四、构建

依赖：Rust 1.77+（实测 1.93）、Node 18+（仅打包）、WebView2（Win10/11 自带）。

```powershell
cd RelayBridge

# 只出可执行文件
cargo build --release
# 产物：target\release\relaybridge.exe

# 出安装包（NSIS + MSI，用户级安装）
npm install -D @tauri-apps/cli@^2
npx tauri build
# 产物：target\release\bundle\nsis\RelayBridge_1.2.0_x64-setup.exe
#       target\release\bundle\msi\RelayBridge_1.2.0_x64_en-US.msi
```

macOS 见 [docs/build-macos.md](docs/build-macos.md)（`.app` 必须在 Mac 上编译、签名、公证）。

> 本仓库的 `target/` 已随 workspace 上移：所有产物都在 `RelayBridge/target/` 下。

---

## 五、使用

1. 「规则」页新建规则，例如：域名 `nrd.nie.163.com`（后缀匹配，自动覆盖所有子域），
   动作「代理到 `🔰 节点选择`」（必须是你 Clash 配置里真实存在的策略组名）。
2. 点 **保存规则** → 确认 → 写入 `project.json` 并生成脚本。
3. 顶部开关切 **ON** → 确认 → 生成启用版本。
4. Clash Verge Rev → 订阅 → 右键配置 → **编辑扩展脚本** → 粘贴 `relaybridge.js` 全文 → 保存 → 重载。
5. 停用：开关切 OFF → 重新粘贴 → 重载（程序不会替你通知 Clash）。

图文细节与 FAQ：[docs/手动接入指南.md](docs/手动接入指南.md)
生成的脚本长什么样：[docs/sample-relaybridge.js](docs/sample-relaybridge.js)

---

## 六、数据目录

```
Windows: %APPDATA%\RelayBridge\
macOS:   ~/Library/Application Support/RelayBridge/
├── settings.json
├── project.json
├── output\clash-verge-rev\
│   ├── relaybridge.js
│   └── manifest.json          # 应用版本 / 模型版本 / 目标 / 时间 / 文件名 / sha256 / 是否启用
└── backups\
    └── relaybridge-<epoch>-<ms>.js
```

路径全部来自 Tauri Path API，业务代码里没有盘符或用户名硬编码；
`state/paths.rs` 是唯一路径来源，输出目录名做了字符过滤防穿越。

**排障/便携用环境变量**（普通用户不需要）：

| 变量 | 作用 |
|---|---|
| `RELAYBRIDGE_DATA_DIR` | 覆盖数据目录（便携模式 / 自动化测试） |
| `RELAYBRIDGE_WEBVIEW_DIR` | 覆盖 WebView2 用户数据目录 |
| `RELAYBRIDGE_LOG_FILE` | 把实时日志同时追加到文件 |

---

## 七、测试

```powershell
cargo test -p relaybridge-core        # 41 个单元测试 + 4 个"真跑 JS"的行为测试
cargo run -p relaybridge-core --example render_sample   # 打印一份示例脚本
cargo build -p relaybridge            # 构建后再跑下面的自检
.\target\debug\relaybridge.exe --selftest               # 真实进程内跑授权→生成→备份→OFF
```

- 单元测试：模型 / 域名与 CIDR 校验 / 编译去重保序 / 校验器 / 原子写 / 备份轮转 / 设置损坏恢复 / 路径穿越 / 日志环形缓冲。
- 行为测试（`core/tests/cvr_script_semantics.rs`）：把生成的 JS 交给 **Node** 执行，断言
  注入顺序、重复加载不叠加、OFF 精确还原、空规则不改 `rules`。
- `--selftest`：应用内端到端，输出 `SELFTEST OK` 并写日志。

结果见 [docs/test-record.md](docs/test-record.md)（含未验证项，如实标注）。

---

## 八、已知限制

- 首期只实现 Clash Verge Rev 适配器；Mihomo / sing-box 保留接口，调用会明确报「未实现」。
- 不做系统通知（用界面 toast + 日志，少一个插件依赖）。
- 策略组名无法替你校验是否存在（脚本里只 `console.warn`）。
- macOS 侧代码同源但**未在本机验证**（Windows 机器无法交叉编译 `.app`）。

---

## 九、许可

仅用于个人配置管理。使用前请自行确认所在网络/单位的相关规定。
