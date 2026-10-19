//! ADTS（AAC 裸流）分帧。
//!
//! 云听主线路的 HLS 分片就是 `.aac`（ADTS），每片约 5 秒。这里只做一件事件：
//! 从字节流里切出完整的 ADTS 帧，交给 `decode::fdk` 解码；不解析音频载荷。

const SAMPLE_RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

const MAX_FRAME: usize = 8191;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdtsHeader {
    /// AAC profile（ADTS 里是 object type - 1，LC = 1）
    pub profile: u8,
    pub sample_rate: u32,
    pub channels: u8,
    pub frame_len: usize,
    pub header_len: usize,
}

/// 解析一个 ADTS 头；不是合法头则返回 `None`。
pub fn parse_header(buf: &[u8]) -> Option<AdtsHeader> {
    if buf.len() < 7 {
        return None;
    }
    if buf[0] != 0xFF || (buf[1] & 0xF6) != 0xF0 {
        return None;
    }
    let protection_absent = buf[1] & 0x01 != 0;
    let profile = (buf[2] >> 6) & 0x03;
    let sf_index = ((buf[2] >> 2) & 0x0F) as usize;
    let sample_rate = *SAMPLE_RATES.get(sf_index)?;
    let channels = ((buf[2] & 0x01) << 2) | ((buf[3] >> 6) & 0x03);
    let frame_len =
        (((buf[3] as usize & 0x03) << 11) | ((buf[4] as usize) << 3) | ((buf[5] as usize) >> 5))
            & 0x1FFF;
    let header_len = if protection_absent { 7 } else { 9 };
    if frame_len < header_len || frame_len > MAX_FRAME {
        return None;
    }
    Some(AdtsHeader {
        profile,
        sample_rate,
        channels,
        frame_len,
        header_len,
    })
}

fn find_sync(buf: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i + 1 < buf.len() {
        if buf[i] == 0xFF && (buf[i + 1] & 0xF6) == 0xF0 {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// 流式分帧：`push` 任意大小的网络块，`next_frame` 取完整 ADTS 帧（含头）。
pub struct AdtsFramer {
    buf: Vec<u8>,
}

impl AdtsFramer {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(16 * 1024),
        }
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
        // 正常情况下缓冲里只会剩下不足一帧的数据；这里只防异常流把内存吃光
        if self.buf.len() > 1024 * 1024 {
            let cut = self.buf.len() - 512 * 1024;
            self.buf.drain(..cut);
        }
    }

    /// 一次喂一整块数据并取出其中所有完整帧（内部按 32 KB 分片，避免大块一次性入缓冲）。
    pub fn frames_from(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for chunk in data.chunks(32 * 1024) {
            self.push(chunk);
            while let Some(f) = self.next_frame() {
                out.push(f);
            }
        }
        out
    }

    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        loop {
            let pos = match find_sync(&self.buf) {
                Some(p) => p,
                None => {
                    // 末尾可能是 0xFF，保留一个字节等下一块
                    let keep = self.buf.last().copied().filter(|b| *b == 0xFF);
                    self.buf.clear();
                    if let Some(b) = keep {
                        self.buf.push(b);
                    }
                    return None;
                }
            };
            if pos > 0 {
                self.buf.drain(..pos);
            }
            if self.buf.len() < 7 {
                // 头还没收全，等下一块
                return None;
            }
            let Some(header) = parse_header(&self.buf) else {
                // 假同步：丢掉一个字节重新找
                self.buf.drain(..1);
                continue;
            };
            if self.buf.len() < header.frame_len {
                return None;
            }
            // 帧尾应紧跟下一个同步字（或正好是流尾），否则是假同步
            if self.buf.len() > header.frame_len {
                let next = &self.buf[header.frame_len..];
                if next.len() >= 2 && (next[0] != 0xFF || (next[1] & 0xF6) != 0xF0) {
                    self.buf.drain(..1);
                    continue;
                }
            }
            let frame: Vec<u8> = self.buf[..header.frame_len].to_vec();
            self.buf.drain(..header.frame_len);
            return Some(frame);
        }
    }
}

impl Default for AdtsFramer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个合法的 ADTS 头（LC / 44.1k / 2ch / 7 字节头）
    fn header(frame_len: usize) -> Vec<u8> {
        let mut h = vec![0u8; 7];
        h[0] = 0xFF;
        h[1] = 0xF1; // sync + MPEG-4 + layer 0 + protection absent
        h[2] = (1 << 6) | (4 << 2); // profile LC、44100、声道高 1 位
        h[3] = (2 << 6) | ((frame_len >> 11) & 0x03) as u8;
        h[4] = ((frame_len >> 3) & 0xFF) as u8;
        h[5] = (((frame_len & 0x07) << 5) as u8) | 0x1F;
        h[6] = 0xFC;
        h
    }

    fn frame(n: usize, fill: u8) -> Vec<u8> {
        let mut f = header(n);
        f.resize(n, fill);
        f
    }

    #[test]
    fn parses_header_fields() {
        let h = parse_header(&header(321)).unwrap();
        assert_eq!(h.profile, 1);
        assert_eq!(h.sample_rate, 44_100);
        assert_eq!(h.channels, 2);
        assert_eq!(h.frame_len, 321);
        assert_eq!(h.header_len, 7);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_header(&[0u8; 7]).is_none());
        assert!(parse_header(&header(321)[..6]).is_none());
    }

    #[test]
    fn splits_frames_across_pushes() {
        let a = frame(128, 1);
        let b = frame(256, 2);
        let mut fr = AdtsFramer::new();
        fr.push(&a[..5]);
        assert!(fr.next_frame().is_none());
        fr.push(&a[5..]);
        assert_eq!(fr.next_frame().unwrap(), a);
        assert!(fr.next_frame().is_none());
        fr.push(&b);
        assert_eq!(fr.next_frame().unwrap(), b);
    }

    #[test]
    fn resyncs_after_garbage() {
        let a = frame(100, 7);
        let mut fr = AdtsFramer::new();
        fr.push(&[0, 1, 2, 3]);
        fr.push(&a);
        assert_eq!(fr.next_frame().unwrap(), a);
    }

    #[test]
    fn drops_false_sync() {
        let a = frame(90, 3);
        // 假头：字段看着合法（帧长 7），但帧尾不是同步字
        let mut buf = vec![0xFF, 0xF1, 0x00, 0x00, 0x00, 0x00, 0xE0, 0x11, 0x22];
        buf.extend_from_slice(&a);
        let mut fr = AdtsFramer::new();
        fr.push(&buf);
        assert_eq!(fr.next_frame().unwrap(), a);
    }

    #[test]
    fn waits_for_full_frame() {
        let a = frame(200, 5);
        let mut fr = AdtsFramer::new();
        fr.push(&a[..100]);
        assert!(fr.next_frame().is_none());
        fr.push(&a[100..]);
        assert_eq!(fr.next_frame().unwrap(), a);
    }
}
