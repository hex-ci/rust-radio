//! 采样率/声道适配：解码器输出固定 44.1 kHz 立体声，声卡默认配置可能是 48 kHz、
//! 也可能不是双声道。这里用 rubato 的窗函数 sinc 重采样（与 ffmpeg swresample 同档），
//! 不做线性插值，避免高频损失。

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

const CHUNK: usize = 1024;

pub struct StreamResampler {
    inner: Option<SincFixedIn<f32>>,
    in_ch: usize,
    out_ch: usize,
    pending: Vec<f32>,
}

impl StreamResampler {
    pub fn new(in_rate: u32, out_rate: u32, in_ch: usize, out_ch: usize) -> Self {
        let in_ch = in_ch.max(1);
        let out_ch = out_ch.max(1);
        let inner = if in_rate == out_rate {
            None
        } else {
            let params = SincInterpolationParameters {
                sinc_len: 256,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Cubic,
                oversampling_factor: 256,
                window: WindowFunction::BlackmanHarris2,
            };
            SincFixedIn::<f32>::new(out_rate as f64 / in_rate as f64, 1.0, params, CHUNK, in_ch)
                .ok()
        };
        Self {
            inner,
            in_ch,
            out_ch,
            pending: Vec::with_capacity(CHUNK * in_ch * 2),
        }
    }

    /// 追加交错采样，把能处理的部分转换后写进 `out`（交错）。
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if input.is_empty() {
            return;
        }
        if self.inner.is_none() {
            // 采样率相同：只做声道映射（直通或单声道→立体声）
            if self.in_ch == self.out_ch {
                out.extend_from_slice(input);
            } else {
                for frame in input.chunks_exact(self.in_ch) {
                    for c in 0..self.out_ch {
                        out.push(frame[c.min(self.in_ch - 1)]);
                    }
                }
            }
            return;
        }
        self.pending.extend_from_slice(input);
        let need = CHUNK * self.in_ch;
        while self.pending.len() >= need {
            let mut planar: Vec<Vec<f32>> =
                (0..self.in_ch).map(|_| Vec::with_capacity(CHUNK)).collect();
            for frame in self.pending[..need].chunks_exact(self.in_ch) {
                for (c, v) in frame.iter().enumerate() {
                    planar[c].push(*v);
                }
            }
            self.pending.drain(..need);
            match &mut self.inner {
                Some(rs) => match rs.process(&planar, None) {
                    Ok(res) => {
                        let frames = res.first().map(|v| v.len()).unwrap_or(0);
                        #[allow(clippy::needless_range_loop)] // 按帧号交织各声道，不能直接迭代替换
                        for i in 0..frames {
                            for c in 0..self.out_ch {
                                out.push(res[c.min(self.in_ch - 1)][i]);
                            }
                        }
                    }
                    Err(_) => {
                        // 理论上不会失败；这一块直接跳过，避免整条链路断掉
                    }
                },
                None => {
                    // 只做声道映射
                    let frames = planar[0].len();
                    #[allow(clippy::needless_range_loop)] // 同上：按帧号交织各声道
                    for i in 0..frames {
                        for c in 0..self.out_ch {
                            out.push(planar[c.min(self.in_ch - 1)][i]);
                        }
                    }
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
