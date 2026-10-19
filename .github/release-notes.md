## 下载

| 平台 | 文件 |
| --- | --- |
| macOS（Apple Silicon / Intel 通用） | `rust-radio_{{VERSION}}_macos_universal.zip` |
| Windows x64 | `rust-radio_{{VERSION}}_windows_x86_64.zip` |
| Linux x64 | `rust-radio_{{VERSION}}_linux_x86_64.tar.gz` |

校验：`sha256sum -c SHA256SUMS.txt`（macOS 用 `shasum -a 256 -c SHA256SUMS.txt`）。

## 安装

**macOS**：解压后把「云听电台.app」拖进「应用程序」。包为 ad-hoc 签名、未做 Apple 公证，
首次打开若提示“已损坏”或“无法验证开发者”，执行：

```bash
xattr -dr com.apple.quarantine "/Applications/云听电台.app"
```

**Windows**：解压到任意目录，双击 `rust-radio.exe`。

**Linux**：

```bash
tar -xzf rust-radio_{{VERSION}}_linux_x86_64.tar.gz
cd rust-radio_{{VERSION}}_linux_x86_64
install -Dm755 rust-radio ~/.local/bin/rust-radio
install -Dm644 rust-radio.png ~/.local/share/icons/hicolor/1024x1024/apps/rust-radio.png
install -Dm644 rust-radio.desktop ~/.local/share/applications/rust-radio.desktop
```

运行时需要 ALSA（`libasound2`）与 X11 / Wayland 会话。

## 说明

- 二进制未做代码签名 / 公证，杀毒软件可能误报。
- 内置 libfdk-aac（经 `fdk-aac-sys`）为 Fraunhofer FDK 许可，非 OSI 许可，分发前请阅读其 NOTICE。
- 非官方客户端，与中央广播电视总台、云听无任何关联，仅供学习与个人使用。
