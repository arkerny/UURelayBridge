#!/usr/bin/env bash
# 在 macOS (Apple Silicon) 上构建 uuengine 内核（arm64）。
#
# Windows 上无法链接 macOS 二进制（需要 Xcode SDK 与 Mach-O 链接器），
# 所以 arm64 内核必须在 Mac 上产出。这个脚本做三件事：
#   1. 装 aarch64 目标
#   2. 编译 release 内核
#   3. 把产物放到 dist/macos-arm64/
#
# 用法：
#   ./scripts/build-macos-arm64.sh
#
# 前置条件：已安装 Xcode Command Line Tools（xcode-select --install）与 Rust。

set -euo pipefail

cd "$(dirname "$0")/.."

echo "== 环境 =="
uname -m
rustc --version
xcode-select -p || true

echo "== 准备 target =="
rustup target add aarch64-apple-darwin

echo "== 构建内核 =="
cargo build --release --target aarch64-apple-darwin -p uuengine

OUT="dist/macos-arm64"
mkdir -p "$OUT"
cp "target/aarch64-apple-darwin/release/uuengine" "$OUT/uuengine"
chmod +x "$OUT/uuengine"

echo "== 自检 =="
file "$OUT/uuengine"
"$OUT/uuengine" version
"$OUT/uuengine" print-config | head -8

echo
echo "产物：$OUT/uuengine"
echo
echo "首次运行会在同目录生成 engine.yaml 与 uuengine.log；"
echo "host 模式监听 80/443 需要 root：sudo $OUT/uuengine"
echo
echo "如需同时构建前端（Tauri），在 Mac 上执行："
echo "  cd RelayBridge && npx @tauri-apps/cli@^2 build"
