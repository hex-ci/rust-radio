//! 直连 MP3 兜底（云听目前四条线路都是 HLS，这里保留旧 `StreamKind::Mp3` 分支）。
//!
//! symphonia 的 MP3 解码器是纯 Rust 实现，输出 f32；不参与 AAC 的 fdk 管线。

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Mutex;

use symphonia::core::codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

struct StreamingSource(Mutex<Box<dyn Read + Send>>);

impl Read for StreamingSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.0.lock().unwrap().read(out)
    }
}

impl Seek for StreamingSource {
    fn seek(&mut self, _from: SeekFrom) -> io::Result<u64> {
        Err(io::Error::other("MP3 直连流不可 seek"))
    }
}

impl MediaSource for StreamingSource {
    fn is_seekable(&self) -> bool {
        false
    }
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

pub struct Mp3Source {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    rate: u32,
    channels: usize,
    buf: Vec<f32>,
}

impl Mp3Source {
    pub fn open(url: &str) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .connect_timeout(std::time::Duration::from_secs(5))
            .user_agent("Mozilla/5.0")
            .build()
            .map_err(|e| format!("创建网络客户端失败：{e}"))?;
        let resp = client
            .get(url)
            .send()
            .map_err(|e| format!("MP3 请求失败：{e}"))?;
        if !resp.status().is_success() {
            return Err(format!("MP3 请求失败（HTTP {}）", resp.status().as_u16()));
        }
        let mss = MediaSourceStream::new(
            Box::new(StreamingSource(Mutex::new(Box::new(resp)))),
            MediaSourceStreamOptions::default(),
        );
        let mut hint = Hint::new();
        hint.with_extension("mp3");
        let format = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| format!("MP3 流解析失败：{e}"))?;
        let (track_id, params) = {
            let track = format
                .default_track(TrackType::Audio)
                .ok_or_else(|| "MP3 流里没有音轨".to_string())?;
            let params: &AudioCodecParameters = track
                .codec_params
                .as_ref()
                .and_then(|p| p.audio())
                .ok_or_else(|| "MP3 音轨参数缺失".to_string())?;
            (track.id, params.clone())
        };
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|e| format!("MP3 解码器初始化失败：{e}"))?;
        let rate = params.sample_rate.unwrap_or(48_000);
        let channels = params.channels.as_ref().map(|c| c.count()).unwrap_or(2);
        Ok(Self {
            format,
            decoder,
            track_id,
            rate,
            channels,
            buf: Vec::new(),
        })
    }

    /// 下一段 PCM（交错 f32）；流结束返回 `Ok(None)`。
    pub fn next_pcm(&mut self) -> Result<Option<(Vec<f32>, u32, usize)>, String> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(None),
                Err(SymError::ResetRequired) => return Ok(None),
                Err(e) => return Err(format!("MP3 读取失败：{e}")),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let decoded = self
                .decoder
                .decode(&packet)
                .map_err(|e| format!("MP3 解码失败：{e}"))?;
            self.buf.clear();
            decoded.copy_to_vec_interleaved::<f32>(&mut self.buf);
            let spec = decoded.spec();
            self.rate = spec.rate();
            self.channels = spec.channels().count();
            if !self.buf.is_empty() {
                return Ok(Some((self.buf.clone(), self.rate, self.channels)));
            }
        }
    }
}
