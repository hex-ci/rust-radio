## 下载

| 平台 | 文件 | 安装 |
| --- | --- | --- |
| macOS（Apple Silicon / Intel 通用） | `rust-radio_{{VERSION}}_macos_universal.dmg` | 打开 DMG，把「云听电台」拖进 Applications |
| Windows x64 | `rust-radio_{{VERSION}}_windows_x64_setup.exe` | 双击运行 NSIS 安装器 |
| Linux x64（Debian / Ubuntu） | `rust-radio_{{VERSION}}_linux_amd64.deb` | `sudo apt install ./rust-radio_{{VERSION}}_linux_amd64.deb` |
| Linux x64（通用） | `rust-radio_{{VERSION}}_linux_amd64.AppImage` | `chmod +x` 后直接运行 |
| Linux x64（Fedora / RHEL） | `rust-radio_{{VERSION}}_linux_amd64.rpm` | `sudo dnf install ./rust-radio-*.rpm` |

校验：`sha256sum -c SHA256SUMS.txt`（macOS 用 `shasum -a 256 -c SHA256SUMS.txt`）。

## 安装说明

**macOS**：DMG 内为 ad-hoc 签名、未做 Apple 公证，首次打开若提示“已损坏”或“无法验证开发者”，执行：

```bash
xattr -dr com.apple.quarantine "/Applications/云听电台.app"
```

**Windows**：安装器默认按当前用户安装，可在向导里选择安装目录；卸载时会询问是否保留设置与收藏。

**Linux**：deb / rpm 会安装二进制、图标与 `.desktop` 菜单项；AppImage 为单文件，需要系统有 ALSA 与
X11 / Wayland 会话。

## 说明

- 二进制未做代码签名 / 公证，杀毒软件可能误报。
- 内置 libfdk-aac（经 `fdk-aac-sys`）为 Fraunhofer FDK 许可，非 OSI 许可，分发前请阅读其 NOTICE。
- 非官方客户端，与中央广播电视总台、云听无任何关联，仅供学习与个人使用。
