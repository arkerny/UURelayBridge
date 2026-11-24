# UURelayBridge v1.3

轻量级 **Clash Verge Rev 规则生成器 + 本地 TLS 分片转发器**。Windows 托盘 / macOS 菜单栏常驻。

两种模式，顶部一键切换，**打开后拨一下开关就把服务起好**：

| 模式 | 服务 | Clash 侧 | 需要管理员 |
|---|---|---|---|
| **Clash 规则**（默认） | 本机 SOCKS5（默认 `127.0.0.1:7899`） | 脚本注入 `socks5` 节点 + 域名规则，Clash 自己把目标域名交给它 | 不需要 |
| **Host 模式** | 本机监听 80 / 443 | 不动 Clash，用 hosts 把域名指到本机 | 改 hosts 需要 |

> UURelayBridge **不是**代理客户端：不跑内核、不接管系统代理、不改订阅、不碰 Clash Verge 的文件。

---

## 一、它做什么 / 不做什么

**做**

- 维护自己的规则模型（`project.json`），生成 Clash Verge Rev 扩展脚本，界面上直接**复制**。
- Host 模式：本地 TLS 转发 + 按域名做 TLS Record / TCP 分段，用来绕过按明文 SNI 匹配的阻断。
- 原子写入、历史备份、`manifest.json`（含 SHA-256）—— 只针对**导出**这一个动作。
- 深/浅主题（默认跟随系统）、可拖拽高度的实时日志。

**不做（硬性约束）**

- 不修改 Clash Verge 的订阅、配置、数据库、安装目录。
- 不自动导入扩展脚本、不调用 Controller API、不探测 Clash 状态。
- 不改系统代理、不改防火墙、不装服务 / 驱动 / 系统扩展。
- 不自动启动第三方内核。
- 默认**不写任何文件**：生成结果留在界面里，你自己复制。
- 所有落盘 / 系统改动都需要你在界面上确认；改 hosts 才会请求管理员权限。

---

## 二、界面

| 页 | 内容 |
|---|---|
| 概览 | 状态、目标内核、上次导出、预览全文 + 复制 / 导出到文件、接入步骤 |
| 规则 | 表格增删改、单条启停、域名 / IP-CIDR、三种动作、JSON 高级编辑 |
| **本机转发** | 域名、端口、分片方式与段数、DNS（支持 DoH）、启动/停止、写入/移除 Hosts、连接统计 |
| 设置 | 主题、启动隐藏、关闭到托盘、提示、目标内核、备份份数、目录入口 |
| 底部日志 | 实时分级日志；**上边缘可拖动改高度**（双击复位 220px），高度记在 localStorage |

---

## 三、规则模式：复制即用

1. 「规则」页新建规则，例如域名 `nrd.nie.163.com`（后缀匹配，覆盖所有子域），
   动作选「代理到策略组」，填你在 Clash 配置里真实存在的策略组名。
2. 顶部开关切 **ON** —— 只切换"生成内容"，**不写文件**。
3. 「概览 → 生成内容预览」点 **复制**。
4. Clash Verge Rev → 订阅 → 右键配置 → 编辑扩展脚本 → 粘贴 → 保存 → 重载。

想留一份文件副本时再点 **导出到文件**（写到 `output/clash-verge-rev/`，旧版自动备份）。
脚本语义：ON 把规则插到最前（`MATCH` 之前）；OFF 只摘掉自己注入的完全相同规则；重复加载不会叠加。

---

## 四、Host 模式：本机 80/443 转发

链路：

```
应用 → hosts 把域名指向 127.0.0.1 → UURelayBridge 监听 443（TLS）/ 80（HTTP）
     → 读 ClientHello 取 SNI → 用 DNS 查真实 IP（绕过被改写的 hosts）
     → 按需把这条记录拆成多段 → 转发 → 之后双向透传（不终止 TLS）
```

操作顺序：

1. 顶部切到「Host 模式」，在页面里核对域名（默认已填 6 个 UU 端点）、端口、分片方式。
2. 拨开关 → 服务启动（443 + 80 一起监听，日志会写监听地址）。
3. 点「写入 Hosts」→ UAC → 完成。
4. 不用时拨回开关，再点「移除 Hosts」；想知道改了什么、想一键还原，点「恢复原 hosts」
   （每次写入前都会把原文件备份到 `backups/hosts/`，只保留最近 5 份）。

> 如果 Hosts 还写着、服务却停了，界面顶部会出现提示条，提醒你开开关或恢复 hosts。

**顺序很重要**：先启动转发，再写 hosts。不用时点「移除 Hosts」，或保持程序运行。

### 预置：UU 远程端点

内置了下面 6 个端点（都在 `nrd.nie.163.com` 下），新装即已填好，两个模式各有一个「UU 远程」按钮可一键填入：

```
api.nrd.nie.163.com            核心控制与房间 API
api-dcdn.nrd.nie.163.com       DCDN 灾备控制 API
api-event.nrd.nie.163.com      客户端事件上报 API
api-ipv4.nrd.nie.163.com       IPv4 远程唤醒与初始化
sig-3303-d.nrd.nie.163.com     WebSocket 控制信令
relay-mg-3303-d.nrd.nie.163.com  TURN/TLS 画面中继
```

只想写一行的话，填 `nrd.nie.163.com` 就能覆盖全部（后缀匹配）。

### DNS 支持 DoH

「本机转发」页的 DNS 一栏既可以填 `223.5.5.5` 这种明文 DNS，也可以填
`https://dns.alidns.com/dns-query` 这类 DoH 地址（下拉里有 AliDNS / DNSPod / Cloudflare / Google 预设）。
选 DoH 时只用加密查询，不会退化成明文。

### SNI 配置同时作用于两种模式

域名与分片参数只有一份，两种模式各用各的落地方式：

- **Clash 模式**：脚本往配置里加一个 `socks5` 节点（`127.0.0.1:<SOCKS 端口>`，
  名字 `UURelayBridge-SNI`）+ `DOMAIN-SUFFIX,<域名>,UURelayBridge-SNI` 规则。
  Clash 把连接交给本机 SOCKS，由我们分片后再发往真实服务器。
- **Host 模式**：`dns.hosts` 指到 127.0.0.1 + `DIRECT` 规则（或直接用系统 hosts）。

> Clash 模式是推荐路径：不依赖内核连回环地址的行为，也不动 DNS 配置。
> 开关关闭时，脚本会把自己加过的节点和规则一并撤掉。

限制（不承诺解决）：DNS 被污染、IP 被墙、QUIC 被封、目标服务器不可用、中间设备能完整重组 TLS、
主动探测类封锁 —— 这些不是分片能解决的。分片只对"按明文 SNI 简单匹配"的设备有效。

---

## 五、构建

```powershell
cd UURelayBridge          # 仓库根目录（workspace）
cargo build --release     # 产物：target\release\UURelayBridge.exe
npm install -D @tauri-apps/cli@^2
npx tauri build           # NSIS + MSI，用户级安装
```

macOS 见 [docs/build-macos.md](docs/build-macos.md)（`.app` 必须在 Mac 上编译/签名/公证）。

---

## 六、数据目录

```
Windows: %APPDATA%\UURelayBridge\
macOS:   ~/Library/Application Support/UURelayBridge/
├── settings.json
├── project.json
├── hostmode.json          # Host 模式配置
├── hosts-op.json          # 提权子进程回写的结果
├── output/clash-verge-rev/{relaybridge.js, manifest.json}
└── backups/
```

环境变量（排障 / 便携）：`UURELAYBRIDGE_DATA_DIR`、`UURELAYBRIDGE_WEBVIEW_DIR`、
`UURELAYBRIDGE_LOG_FILE`、`UURELAYBRIDGE_WEBVIEW_ARGS`、`UURELAYBRIDGE_DUMP_LAYOUT`、
`UURELAYBRIDGE_HOSTS_FILE`（测试用，覆盖 hosts 路径）。旧的 `RELAYBRIDGE_*` 仍然兼容。

---

## 七、测试

```powershell
cargo test -p uurelaybridge-core     # 52 单元测试 + 4 个真跑 Node 的脚本行为测试
cargo run -p uurelaybridge-core --example render_sample
.\target\debug\UURelayBridge.exe --selftest          # 授权→生成→备份→OFF 全链路
.\target\debug\UURelayBridge.exe --host-selftest 18443 example.com   # 本地转发 20 秒
```

- 单元测试覆盖：模型 / 域名与 CIDR 校验 / 编译去重保序 / 校验器 / 原子写 / 备份轮转 /
  设置损坏恢复 / 主题 / 路径穿越 / 日志 / **TLS ClientHello 解析与分片 / DNS 报文 / hosts 改写**。
- 脚本行为测试：把生成的 JS 交给真实 Node 执行，断言注入顺序、幂等、OFF 精确还原。
- `--host-selftest`：真实进程起转发，配合 `tools/host_selftest.py` 验证「配置域名分片 + 未配置域名透传」。

结果见 [docs/test-record.md](docs/test-record.md)。

---

## 八、已知限制

- 首期只实现 Clash Verge Rev 适配器；Mihomo / sing-box 仍是占位（调用会明确报未实现）。
- Host 模式的 443 监听：Windows 普通用户可绑；macOS 需要 root。
- 分片参数是经验值（默认 4 段）；不同网络设备行为不同，可能需要微调。
- 策略组名无法替你校验是否存在（脚本里只 `console.warn`）。
- macOS 侧代码同源但**未在本机验证**。

SNI 模块的长期设计（多内核、ECH、诊断）见 [docs/SNI模块设计.md](docs/SNI模块设计.md)。
