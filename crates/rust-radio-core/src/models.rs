//! 归一化后的数据模型。网络字段名与 UI 模型分开：接口字段会变，UI 不跟着变。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Quality {
    High,
    #[default]
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Line {
    #[default]
    Auto,
    /// 主线路（`playUrl*`，AAC/ADTS）
    Main,
    /// 备用线路（`mp3PlayUrl*`，可能是 TS 或真 MP3）
    Alt,
}

/// 播放源的容器形态，决定播放器怎么喂数据。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamKind {
    /// HLS（m3u8，直播）
    Hls,
    /// 无限 MP3 流
    Mp3,
    /// 可 seek 的整段文件（回听 m4a）
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamTarget {
    pub url: String,
    pub kind: StreamKind,
    pub quality: Quality,
    pub line: Line,
    /// 提供给 UI 展示的线路名，如「高清 AAC」
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Province {
    #[serde(rename = "provinceName", default)]
    pub name: String,
    #[serde(rename = "provinceCode", default)]
    pub code: u32,
}

/// 列表接口归一化条目。**磁盘缓存键名固定 camelCase**，与参考实现共用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Station {
    pub content_id: String,
    pub title: String,
    pub tag_name: String,
    pub subtitle: String,
    pub image: String,
    #[serde(default)]
    pub province_code: u32,
}

impl Station {
    /// 「正在直播：新闻进行时」→「新闻进行时」
    pub fn now_playing(&self) -> &str {
        self.subtitle
            .strip_prefix("正在直播：")
            .or_else(|| self.subtitle.strip_prefix("正在直播:"))
            .unwrap_or(&self.subtitle)
    }

    pub fn initial(&self) -> String {
        self.title
            .chars()
            .next()
            .map(String::from)
            .unwrap_or_else(|| "台".into())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StationDetail {
    pub id: String,
    pub name: String,
    #[serde(deserialize_with = "de_bool")]
    pub is_living: bool,
    pub living_program_name: Option<String>,
    pub play_url_low: Option<String>,
    pub play_url_high: Option<String>,
    pub mp3_play_url_low: Option<String>,
    pub mp3_play_url_high: Option<String>,
    pub listen_back_flag: Option<i32>,
    pub listen_count: Option<i64>,
    pub image: Option<String>,
    pub province: Option<String>,
    pub des: Option<String>,
    pub play_background_img: Option<String>,
    pub home_background_img: Option<String>,
}

fn de_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Bool(b) => Ok(b),
        serde_json::Value::Number(n) => Ok(n.as_i64().unwrap_or(0) != 0),
        serde_json::Value::String(s) => Ok(s == "1" || s.eq_ignore_ascii_case("true")),
        serde_json::Value::Null => Ok(false),
        _ => Err(D::Error::custom("isLiving 既不是 bool 也不是 0/1")),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Program {
    pub id: String,
    pub broadcast_id: String,
    pub program_name: String,
    pub start_time: i64,
    pub end_time: i64,
    pub play_url_low: Option<String>,
    pub play_url_high: Option<String>,
    pub download_url: Option<String>,
    pub image: Option<String>,
    pub play_flag: Option<i32>,
}

impl Program {
    /// 回听地址：高清 → 低清 → 下载
    pub fn replay_url(&self) -> Option<&str> {
        self.play_url_high
            .as_deref()
            .or(self.play_url_low.as_deref())
            .or(self.download_url.as_deref())
    }

    pub fn duration_secs(&self) -> i64 {
        ((self.end_time - self.start_time) / 1000).max(0)
    }

    pub fn start_local(&self) -> chrono::DateTime<chrono::Local> {
        chrono::DateTime::from_timestamp_millis(self.start_time)
            .map(|d| d.with_timezone(&chrono::Local))
            .unwrap_or_else(chrono::Local::now)
    }

    pub fn end_local(&self) -> chrono::DateTime<chrono::Local> {
        chrono::DateTime::from_timestamp_millis(self.end_time)
            .map(|d| d.with_timezone(&chrono::Local))
            .unwrap_or_else(chrono::Local::now)
    }

    /// 是否正在播（按本地时钟判断）
    pub fn is_live_now(&self) -> bool {
        let now = chrono::Local::now().timestamp_millis();
        self.start_time <= now && now < self.end_time
    }
}
