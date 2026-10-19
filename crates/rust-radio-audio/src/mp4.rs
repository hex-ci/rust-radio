//! 回听：HTTP Range + symphonia 的 MP4 解复用。
//!
//! 回听文件是 `.m4a`（AAC-LC，30 分钟级）。symphonia 只负责拆包，
//! AAC 仍交给 fdk 解码；seek 用它自己的采样索引，精确到帧。

use std::io::{self, Read, Seek, SeekFrom};

use symphonia::core::codecs::audio::well_known::CODEC_ID_AAC;
use symphonia::core::codecs::audio::AudioCodecParameters;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

const BLOCK: u64 = 256 * 1024;

/// 只读、可 seek 的 HTTP 源：按 Range 分块取，缓存最近一块。
struct HttpRangeSource {
    client: reqwest::blocking::Client,
    url: String,
    pos: u64,
    len: u64,
    cache_start: u64,
    cache: Vec<u8>,
}

impl HttpRangeSource {
    fn open(url: &str) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .connect_timeout(std::time::Duration::from_secs(8))
            .user_agent("Mozilla/5.0")
            .build()
            .map_err(|e| format!("创建网络客户端失败：{e}"))?;
        let len = probe_len(&client, url)?;
        Ok(Self {
            client,
            url: url.to_string(),
            pos: 0,
            len,
            cache_start: 0,
            cache: Vec::new(),
        })
    }

    fn fill(&mut self, at: u64) -> io::Result<()> {
        let end = (at + BLOCK - 1).min(self.len.saturating_sub(1));
        let resp = self
            .client
            .get(&self.url)
            .header(reqwest::header::RANGE, format!("bytes={at}-{end}"))
            .send()
            .map_err(|e| io::Error::other(format!("回听请求失败：{e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::PARTIAL_CONTENT {
            let bytes = resp.bytes().map_err(|e| io::Error::other(e.to_string()))?;
            self.cache = bytes.to_vec();
            self.cache_start = at;
            Ok(())
        } else if status.is_success() {
            // 服务端忽略了 Range，返回整文件：缓存从 0 开始
            let bytes = resp.bytes().map_err(|e| io::Error::other(e.to_string()))?;
            self.len = bytes.len() as u64;
            self.cache = bytes.to_vec();
            self.cache_start = 0;
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "回听请求失败（HTTP {}）",
                status.as_u16()
            )))
        }
    }
}

impl Read for HttpRangeSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.pos >= self.len {
            return Ok(0);
        }
        let cache_end = self.cache_start + self.cache.len() as u64;
        if self.pos < self.cache_start || self.pos >= cache_end {
            self.fill(self.pos)?;
        }
        let off = (self.pos - self.cache_start) as usize;
        if off >= self.cache.len() {
            return Ok(0);
        }
        let n = (self.cache.len() - off).min(out.len());
        out[..n].copy_from_slice(&self.cache[off..off + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HttpRangeSource {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let next = match from {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
            SeekFrom::End(d) => self.len as i128 + d as i128,
        };
        if next < 0 {
            return Err(io::Error::other("seek 越界"));
        }
        self.pos = (next as u64).min(self.len);
        Ok(self.pos)
    }
}

impl MediaSource for HttpRangeSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

fn probe_len(client: &reqwest::blocking::Client, url: &str) -> Result<u64, String> {
    // 有些 CDN 对 HEAD 返回 200 + Content-Length: 0，所以要校验长度非 0
    if let Ok(resp) = client.head(url).send() {
        if resp.status().is_success() {
            if let Some(n) = resp.content_length() {
                if n > 0 {
                    return Ok(n);
                }
            }
        }
    }
    // 有些 CDN 不支持 HEAD，用 0-0 的 Range 探长度
    let resp = client
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .map_err(|e| format!("回听探测失败：{e}"))?;
    if let Some(cr) = resp.headers().get(reqwest::header::CONTENT_RANGE) {
        if let Some(total) = cr.to_str().ok().and_then(|s| s.rsplit('/').next()) {
            if let Ok(n) = total.trim().parse::<u64>() {
                return Ok(n);
            }
        }
    }
    Err("回听文件长度未知".into())
}

pub struct M4aSource {
    format: Box<dyn FormatReader>,
    track_id: u32,
    asc: Vec<u8>,
}

impl M4aSource {
    pub fn open(url: &str, start_at: f64) -> Result<Self, String> {
        let src = HttpRangeSource::open(url)?;
        let mss = MediaSourceStream::new(Box::new(src), MediaSourceStreamOptions::default());
        let mut hint = Hint::new();
        hint.with_extension("m4a");
        let mut format = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| format!("回听文件解析失败：{e}"))?;

        let track = format
            .default_track(TrackType::Audio)
            .or_else(|| {
                format
                    .tracks()
                    .iter()
                    .find(|t| t.codec_params.as_ref().and_then(|p| p.audio()).is_some())
            })
            .ok_or_else(|| "回听文件里没有音轨".to_string())?;
        let params: &AudioCodecParameters = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| "回听音轨参数缺失".to_string())?;
        if params.codec != CODEC_ID_AAC {
            return Err(format!("回听音轨不是 AAC（{}）", params.codec));
        }
        let asc = params
            .extra_data
            .as_ref()
            .map(|d| d.to_vec())
            .ok_or_else(|| "回听音轨缺少 AudioSpecificConfig".to_string())?;
        let track_id = track.id;

        if start_at > 0.5 {
            let time = symphonia::core::units::Time::try_from_secs_f64(start_at)
                .unwrap_or(symphonia::core::units::Time::ZERO);
            format
                .seek(
                    SeekMode::Accurate,
                    SeekTo::Time {
                        time,
                        track_id: Some(track_id),
                    },
                )
                .map_err(|e| format!("回听定位失败：{e}"))?;
        }

        Ok(Self {
            format,
            track_id,
            asc,
        })
    }

    pub fn asc(&self) -> &[u8] {
        &self.asc
    }

    /// 下一包 AAC 裸帧；到结尾返回 `Ok(None)`。
    pub fn next_packet(&mut self) -> Result<Option<Vec<u8>>, String> {
        loop {
            match self.format.next_packet() {
                Ok(Some(p)) => {
                    if p.track_id == self.track_id {
                        return Ok(Some(p.data.to_vec()));
                    }
                }
                Ok(None) => return Ok(None),
                Err(SymError::ResetRequired) => return Ok(None),
                Err(e) => return Err(format!("回听读取失败：{e}")),
            }
        }
    }
}
