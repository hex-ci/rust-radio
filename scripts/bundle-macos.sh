#!/usr/bin/env bash
# 打包 macOS .app：cargo release 产物 + 图标 + Info.plist + ad-hoc 签名。
# 用法：bash scripts/bundle-macos.sh
#       APP_VERSION=0.2.0 bash scripts/bundle-macos.sh    # 指定版本（默认读 Cargo.toml）
#       SKIP_BUILD=1 bash scripts/bundle-macos.sh         # 复用已有 target/release/rust-radio
#                                                         # （CI 里先 lipo 出通用二进制再打包）
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

APP_NAME="云听电台"
BIN="rust-radio"
APP="$ROOT/dist/$APP_NAME.app"
BUILD="$ROOT/target/bundle"
ICONSET="$BUILD/AppIcon.iconset"

APP_VERSION="${APP_VERSION:-}"
if [ -z "$APP_VERSION" ]; then
    APP_VERSION="$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n1)"
fi
[ -n "$APP_VERSION" ] || { echo "无法从 Cargo.toml 读取版本" >&2; exit 1; }
# CFBundleVersion 只允许数字和点
APP_BUILD_VERSION="$(printf '%s' "$APP_VERSION" | tr -cd '0-9.')"
[ -n "$APP_BUILD_VERSION" ] || APP_BUILD_VERSION="1"

if [ "${SKIP_BUILD:-0}" = "1" ]; then
    echo "==> 跳过 cargo build（SKIP_BUILD=1）"
    [ -f "target/release/$BIN" ] || { echo "缺少 target/release/$BIN" >&2; exit 1; }
else
    echo "==> cargo build --release -p rust-radio-app"
    cargo build --release --locked -p rust-radio-app
fi

echo "==> 生成图标"
rm -rf "$BUILD"
mkdir -p "$ICONSET"
swift "$ROOT/scripts/make-icon.swift" "$BUILD/icon-1024.png" >/dev/null
for spec in "16 16x16" "32 16x16@2x" "32 32x32" "64 32x32@2x" "128 128x128" \
            "256 128x128@2x" "256 256x256" "512 256x256@2x" "512 512x512" "1024 512x512@2x"; do
    set -- $spec
    sips -z "$1" "$1" "$BUILD/icon-1024.png" --out "$ICONSET/icon_$2.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$BUILD/AppIcon.icns"

echo "==> 组装 $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/target/release/$BIN" "$APP/Contents/MacOS/$BIN"
cp "$BUILD/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
printf 'APPL????' > "$APP/Contents/PkgInfo"

# 让 AppKit 的标准菜单（关于、服务、隐藏…）按简体中文显示，而不是回退英文
mkdir -p "$APP/Contents/Resources/zh-Hans.lproj"
cat > "$APP/Contents/Resources/zh-Hans.lproj/InfoPlist.strings" <<STRINGS
CFBundleName = "云听电台";
CFBundleDisplayName = "云听电台";
STRINGS

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>云听电台</string>
    <key>CFBundleDisplayName</key><string>云听电台</string>
    <key>CFBundleExecutable</key><string>rust-radio</string>
    <key>CFBundleIdentifier</key><string>com.hex.rust-radio</string>
    <key>CFBundleDevelopmentRegion</key><string>zh_CN</string>
    <key>CFBundleLocalizations</key>
    <array>
        <string>zh-Hans</string>
        <string>en</string>
    </array>
    <key>LSHasLocalizedDisplayName</key><true/>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>${APP_VERSION}</string>
    <key>CFBundleVersion</key><string>${APP_BUILD_VERSION}</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>MIT License. 非官方客户端，仅供学习与个人使用。</string>
</dict>
</plist>
PLIST

echo "==> ad-hoc 签名"
codesign --force --sign - "$APP"

echo "==> 完成：${APP}（版本 ${APP_VERSION}）"
codesign -dv "$APP" 2>&1 | sed -n '1,4p'
du -sh "$APP"
