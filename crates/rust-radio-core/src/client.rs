//! 网关客户端：签名、请求头、418 退避、备用域名、缓存与选流。

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::cache::Cache;
use crate::models::{
    Line, Program, Province, Quality, Station, StationDetail, StreamKind, StreamTarget,
};
use crate::sign;

pub const GATEWAY: &str = "https://ytmsout.radio.cn";
pub const GATEWAY_BACKUP: &str = "https://ytmsotherout.radio.cn";

/// 云听 App 的产品 id（`com.linker.xlyt.constant.HttpClentLinkNet.MAIN_PRODUCTID`）。
/// 搜索接口必须带这个请求头，缺了恒返回 `totalNum: 0`。
pub const PRODUCT_ID: &str = "1605403829833195520";
/// 搜索只搜电台：tab=4 / contentType=5（见 docs/SEARCH-API.md）。
const SEARCH_TAB_RADIO: &str = "4";
const SEARCH_TYPE_RADIO: &str = "5";

const TTL_STATIONS: Duration = Duration::from_secs(24 * 3600);
const TTL_PROVINCES: Duration = Duration::from_secs(30 * 24 * 3600);
const TTL_PROGRAMS_TODAY: Duration = Duration::from_secs(3600);

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("接口限流（HTTP 418），请稍后再试")]
    RateLimited,
    #[error("网络不可用：{0}")]
    Network(String),
    #[error("接口返回 HTTP {0}")]
    Http(u16),
    #[error("接口返回错误 code={code}：{message}")]
    Api { code: i64, message: String },
    #[error("数据解析失败：{0}")]
    Parse(String),
}

impl ApiError {
    /// 是否值得重试（网络/限流）
    pub fn retryable(&self) -> bool {
        matches!(self, ApiError::Network(_) | ApiError::RateLimited)
    }

    pub fn ui_hint(&self) -> &'static str {
        match self {
            ApiError::RateLimited => "云听接口限流，请 2 分钟后再试",
            ApiError::Network(_) => "网络不可用，检查连接后重试",
            ApiError::Http(_) => "接口暂不可用，可能已变更",
            ApiError::Api { .. } => "接口返回错误，可能已变更",
            ApiError::Parse(_) => "数据格式变化，请更新客户端",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Live,
    Cached(Duration),
}

impl Freshness {
    pub fn minutes(&self) -> u64 {
        match self {
            Freshness::Live => 0,
            Freshness::Cached(d) => d.as_secs() / 60,
        }
    }
}

pub struct Client {
    http: reqwest::blocking::Client,
    cache: Cache,
    /// 上一次是否走了备用域名
    pub used_backup: std::sync::atomic::AtomicBool,
}

impl Client {
    pub fn new(cache_dir: impl Into<std::path::PathBuf>) -> anyhow::Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(12))
            .connect_timeout(Duration::from_secs(6))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) YuntingRadio/0.1")
            .build()?;
        Ok(Self {
            http,
            cache: Cache::new(cache_dir),
            used_backup: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    fn now_ms() -> String {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
            .to_string()
    }

    /// 原始 GET：自动签名、418 退避、备用域名、`code != 0` 转错。
    fn get(&self, path: &str, params: &[(&str, String)]) -> Result<Value, ApiError> {
        let mut last: Option<ApiError> = None;
        for gateway in [GATEWAY, GATEWAY_BACKUP] {
            for attempt in 0..2u32 {
                let timestamp = Self::now_ms();
                let refs: Vec<(&str, &str)> =
                    params.iter().map(|(k, v)| (*k, v.as_str())).collect();
                let signature = sign::sign(path, &refs, &timestamp);
                // reqwest 0.13 移除了 RequestBuilder::query，改用 Url 构造查询串（编码仍然正确）
                let mut url = match reqwest::Url::parse(&format!("{gateway}{path}")) {
                    Ok(u) => u,
                    Err(e) => {
                        last = Some(ApiError::Network(format!("URL 构造失败：{e}")));
                        break;
                    }
                };
                if !params.is_empty() {
                    let mut pairs = url.query_pairs_mut();
                    for (k, v) in params {
                        pairs.append_pair(k, v);
                    }
                }
                let req = self
                    .http
                    .get(url)
                    .header("Content-Type", "application/json")
                    .header("equipmentId", "0000")
                    .header("platformCode", "ANDROID")
                    .header("equipmentSource", "ANDROID")
                    .header("productId", PRODUCT_ID)
                    .header("timestamp", timestamp.clone())
                    .header("sign", signature);

                match req.send() {
                    Ok(resp) => {
                        let status = resp.status().as_u16();
                        if status == 418 {
                            last = Some(ApiError::RateLimited);
                            std::thread::sleep(Duration::from_secs(2 * (attempt as u64 + 1)));
                            continue;
                        }
                        if !(200..300).contains(&status) {
                            last = Some(ApiError::Http(status));
                            break; // 换备用域名试
                        }
                        let body: Value = resp
                            .json()
                            .map_err(|e| ApiError::Parse(format!("响应不是合法 JSON：{e}")))?;
                        let code = body.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
                        if code != 0 {
                            let message = body
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("未知错误")
                                .to_string();
                            last = Some(ApiError::Api { code, message });
                            break;
                        }
                        if gateway == GATEWAY_BACKUP {
                            self.used_backup
                                .store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        return Ok(body);
                    }
                    Err(e) => {
                        last = Some(ApiError::Network(e.to_string()));
                        std::thread::sleep(Duration::from_millis(300));
                    }
                }
            }
        }
        Err(last.unwrap_or(ApiError::Network("请求失败".into())))
    }

    fn decode<T: DeserializeOwned>(body: &Value) -> Result<T, ApiError> {
        let data = body.get("data").cloned().unwrap_or(Value::Null);
        if data.is_null() {
            // 接口对空结果返回 data:null，按空集合处理
            let empty: Value = Value::Array(vec![]);
            return serde_json::from_value(empty).map_err(|e| ApiError::Parse(e.to_string()));
        }
        serde_json::from_value(data).map_err(|e| ApiError::Parse(e.to_string()))
    }

    // ---------------------------------------------------------------- 列表

    pub fn provinces(&self) -> Result<Vec<Province>, ApiError> {
        if let Some(cached) = self.cache.read("provinces.json", TTL_PROVINCES) {
            return Ok(cached);
        }
        let body = self.get("/format/appProvince/list/all", &[])?;
        let list: Vec<Province> = Self::decode(&body)?;
        let _ = self.cache.write("provinces.json", &list);
        Ok(list)
    }

    /// 央广 19 个频率
    pub fn national(&self) -> Result<Vec<Station>, ApiError> {
        let cached: Option<Vec<Station>> = self.cache.read("national.json", TTL_STATIONS);
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let body = self.get("/contentBiz/appBroadcast/all/group/1", &[])?;
        let rows = body.get("data").cloned().unwrap_or(Value::Array(vec![]));
        let list = normalize_list(&rows);
        let _ = self.cache.write("national.json", &list);
        Ok(list)
    }

    pub fn by_province(&self, code: u32) -> Result<Vec<Station>, ApiError> {
        let key = format!("province-{code}.json");
        if let Some(cached) = self.cache.read(&key, TTL_STATIONS) {
            return Ok(cached);
        }
        let body = self.get(
            "/contentBiz/appBroadcast/allByProvince",
            &[("provinceCode", code.to_string())],
        )?;
        let rows = body.get("data").cloned().unwrap_or(Value::Array(vec![]));
        let mut list = normalize_list(&rows);
        for s in &mut list {
            if s.province_code == 0 {
                s.province_code = code;
            }
        }
        let _ = self.cache.write(&key, &list);
        Ok(list)
    }

    /// 服务端搜索（只搜电台）：`searchTabId=4` + `contentType=5`，`page` 从 0 开始。
    ///
    /// `page_size` 会作为 `pageSize` 参数发送：实测可用范围是 **21–100**，
    /// 小于 21（例如 20）会被服务端忽略并退回 100。响应缺 `tagName` / `provinceCode`，
    /// `subtitle` 多为「暂无节目单」，调用方应按 `contentId` 回查本地索引补全。
    pub fn search_stations(
        &self,
        keyword: &str,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<Station>, u64), ApiError> {
        let body = self.get(
            "/search/search/findSearchResourceList",
            &[
                ("keyWord", keyword.to_string()),
                ("searchTabId", SEARCH_TAB_RADIO.to_string()),
                ("contentType", SEARCH_TYPE_RADIO.to_string()),
                ("pageNo", page.to_string()),
                ("pageSize", page_size.to_string()),
                ("sortType", "0".to_string()),
            ],
        )?;
        let rows = body
            .get("data")
            .and_then(|d| d.get("data"))
            .cloned()
            .unwrap_or(Value::Array(vec![]));
        let total = body
            .get("data")
            .and_then(|d| d.get("totalNum"))
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let mut list = normalize_list(&rows);
        for s in &mut list {
            if s.tag_name.is_empty() {
                s.tag_name = "电台".into();
            }
        }
        Ok((list, total))
    }

    /// 全国索引：央广 + 31 省合并去重（6 并发），带 24 小时缓存。
    pub fn all_stations(&self, refresh: bool) -> Result<(Vec<Station>, Freshness), ApiError> {
        if !refresh {
            if let Some((list, age)) = self.cache.read_stale::<Vec<Station>>("stations.json") {
                if age <= TTL_STATIONS {
                    return Ok((list, Freshness::Cached(age)));
                }
            }
        }

        let provinces = self.provinces()?;
        let mut merged: BTreeMap<String, Station> = self
            .national()?
            .into_iter()
            .map(|s| (s.content_id.clone(), s))
            .collect();

        let codes: Vec<u32> = provinces.iter().map(|p| p.code).collect();
        let chunk = codes.len().div_ceil(6);
        let fetched: Vec<Station> = std::thread::scope(|scope| {
            let handles: Vec<_> = codes
                .chunks(chunk.max(1))
                .map(|part| {
                    let this = &*self;
                    scope.spawn(move || {
                        let mut out = Vec::new();
                        for code in part {
                            if let Ok(mut list) = this.by_province(*code) {
                                out.append(&mut list);
                            }
                        }
                        out
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect()
        });

        if fetched.is_empty() && merged.is_empty() {
            return Err(ApiError::Network("全国索引构建失败".into()));
        }
        for s in fetched {
            merged.entry(s.content_id.clone()).or_insert(s);
        }
        let list: Vec<Station> = merged.into_values().collect();
        let _ = self.cache.write("stations.json", &list);
        Ok((list, Freshness::Live))
    }

    // ---------------------------------------------------------------- 详情

    /// 详情：唯一给高清地址的接口，**每次现取**（地址是临时令牌）。
    pub fn detail(&self, content_id: &str) -> Result<StationDetail, ApiError> {
        let body = self.get(
            &format!("/contentBiz/appBroadcast/detail/{content_id}"),
            &[],
        )?;
        let data = body
            .get("data")
            .cloned()
            .filter(|d| !d.is_null())
            .ok_or_else(|| ApiError::Parse("详情为空".into()))?;
        serde_json::from_value(data).map_err(|e| ApiError::Parse(e.to_string()))
    }

    // ---------------------------------------------------------------- 节目单

    pub fn programs(
        &self,
        content_id: &str,
        date: &str,
    ) -> Result<(Vec<Program>, Freshness), ApiError> {
        let key = format!("programs/{content_id}-{date}.json");
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let ttl = if date >= today.as_str() {
            TTL_PROGRAMS_TODAY
        } else {
            Duration::from_secs(365 * 24 * 3600)
        };

        if let Some(cached) = self.cache.read::<Vec<Program>>(&key, ttl) {
            let age = self
                .cache
                .read_stale::<Vec<Program>>(&key)
                .map(|(_, a)| a)
                .unwrap_or_default();
            return Ok((cached, Freshness::Cached(age)));
        }

        let body = self.get(
            "/contentBiz/v1/appProgram/listByDate",
            &[
                ("date", date.to_string()),
                ("broadcastId", content_id.to_string()),
            ],
        )?;
        let rows = body.get("data").cloned().unwrap_or(Value::Array(vec![]));
        let list: Vec<Program> = serde_json::from_value(rows).unwrap_or_default();
        let _ = self.cache.write(&key, &list);
        Ok((list, Freshness::Live))
    }

    /// 有缓存先给缓存（历史日期永久有效），拿不到网络也不报错。
    pub fn programs_cached(&self, content_id: &str, date: &str) -> Option<Vec<Program>> {
        self.cache
            .read_stale::<Vec<Program>>(&format!("programs/{content_id}-{date}.json"))
            .map(|(v, _)| v)
    }

    // ---------------------------------------------------------------- 选流

    /// 选流梯子：高清优先 → 备用 → 标清。返回全部候选，播放器失败时逐档下移。
    pub fn stream_candidates(
        &self,
        detail: &StationDetail,
        quality: Quality,
        line: Line,
    ) -> Vec<StreamTarget> {
        let hi: [(&Option<String>, &str, Line); 2] = [
            (&detail.play_url_high, "高清 AAC", Line::Main),
            (&detail.mp3_play_url_high, "高清 备用", Line::Alt),
        ];
        let lo: [(&Option<String>, &str, Line); 2] = [
            (&detail.play_url_low, "标清 AAC", Line::Main),
            (&detail.mp3_play_url_low, "标清 备用", Line::Alt),
        ];

        let ordered: Vec<(&Option<String>, &str, Line)> = match line {
            Line::Main => vec![hi[0], lo[0], hi[1], lo[1]],
            Line::Alt => vec![hi[1], lo[1], hi[0], lo[0]],
            Line::Auto => match quality {
                Quality::High => vec![hi[0], hi[1], lo[0], lo[1]],
                Quality::Low => vec![lo[0], lo[1], hi[0], hi[1]],
            },
        };

        ordered
            .into_iter()
            .filter_map(|(url, label, ln)| {
                let url = url.as_deref()?.trim();
                if url.is_empty() {
                    return None;
                }
                let kind = kind_of(url);
                let q = if label.starts_with("高清") {
                    Quality::High
                } else {
                    Quality::Low
                };
                let label = match (kind, label) {
                    (StreamKind::Mp3, _) => "MP3 直连 48k",
                    (StreamKind::File, _) => "回听文件",
                    _ => label,
                };
                Some(StreamTarget {
                    url: url.to_string(),
                    kind,
                    quality: q,
                    line: ln,
                    label: label.to_string(),
                })
            })
            .collect()
    }

    /// 现取播放地址（每次都调 detail，绝不落盘）。
    pub fn fresh_stream(
        &self,
        content_id: &str,
        quality: Quality,
        line: Line,
    ) -> Result<(StationDetail, StreamTarget), ApiError> {
        let detail = self.detail(content_id)?;
        let target = self
            .stream_candidates(&detail, quality, line)
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::Parse("该电台没有可用播放地址".into()))?;
        Ok((detail, target))
    }
}

pub fn kind_of(url: &str) -> StreamKind {
    let lower = url.to_lowercase();
    if lower.contains(".m3u8") {
        StreamKind::Hls
    } else if lower.contains(".m4a") || lower.contains(".mp4") {
        StreamKind::File
    } else if lower.contains(".mp3") {
        StreamKind::Mp3
    } else {
        StreamKind::Hls
    }
}

fn normalize_list(rows: &Value) -> Vec<Station> {
    let Value::Array(rows) = rows else {
        return Vec::new();
    };
    rows.iter().filter_map(normalize_one).collect()
}

fn normalize_one(row: &Value) -> Option<Station> {
    let pick = |key: &str| -> String {
        match row.get(key) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        }
    };
    let content_id = {
        let a = pick("contentId");
        if a.is_empty() {
            pick("id")
        } else {
            a
        }
    };
    if content_id.is_empty() {
        return None;
    }
    let title = {
        let a = pick("title");
        if a.is_empty() {
            pick("name")
        } else {
            a
        }
    };
    Some(Station {
        content_id,
        title,
        tag_name: pick("tagName"),
        subtitle: pick("subtitle"),
        image: pick("image"),
        province_code: pick("provinceCode").parse().unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_url_to_kind() {
        assert_eq!(kind_of("http://x/y/index.m3u8?key=1"), StreamKind::Hls);
        assert_eq!(kind_of("https://x/stream_1.mp3?type=1"), StreamKind::Mp3);
        assert_eq!(kind_of("https://x/echo/2/1.m4a?e=0"), StreamKind::File);
    }

    #[test]
    fn builds_quality_ladder_in_order() {
        let client = Client::new(std::env::temp_dir().join("yt-test")).unwrap();
        let d = StationDetail {
            play_url_high: Some("h.m3u8".into()),
            play_url_low: Some("l.m3u8".into()),
            mp3_play_url_high: Some("alt.mp3".into()),
            mp3_play_url_low: Some("alt-low.m3u8".into()),
            ..Default::default()
        };
        let hi = client.stream_candidates(&d, Quality::High, Line::Auto);
        assert_eq!(hi[0].url, "h.m3u8");
        assert_eq!(hi[1].url, "alt.mp3");
        assert_eq!(hi[1].kind, StreamKind::Mp3);
        let alt = client.stream_candidates(&d, Quality::High, Line::Alt);
        assert_eq!(alt[0].url, "alt.mp3");
    }
}
