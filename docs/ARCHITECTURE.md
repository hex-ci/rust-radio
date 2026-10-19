# 架构与接口说明

这份文档记录**已经落地**的实现与关键决策。功能清单见 [README](../README.md)。

## 1. 接口能力

| 项 | 实测结果 |
|---|---|
| 签名 | `md5(参数名升序 k=v&… + timestamp + &key=分组密钥)`，参数值不做 URL 编码；4 组对拍向量进单测 |
| 省份 | 31 个（含「新疆兵团」） |
| 央广频率 | 19 个 |
| 全国索引 | 941 个台，33 个请求 / 6 并发，实测 0.8s |
| 电台详情 | 每台 4 条线路（AAC-HLS ×2、TS-HLS ×2） |
| 直播流 | `#EXTM3U`，5 段 ADTS 分片，**不加密**，`ACAO: *`，≈132 kbps HE-AAC v2 44.1k 立体声 |
| 节目单 | 中国之声（639）当天 23 档、昨天 23 档，均带回听地址 |
| 回听 .m4a | 返回 `206 + Content-Range` → **可任意拖动** |
| 直连 MP3 | 941 台里 756 台的 `mp3PlayUrlLow` 是真 `.mp3`（`audio/mpeg`，48 kbps） |
| 服务端搜索 | **已打通**：`/search/search/findSearchResourceList`，参数是 `keyWord`（大写 W）+ `searchTabId` / `contentType` / `pageNo` / `sortType`，**必须带 `productId: 1605403829833195520` 请求头**，缺了恒为 0 |

三条直接影响设计的事实：

1. **两种流两套路径**：132–195 kbps 的 HLS 与 48 kbps 的直连 MP3，所以选流做成四级梯子
   （`playUrlHigh → mp3PlayUrlHigh → playUrlLow → mp3PlayUrlLow`），失败自动降级并在播放条标注当前线路。
2. **播放地址是临时令牌**：旧链接现取 403，detail 现取的正常 → 收藏/历史只存 `contentId`。
3. **内容维度只有两个**：央广 19 + 31 省。没有分类、曲库、播客，所以浏览按「地理 + 台别 + 人气」三轴，
   导航能力由服务端搜索承担；本地目录只按 `contentId` 回查台别与在播节目。

## 2. 六条硬规则

1. 列表里**必须**显示 `subtitle`（「正在直播：xxx」）—— 这是用户选台的第一决策信息。
2. 收藏 / 历史 **绝不存播放地址**，只存 `contentId`。
3. 地址报错**必须重新调 detail**，不能只重试旧 URL（令牌会过期）。
4. 列表走缓存时，UI 明确标注「列表缓存 · X 分钟前」，不冒充实时。
5. 418 限流：退避后仍失败 → 明确文案，不静默重试。
6. HLS 起播从 playlist **倒数第 2–3 段**开始，不从最早段起，否则永远落后于直播。

## 3. 架构

```
rust-radio-core    接口层：签名、列表、详情、节目单、选流、磁盘缓存、电台目录索引
rust-radio-audio   音频引擎：内置解码（fdk-aac / symphonia）→ 重采样 → ringbuf → cpal 回调输出
rust-radio-app     egui 界面：主题、图标、图片缓存、各视图、播放条
```

**播放链路**：HLS（ADTS / MPEG-TS）或 MP4 → 解复用/分帧 → libfdk-aac 解码（HE-AAC v2）→
rubato 重采样到设备率 → `ringbuf` 环形缓冲 → `cpal` 输出回调。MP3 直连走 symphonia 解码。
直播与回听共用一条引擎，只区分 `StreamKind`（`Hls` / `Mp3` / `File`），
对外统一成 `Snapshot { state, position, duration, level }`。HLS 下载在独立线程预取 3 段，
解码线程只消费队列，避免网络抖动掏空 2 秒环形缓冲。

**状态机**：`Idle → Buffering → Playing ⇄ Paused`，异常进 `Reconnecting`（重新取地址，≤2 轮）→ `Error`。
直播的「暂停」是停止并记住电台，恢复时重新解析、从最新位置起播。

**落盘**：JSON + 原子替换（临时文件 rename），不用数据库。缓存分层：列表 24h、省份 30d、
历史节目单永久、台标按 `contentId` 存文件。

## 4. 关键取舍

| 决策 | 选择 | 原因 |
|---|---|---|
| 壳层 | **egui 0.37 自绘** | 无 WebView、无 Node、单 exe；项目不需要前端生态 |
| 解码 | **内置 fdk-aac + symphonia** | 云听主线路是 HE-AAC v2；`symphonia` 0.6 只支持 AAC-LC 并显式拒绝 SBR，纯 Rust 会掉到 22 kHz 单声道。AAC 交给静态编译的 libfdk-aac，MP4/MP3 用 symphonia，HLS/TS 自解析——无外部进程、单二进制 |
| 存储 | **JSON**，不用 SQLite | Windows GNU 工具链没有 C 编译器；数据量只有几百条，少一个 C 依赖 |
| 图标 | **Phosphor Icons 字体**（内嵌） | 手绘矢量图标小尺寸下发糊，且曾因共线多边形三角化出 NaN 顶点，画出一条穿屏斜线 |
| 字体 | 系统字体探测 + 真粗体家族 | egui 默认字体没有中文 |

## 5. 界面自检

手工定位的控件（工具条、播放条、面板头）靠算坐标摆放，容易互相压住。`--audit` 把每个
手工定位的控件与文本矩形登记下来（按裁剪区裁剪），帧末两两求交，部分重叠即报告；
「文字被控件包含」豁免（按钮里的标签是设计如此），「两个控件互相包含」判为 bug。

自检自带**阳性对照**：故意登记两个重叠矩形，必须被抓到——否则「0 处重叠」可能只是检测器没工作。
轮换 7 个页面 + 节目单开关 + 搜索浮层 + 真实播放态，单帧约 138 个矩形。

## 6. 范围边界

录音 / 转存 / 重新分发音频、DLNA 推流、账号登录、点播曲库、歌词、社交互动、后台大规模抓取。
这些不是「还没做」，是刻意不做：它们要么越界（转存与分发），要么与「听直播 + 回听」的定位无关。

## 7. 已知限制

- **FDK AAC 许可**：`fdk-aac-sys` 以 MIT 发布，但 libfdk-aac 本身是 Fraunhofer FDK 许可（非 OSI）；
  个人使用无碍，对外分发需附 NOTICE，AAC 专利授权需自行评估。
- **直播暂停恢复需要重新解析地址**，会有 1–2 秒重新缓冲。
- **Windows / Linux 未实机验证**：代码无平台特定 API；libfdk-aac 由 `cc` 编译 C++，Windows 需要 C++ 构建工具。
- 未做：系统托盘、系统媒体键（SMTC/MPRIS）、全局快捷键、回听倍速、开机自启、收藏导入导出。
