//! libfdk-aac 的安全薄封装（全 crate 唯一的 `unsafe` 集中在这里）。
//!
//! 云听主线路是 HE-AAC v2：AAC-LC 核心 + SBR + 参数立体声。symphonia 只解 LC，
//! 会掉到 22.05 kHz 单声道，所以 AAC 一律交给 libfdk-aac。

use fdk_aac_sys as fdk;

#[derive(Debug, Clone)]
pub struct DecodedFrame {
    /// 交错 f32 采样（左、右、左、右…）
    pub samples: Vec<f32>,
    pub rate: u32,
    pub channels: usize,
}

pub struct AacDecoder {
    handle: fdk::HANDLE_AACDECODER,
    pcm: Vec<i16>,
    rate: u32,
    channels: usize,
}

// fdk 的句柄只在本线程使用；解码器不共享，满足 Send。
unsafe impl Send for AacDecoder {}

impl AacDecoder {
    /// ADTS 传输（直播主线路：每帧自带头）
    pub fn open_adts() -> Result<Self, String> {
        Self::open(fdk::TRANSPORT_TYPE_TT_MP4_ADTS, None)
    }

    /// 裸 AAC（回听 MP4：靠 AudioSpecificConfig 初始化）
    pub fn open_asc(asc: &[u8]) -> Result<Self, String> {
        Self::open(fdk::TRANSPORT_TYPE_TT_MP4_RAW, Some(asc))
    }

    fn open(transport: fdk::TRANSPORT_TYPE, asc: Option<&[u8]>) -> Result<Self, String> {
        unsafe {
            let handle = fdk::aacDecoder_Open(transport, 1);
            if handle.is_null() {
                return Err("AAC 解码器初始化失败".into());
            }
            if let Some(asc) = asc {
                let mut ptr = asc.as_ptr() as *mut u8;
                let len = asc.len() as u32;
                let err = fdk::aacDecoder_ConfigRaw(handle, &mut ptr, &len);
                if err != 0 {
                    fdk::aacDecoder_Close(handle);
                    return Err(format!("AAC 配置失败（{err}）"));
                }
            }
            Ok(Self {
                handle,
                // HE-AAC v2 一帧 2048 采样 × 2 声道，留一倍余量
                pcm: vec![0i16; 8192 * 2],
                rate: 0,
                channels: 0,
            })
        }
    }

    /// 喂一个完整 ADTS 帧（或一个 MP4 AAC 采样），吐出解出的 PCM 帧。
    /// 输入不完整/解码器需要更多数据时返回空 `Vec`。
    pub fn decode(&mut self, data: &[u8]) -> Result<Vec<DecodedFrame>, String> {
        if data.is_empty() {
            return Ok(Vec::new());
        }
        let mut buf = data.to_vec();
        let mut out = Vec::new();
        unsafe {
            let mut ptr = buf.as_mut_ptr();
            let size = buf.len() as u32;
            let mut valid = size;
            let err = fdk::aacDecoder_Fill(self.handle, &mut ptr, &size, &mut valid);
            if err != 0 {
                return Err(format!("AAC 填充失败（{err}）"));
            }
            // valid > 0 表示解码器内部缓冲已满，这一帧的剩余部分会被丢弃；
            // 正常 ADTS 帧不会出现，出现说明帧过大。
            loop {
                let err = fdk::aacDecoder_DecodeFrame(
                    self.handle,
                    self.pcm.as_mut_ptr(),
                    self.pcm.len() as i32,
                    0,
                );
                if err == fdk::AAC_DECODER_ERROR_AAC_DEC_NOT_ENOUGH_BITS {
                    break;
                }
                if err != 0 {
                    // 4096..8191 是传输层同步错误，交给上层决定是否跳过
                    return Err(format!("AAC 解码失败（{err}）"));
                }
                let info = &*fdk::aacDecoder_GetStreamInfo(self.handle);
                let channels = info.numChannels.max(1) as usize;
                let n = info.frameSize as usize * channels;
                if n == 0 || n > self.pcm.len() {
                    return Err("AAC 解码输出长度异常".into());
                }
                self.rate = info.sampleRate as u32;
                self.channels = channels;
                let samples = self.pcm[..n].iter().map(|s| *s as f32 / 32768.0).collect();
                out.push(DecodedFrame {
                    samples,
                    rate: self.rate,
                    channels,
                });
                // 一个 ADTS 帧理论上只产出一帧 PCM；防御性上限
                if out.len() >= 8 {
                    break;
                }
            }
        }
        Ok(out)
    }
}

impl Drop for AacDecoder {
    fn drop(&mut self) {
        unsafe {
            if !self.handle.is_null() {
                fdk::aacDecoder_Close(self.handle);
            }
        }
    }
}
