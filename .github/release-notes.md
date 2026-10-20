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

**macOS**：DMG 内的应用为 ad-hoc 签名、未做 Apple 公证，Gatekeeper 会拦。macOS 15 起右键「打开」已不能绕过，按下面两层处理（路径按实际下载位置调整，一般在 `~/Downloads`）：

1. 双击 DMG 就提示“Apple 无法验证‘rust-radio_{{VERSION}}_macos_universal.dmg’是否包含可能危害 Mac 安全或泄漏隐私的恶意软件”，镜像根本打不开——先去掉下载隔离标记：

   ```bash
   xattr -d com.apple.quarantine ~/Downloads/rust-radio_{{VERSION}}_macos_universal.dmg
   ```

   之后双击即可正常挂载，再把「云听电台」拖进 Applications。

2. 拖进 Applications 后首次启动仍被拦——对应用本体再执行一次：

   ```bash
   xattr -dr com.apple.quarantine "/Applications/云听电台.app"
   ```

3. 不想敲命令行：打开“系统设置 → 隐私与安全性”，下滚到“安全性”区域，点对应条目的“仍要打开”，输入密码确认，再回 Finder 双击一次。

以上操作只是撤掉 macOS 给下载文件打的隔离标记，不改变应用本身；升级后若从浏览器重新下载，需要重做第 1 步。

**Windows**：安装器默认按当前用户安装，可在向导里选择安装目录；卸载时会询问是否保留设置与收藏。

**Linux**：deb / rpm 会安装二进制、图标与 `.desktop` 菜单项；AppImage 为单文件，需要系统有 ALSA 与
X11 / Wayland 会话。

## 说明

- 二进制仅做 ad-hoc 签名（`codesign -s -`），没有 Apple Developer ID 签名、未做公证，因此会触发 Gatekeeper 拦截，杀毒软件也可能误报。
- 内置 libfdk-aac（经 `fdk-aac-sys`）为 Fraunhofer FDK 许可，非 OSI 许可，分发前请阅读其 NOTICE。
- 非官方客户端，与中央广播电视总台、云听无任何关联，仅供学习与个人使用。
