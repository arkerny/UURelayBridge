# macOS 构建、签名与公证

> 结论先行：代码是同一套（`core/` 平台无关，`src-tauri/src/platform/macos.rs` 只差"打开目录"这一行），
> 但 **`.app` 必须在 macOS 上编译**。Windows 上不能交叉编译 Tauri 的 macOS 包。

---

## 1. 环境

```bash
xcode-select --install          # Xcode Command Line Tools
rustup target add aarch64-apple-darwin   # Apple Silicon
rustup target add x86_64-apple-darwin    # Intel（可选）
node -v && npm -v               # 仅打包用
```

## 2. 构建 .app

```bash
cd RelayBridge
npm install -D @tauri-apps/cli@^2

# Apple Silicon
npx tauri build --target aarch64-apple-darwin

# Intel
npx tauri build --target x86_64-apple-darwin
```

产物：

```
src-tauri/target/aarch64-apple-darwin/release/bundle/macos/RelayBridge.app
src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/RelayBridge_1.15.1_aarch64.dmg
```

> 注意：workspace 的 target 目录在 `RelayBridge/target/` 下时，路径相应变成
> `RelayBridge/target/aarch64-apple-darwin/release/bundle/...`。以命令实际输出为准。

只想快速跑起来调试：

```bash
cd src-tauri
cargo run --target aarch64-apple-darwin
```

## 3. 未签名版本的 Gatekeeper 行为

没有 Apple Developer 账号时，直接双击 `.app` 可能提示
"无法打开，因为 Apple 无法检查其是否包含恶意软件"。

本地自用可以：

```bash
xattr -dr com.apple.quarantine /Applications/RelayBridge.app
open /Applications/RelayBridge.app
```

**这属于开发/自测版本**，对外分发前必须签名 + 公证。

## 4. 签名

准备：Apple Developer Program 账号、Developer ID Application 证书（钥匙串里已安装）。

```bash
codesign --force --deep --options runtime \
  --sign "Developer ID Application: Your Name (TEAMID)" \
  RelayBridge.app

codesign --verify --strict --verbose=2 RelayBridge.app
spctl --assess --type execute --verbose RelayBridge.app
```

Tauri 也支持在 `tauri.conf.json` 的 `bundle.macOS` 里配置
`signingIdentity`，或用环境变量：

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
npx tauri build --target aarch64-apple-darwin
```

## 5. 公证（notarize）

```bash
ditto -c -k --keepParent RelayBridge.app RelayBridge.zip

xcrun notarytool submit RelayBridge.zip \
  --apple-id "you@example.com" \
  --team-id "TEAMID" \
  --password "app-specific-password" \
  --wait

xcrun stapler staple RelayBridge.app
spctl --assess --type execute --verbose RelayBridge.app   # 应显示 accepted
```

Tauri 2 内置公证支持，设置以下环境变量后 `tauri build` 会自动提交：

```bash
export APPLE_ID="you@example.com"
export APPLE_PASSWORD="app-specific-password"
export APPLE_TEAM_ID="TEAMID"
```

## 6. 菜单栏图标注意事项

- 托盘图标使用 `icons/tray/tray-on-template.png` / `tray-off-template.png`，
  是**纯黑 + 透明**的模板图；代码里调用了 `icon_as_template(true)`，
  macOS 会自动按浅色/深色菜单栏反色。
- 状态不靠颜色区分：菜单第一行文字会显示「RelayBridge 已启用 / 已停用 / ⚠ 上次生成失败」。
- 左键点击弹菜单（`show_menu_on_left_click(true)`），符合菜单栏应用习惯；
  Windows 则是左键打开主界面。

## 7. 关闭窗口 / 退出

- 关闭主窗口：`CloseRequested` 被拦截 → 只 `hide()`，进程与菜单栏图标保留。
- 退出：菜单栏 `Quit RelayBridge` → `app.exit(0)`；`RunEvent::ExitRequested` 在 macOS 上
  不阻止（保留 Cmd+Q 的原生行为）。
- 数据目录：`~/Library/Application Support/RelayBridge/`，退出不删除。

## 8. 验证清单（在 Mac 上必须逐条过）

- [ ] 应用能启动，菜单栏出现图标
- [ ] 关闭窗口后图标仍在、进程仍在
- [ ] 点菜单栏图标 → 打开主界面
- [ ] 开关 ON → 生成 `relaybridge.js`（`~/Library/Application Support/RelayBridge/output/...`）
- [ ] 开关 OFF → 重新生成，`var RB_ENABLED = false;`
- [ ] 退出后进程消失，数据目录仍在
- [ ] 重新启动后状态恢复
- [ ] `cargo run -- --selftest` 输出 `SELFTEST OK`

> 本仓库的 Windows 机器无法执行以上 macOS 验证，测试记录里如实标注为"未验证"。
