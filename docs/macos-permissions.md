# macOS 权限说明

UURelayBridge 在 macOS 上需要三类权限，各有不同的触发时机和处理方式。

## 1. 自动化权限（提权用）

Host 模式要监听 **80 / 443**，这两个端口在 macOS 上只允许 root 绑定。

程序的做法：检测到内核返回 `needs admin` 时，用 `osascript` 调起系统授权框：

```
do shell script "..." with administrator privileges
```

macOS 第一次会弹两个框：

1. **「UURelayBridge」想要控制「System Events」** —— 这是自动化权限（TCC）
2. **输入管理员密码**

> **如果第 1 个框点了「不允许」**，后续提权会一直失败，且 macOS 不会再弹框。
> 需要手动打开：
> `系统设置 → 隐私与安全性 → 自动化 → UURelayBridge → 勾选 System Events`

程序在失败时会给出这个提示，日志里也会记录。

## 2. 修改 hosts

把域名指到 `127.0.0.1` 需要写 `/etc/hosts`，同样需要 root。

- 写入前会自动备份到数据目录的 `backups/hosts/`；
- 写入的段落用标记包裹，卸载时可精确移除，不会动你自己的条目；
- 失败时按第 1 条的方式提权。

## 3. 开机自启

macOS 用 LaunchAgent（不需要 root）：

```
~/Library/LaunchAgents/com.uurelaybridge.desktop.plist
```

在「设置」里勾选「开机自动启动」即写入，取消勾选会 `launchctl unload` 并删除文件。

---

## 与 Windows 的差异

| 项目 | Windows | macOS |
|---|---|---|
| 提权方式 | `ShellExecuteW(runas)` → UAC | `osascript` → 系统授权框 |
| 额外权限提示 | 无 | **自动化权限（TCC）**，容易被忽略 |
| 端口 80/443 | 需管理员 | 需 root |
| 开机自启 | `HKCU\...\Run` 注册表 | LaunchAgent plist |
| 提权后进程 | 新进程接管，旧实例退出 | 同左（`spawn` 后不阻塞） |

## 已知限制

- **GUI（Tauri）必须在 Mac 上构建**：Windows 上没有 macOS 的 `cc` 工具链，
  `cargo check --target aarch64-apple-darwin` 对 `core` 和 `engine` 可以通过，
  但 `src-tauri` 会卡在 `objc2-exception-helper` / `vswhom-sys` 的构建脚本上。
- 本仓库的 `dist/uuengine-*-macos-arm64-src.zip` 提供的是**源码 + 构建脚本**，
  不是编译好的二进制。

## 构建步骤（在 Mac 上）

```bash
# 1. 内核
unzip uuengine-x.y.z-macos-arm64-src.zip -d mac && cd mac
./build-macos-arm64.sh          # 产出 ./uuengine

# 2. GUI
cd ../RelayBridge
rustup target add aarch64-apple-darwin
cp ../mac/uuengine src-tauri/binaries/uuengine-aarch64-apple-darwin
npx @tauri-apps/cli build        # 产出 .app / .dmg

# 3. Host 模式需要 root
sudo ./uuengine run --config engine.yaml
```
