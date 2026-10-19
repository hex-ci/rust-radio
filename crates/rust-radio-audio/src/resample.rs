//! 采样率/声道适配：解码器输出固定 44.1 kHz 立体声，声卡默认配置可能是 48 kHz、
//! 也可能不是双声道。这里用 rubato 的窗函数 sinc 重采样，
//! 不做线性插值，避免高频损失。

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};

const CHUNK: usize = 1024;

pub struct StreamResampler {
    inner: Option<Async<f32>>,
    in_ch: usize,
    out_ch: usize,
    pending: Vec<f32>,
    /// 输出暂存（交错、`in_ch` 声道），每块复用，避免在播放线程里反复分配
    scratch: Vec<f32>,
}

impl StreamResampler {
    pub fn new(in_rate: u32, out_rate: u32, in_ch: usize, out_ch: usize) -> Self {
        let in_ch = in_ch.max(1);
        let out_ch = out_ch.max(1);
        let inner = if in_rate == out_rate {
            None
        } else {
            let params = SincInterpolationParameters::new(256, WindowFunction::BlackmanHarris2)
                .f_cutoff(0.95)
                .oversampling_factor(256)
                .interpolation(SincInterpolationType::Cubic);
            Async::<f32>::new_sinc(
                out_rate as f64 / in_rate as f64,
                1.0,
                &params,
                CHUNK,
                in_ch,
                FixedAsync::Input,
            )
            .ok()
        };
        Self {
            inner,
            in_ch,
            out_ch,
            pending: Vec::with_capacity(CHUNK * in_ch * 2),
            scratch: Vec::new(),
        }
    }

    /// 追加交错采样，把能处理的部分转换后写进 `out`（交错）。
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if input.is_empty() {
            return;
        }
        let Self {
            inner,
            in_ch,
            out_ch,
            pending,
            scratch,
        } = self;
        let (in_ch, out_ch) = (*in_ch, *out_ch);
        let Some(rs) = inner.as_mut() else {
            // 采样率相同：只做声道映射（直通或单声道→立体声）
            if in_ch == out_ch {
                out.extend_from_slice(input);
            } else {
                for frame in input.chunks_exact(in_ch) {
                    for c in 0..out_ch {
                        out.push(frame[c.min(in_ch - 1)]);
                    }
                }
            }
            return;
        };
        pending.extend_from_slice(input);
        let need = CHUNK * in_ch;
        while pending.len() >= need {
            let out_frames = rs.output_frames_next();
            scratch.resize(out_frames * in_ch, 0.0);
            let produced = {
                let (Ok(in_adapter), Ok(mut out_adapter)) = (
                    InterleavedSlice::new(&pending[..need], in_ch, CHUNK),
                    InterleavedSlice::new_mut(scratch, in_ch, out_frames),
                ) else {
                    pending.drain(..need);
                    continue;
                };
                match rs.process_into_buffer(&in_adapter, &mut out_adapter, None) {
                    Ok((_, nbr_out)) => nbr_out,
                    // 理论上不会失败；这一块直接跳过，避免整条链路断掉
                    Err(_) => 0,
                }
            };
            pending.drain(..need);
            for frame in scratch.chunks_exact(in_ch).take(produced) {
                for c in 0..out_ch {
                    out.push(frame[c.min(in_ch - 1)]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn sine(rate: u32, hz: f32, secs: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n)
            .flat_map(|i| {
                let v = (TAU * hz * i as f32 / rate as f32).sin() * 0.5;
                [v, v]
            })
            .collect()
    }

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt()
    }

    #[test]
    fn resamples_44k1_to_48k() {
        let input = sine(44_100, 440.0, 0.5);
        let mut rs = StreamResampler::new(44_100, 48_000, 2, 2);
        let mut out = Vec::new();
        rs.push(&input, &mut out);
        assert!(!out.is_empty());
        // 输出长度应接近输入长度 × 48/44.1（允许一个 chunk 的延迟）
        let expect = input.len() / 2 * 48_000 / 44_100;
        let got = out.len() / 2;
        assert!(got + 2048 >= expect, "got {got}, expect {expect}");
        assert!(got <= expect + 2048, "got {got}, expect {expect}");
        // 幅度不应被明显改变
        assert!((rms(&out) - 0.35).abs() < 0.05, "rms={}", rms(&out));
    }

    #[test]
    fn passthrough_when_rates_match() {
        let input = sine(48_000, 1000.0, 0.1);
        let mut rs = StreamResampler::new(48_000, 48_000, 2, 2);
        let mut out = Vec::new();
        rs.push(&input, &mut out);
        assert_eq!(out, input);
    }

    #[test]
    fn mono_to_stereo_maps_channels() {
        let mut rs = StreamResampler::new(44_100, 44_100, 1, 2);
        let mut out = Vec::new();
        rs.push(&[0.5, -0.5], &mut out);
        assert_eq!(out, vec![0.5, 0.5, -0.5, -0.5]);
    }
}
