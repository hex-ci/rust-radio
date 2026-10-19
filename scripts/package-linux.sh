#!/usr/bin/env bash
# 打包 Linux 发布包：release 二进制 + 图标 + .desktop + 许可文本，输出 dist/rust-radio_<版本>_linux_<架构>.tar.gz
# 用法：bash scripts/package-linux.sh          # 版本从 Cargo.toml 读
#       APP_VERSION=0.2.0 bash scripts/package-linux.sh
# 需要先 cargo build --release -p rust-radio-app。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BIN="rust-radio"
ICON="$ROOT/crates/rust-radio-app/assets/app-icon-1024.png"

APP_VERSION="${APP_VERSION:-}"
if [ -z "$APP_VERSION" ]; then
    APP_VERSION="$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n1)"
fi
[ -n "$APP_VERSION" ] || { echo "无法从 Cargo.toml 读取版本" >&2; exit 1; }

ARCH="${ARCH:-$(uname -m)}"
case "$ARCH" in
    amd64) ARCH="x86_64" ;;
    arm64) ARCH="aarch64" ;;
esac

NAME="rust-radio_${APP_VERSION}_linux_${ARCH}"
STAGE="$ROOT/dist/$NAME"
OUT="$ROOT/dist/$NAME.tar.gz"

echo "==> 检查产物 target/release/$BIN"
[ -x "target/release/$BIN" ] || { echo "缺少 target/release/$BIN，先 cargo build --release -p rust-radio-app" >&2; exit 1; }

echo "==> 组装 $STAGE"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp "target/release/$BIN" "$STAGE/$BIN"
chmod 755 "$STAGE/$BIN"
cp README.md LICENSE "$STAGE/"
cp "$ICON" "$STAGE/rust-radio.png"

cat > "$STAGE/rust-radio.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=云听电台
Name[zh_CN]=云听电台
GenericName=网络电台
Comment=云听电台桌面客户端
Exec=rust-radio
Icon=rust-radio
Terminal=false
Categories=AudioVideo;Audio;Player;
Keywords=radio;电台;广播;
StartupWMClass=rust-radio
DESKTOP

echo "==> 压缩 $OUT"
rm -f "$OUT"
tar -czf "$OUT" -C "$ROOT/dist" "$NAME"

echo "==> 完成：$OUT"
tar -tzf "$OUT"