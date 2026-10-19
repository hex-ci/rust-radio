# 云听电台

非官方云听电台桌面客户端。`rust-radio-core` 是签名 HTTP 客户端 + 磁盘缓存 + 电台目录索引，
`rust-radio-audio` 用内置解码器（libfdk-aac + symphonia）解出 PCM，经重采样和 ringbuf 送进 cpal，
`rust-radio-app` 是 egui 0.37 / eframe 自绘界面。注释与 UI 文案用中文，标识符用英文。

## 结构

```
crates/rust-radio-core/   sign / client / models / cache / search（单测都在这里）
crates/rust-radio-audio/  Engine 状态机、音量、seek；adts/hls/ts/mp4/mp3/decode/resample；examples/check.rs
crates/rust-radio-app/    main.rs 参数解析；app.rs 全部视图；theme/widgets/images/icons/audit
docs/ARCHITECTURE.md      接口实测结论、六条硬规则、架构取舍——改接口层前先读
.docs/SEARCH-API.md       服务端搜索参数（keyWord 大写 W，必须带 productId 头）
```

## 环境

- 稳定版 Rust，edition 2021；MSRV 1.87（symphonia 0.6 / rubato 0.16 的要求）。
- AAC 由 `fdk-aac-sys` 静态编译 libfdk-aac（需要 C++ 工具链；Windows 用 MSVC，
  MinGW GCC 16 下 fdk-aac-sys 0.5.0 的 `fMax(SHORT, SHORT)` 重定义会编译失败），
  MP3/MP4 由 symphonia，HLS/ADTS/MPEG-TS 在 `rust-radio-audio` 内自行处理。
- 运行数据（`store.json`、列表缓存、台标）写在平台数据目录；用 `RUST_RADIO_DATA_DIR=/tmp/...` 隔离测试，避免污染本机收藏与进度。

## 构建与测试

```bash
cargo check --workspace                     # 当前 0 error / 0 warning
cargo test --workspace                      # 20 个单测：audio 13、core 5、client 2
cargo test -p rust-radio-core sign::        # 按模块过滤
cargo run -p rust-radio-app                 # 启动 GUI
cargo run -p rust-radio-app -- --selftest   # 端到端：接口/索引/搜索/直播出声/回听拖动，需网络+声卡
cargo run -p rust-radio-app -- --check-live --station 639  # 回听→回直播→暂停/继续，断言状态；不带 --station 会一直等播放
cargo run -p rust-radio-app -- --audit      # 轮换 7 个页面查控件重叠，约 40s 后自动退出
cargo run -p rust-radio-audio --example check -- 639   # 单台播放链路，8s 后 PASS/FAIL
cargo build --release -p rust-radio-app     # 产物 target/release/rust-radio
bash scripts/bundle-macos.sh                # 产物 dist/云听电台.app（图标 + ad-hoc 签名）
```

GitHub Actions：`.github/workflows/ci.yml` 在 macOS / Ubuntu / Windows 三平台跑 fmt / clippy / test 并对打包脚本做冒烟；
`.github/workflows/release.yml` 负责发版——推 `v<版本>` tag（必须等于 `[workspace.package] version`）后构建
macOS 通用包（`lipo` 双架构）/ Windows zip / Linux tar.gz，跑测试，创建 GitHub Release 并上传 `SHA256SUMS.txt`。
手工打包脚本：`scripts/bundle-macos.sh`、`scripts/package-linux.sh`、`scripts/package-windows.ps1`。
测试是模块内 `#[cfg(test)] mod tests`，不放 `tests/`。

## 约定

- 库层错误用 `thiserror` 枚举 + 中文 `Display`；`core` 对外的 `ApiError::ui_hint()` 提供用户文案；app 边界用 `anyhow`。
- `core` 用阻塞式 `reqwest`：绝不能在 UI 线程调 `Client`，走 `App::spawn` + `mpsc::Sender<Msg>`，在 `poll()` 里消费。
- 网络/缓存模型 `#[serde(rename_all = "camelCase")]`：磁盘缓存键名与旧实现共用，不要改字段名。
- UI 全部手算坐标绘制。颜色/圆角/尺寸取自 `theme::*`，文字用 `theme::regular(size)` / `theme::bold(size)`；新视图是 `app.rs` 里的 `App::view_*` 方法，通用绘制原语放 `widgets.rs`。
- 窗口无系统标题栏：macOS 用透明标题栏保留原生红绿灯，Windows/Linux 自绘窗口按钮；空白区拖动走 `window_drag()`（标题栏/工具条），别把整页都做成拖拽区。
- 手工定位的控件必须 `audit::ctl("name", rect, clip)` 登记，文字用 `audit::text(...)` 而不是 `painter.text(...)`，否则 `--audit` 查不到它们。
- 搜索只走服务端：结果页是 `App::view_search`（`View::Search`），`Client::search_stations` 的 `pageNo` 是 0 基；服务端条目按 `contentId` 用 `StationIndex::get` 补台别/省份/在播节目，本地不做关键词匹配。

## 领域硬规则（ARCHITECTURE.md 第 2 节，改动时不得违反）

1. 列表行必须显示 `subtitle`（「正在直播：xxx」）。
2. 收藏 / 最近播放只存 `content_id`，绝不落播放地址。
3. 播放地址是临时令牌：报错必须重新调 detail 取新地址，重试旧 URL 只会 403。
4. 走缓存时必须标出缓存年龄（`Freshness`），不冒充实时。
5. HTTP 418 限流：退避后仍失败就给明确文案，不静默重试。
6. HLS 直播从 playlist 倒数第 2–3 段起播，不能从最早段开始。

## 手工验收开关

`--view discover|national|all|region|favorites|recent|settings`、`--station <id>`、`--search=关键词`、
`--list`、`--replay`、`--mini`、`--size=WxH`、`--icon-sheet`；`RUST_RADIO_NOICONS=1` 关图标，便于截图对比。

## 坑

- 代码已统一 `cargo fmt`；提交前跑 `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`。
- 代码已清到 0 warning（egui 0.37 的 `Ui::scope_id` / `Id::unique` 已替换）。新增代码请保持无 warning。
- 音频链路不要重新引入外部进程；`AdtsFramer::frames_from` 必须分块喂（一次 push 几十 KB 会触发内部截断，丢帧）。
- 回听 CDN 对 HEAD 返回 `Content-Length: 0`，`mp4::probe_len` 会退到 Range 探测，别把 0 当有效长度。
- 应用图标由 `scripts/make-icon.swift` 生成；`crates/rust-radio-app/assets/app-icon-1024.png` 是运行时内嵌副本，改图标要同时重新生成这两处（`bash scripts/bundle-macos.sh` 只更新 `.app`）。
- `--selftest` 会真的打开声卡并以 0.35 音量放音，失败返回非 0；`--audit` / `--check-live` 要开窗，不能当 CI 用。
- GUI 模式阻塞到窗口关闭；自动化截图用上面的 `--view` / `--search` / `--size` 组合。
- Windows release 构建带 `windows_subsystem = "windows"`（`main.rs` 首行），双击不会弹控制台；
  代价是 release exe 里 `--selftest` / `--check-live` 的 stdout 看不到，要跑这类诊断用 `cargo run`（debug 仍带控制台）。
- 仓库已有 `.git`；内部笔记/逆向资料放在 `.docs/`，整目录不入库。
