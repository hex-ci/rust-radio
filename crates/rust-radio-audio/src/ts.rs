//! 极简 MPEG-TS 解复用：只服务云听备用线路（单节目、AAC/ADTS）。
//!
//! 一个分片内先扫 PAT/PMT 找到 AAC 的 PID，再把该 PID 的 PES 载荷拼起来，
//! 交给 `AdtsFramer` 切帧。不做 CRC 校验、不处理多节目/加密/LATM——云听没有这些。

use crate::adts::AdtsFramer;

const TS_PACKET: usize = 188;
const PID_PAT: u16 = 0x0000;

fn packet_payload(pkt: &[u8]) -> Option<(bool, &[u8])> {
    if pkt.len() < TS_PACKET || pkt[0] != 0x47 {
        return None;
    }
    let pusi = pkt[1] & 0x40 != 0;
    let adaptation = (pkt[3] >> 4) & 0x03;
    let mut off = 4;
    if adaptation == 2 {
        return None; // 只有自适应字段，没有载荷
    }
    if adaptation == 3 {
        off += 1 + pkt[4] as usize;
    } else if adaptation == 1 {
        // 载荷从 4 开始
    }
    if off >= TS_PACKET {
        return None;
    }
    Some((pusi, &pkt[off..]))
}

fn section_from_payload(payload: &[u8]) -> Option<&[u8]> {
    // PSI 载荷前有 pointer_field
    let ptr = *payload.first()? as usize;
    let start = 1 + ptr;
    let rest = payload.get(start..)?;
    if rest.len() < 3 {
        return None;
    }
    let len = (((rest[1] & 0x0F) as usize) << 8) | rest[2] as usize;
    let end = (3 + len).min(rest.len());
    Some(&rest[..end])
}

#[allow(clippy::chunks_exact_to_as_chunks)] // as_chunks 需要 Rust 1.88，项目 MSRV 1.87
fn pmt_pid(segment: &[u8]) -> Option<u16> {
    for pkt in segment.chunks_exact(TS_PACKET) {
        let Some((pusi, payload)) = packet_payload(pkt) else {
            continue;
        };
        if !pusi {
            continue;
        }
        let pid = (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16;
        if pid != PID_PAT {
            continue;
        }
        let sec = section_from_payload(payload)?;
        if sec[0] != 0x00 {
            continue;
        }
        let mut i = 8;
        while i + 4 <= sec.len().saturating_sub(4) {
            let program = ((sec[i] as u16) << 8) | sec[i + 1] as u16;
            let pid = (((sec[i + 2] & 0x1F) as u16) << 8) | sec[i + 3] as u16;
            if program != 0 {
                return Some(pid);
            }
            i += 4;
        }
    }
    None
}

#[allow(clippy::chunks_exact_to_as_chunks)] // as_chunks 需要 Rust 1.88，项目 MSRV 1.87
fn audio_pid(segment: &[u8], pmt: u16) -> Option<u16> {
    for pkt in segment.chunks_exact(TS_PACKET) {
        let Some((pusi, payload)) = packet_payload(pkt) else {
            continue;
        };
        if !pusi {
            continue;
        }
        let pid = (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16;
        if pid != pmt {
            continue;
        }
        let sec = section_from_payload(payload)?;
        if sec[0] != 0x02 {
            continue;
        }
        let program_info_len = (((sec[10] & 0x0F) as usize) << 8) | sec[11] as usize;
        let mut i = 12 + program_info_len;
        let end = sec.len().saturating_sub(4);
        while i + 5 <= end {
            let stream_type = sec[i];
            let pid = (((sec[i + 1] & 0x1F) as u16) << 8) | sec[i + 2] as u16;
            let es_len = (((sec[i + 3] & 0x0F) as usize) << 8) | sec[i + 4] as usize;
            if stream_type == 0x0F || stream_type == 0x11 {
                // 0x0F = ADTS AAC；0x11 = LATM（不支持，但先返回让上层报错）
                return Some(pid);
            }
            i += 5 + es_len;
        }
    }
    None
}

/// 从一个 TS 分片里取出所有 ADTS 帧（含头）。
pub fn extract_adts(segment: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let aligned = align(segment);
    if aligned.len() < TS_PACKET {
        return Err("TS 分片为空".into());
    }
    let pmt = pmt_pid(aligned).ok_or_else(|| "TS 分片里找不到 PAT/PMT".to_string())?;
    let audio = audio_pid(aligned, pmt).ok_or_else(|| "TS 分片里没有 AAC 音轨".to_string())?;

    let mut es: Vec<u8> = Vec::with_capacity(aligned.len() / 2);
    let mut pes_open = false;
    #[allow(clippy::chunks_exact_to_as_chunks)] // as_chunks 需要 Rust 1.88，项目 MSRV 1.87
    for pkt in aligned.chunks_exact(TS_PACKET) {
        let Some((pusi, payload)) = packet_payload(pkt) else {
            continue;
        };
        let pid = (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16;
        if pid != audio {
            continue;
        }
        if pusi {
            // 新 PES：跳过 PES 头（00 00 01 + stream_id + len + flags + hdr_len）
            if payload.len() >= 9 && payload[0..3] == [0, 0, 1] {
                let hdr = 9 + payload[8] as usize;
                if hdr <= payload.len() {
                    es.extend_from_slice(&payload[hdr..]);
                    pes_open = true;
                }
            }
        } else if pes_open {
            es.extend_from_slice(payload);
        }
    }
    if es.is_empty() {
        return Err("TS 分片里没有音频载荷".into());
    }

    let mut framer = AdtsFramer::new();
    let frames = framer.frames_from(&es);
    if frames.is_empty() {
        return Err("TS 里没有 ADTS 帧".into());
    }
    Ok(frames)
}

/// 允许分片前面有少量垃圾/ID3，按 188 字节对齐到第一个 0x47。
fn align(segment: &[u8]) -> &[u8] {
    if segment.first() == Some(&0x47) {
        return segment;
    }
    for i in 0..segment.len().min(TS_PACKET * 2) {
        if segment[i] == 0x47 && segment.get(i + TS_PACKET) == Some(&0x47) {
            return &segment[i..];
        }
    }
    segment
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts_packet(pid: u16, pusi: bool, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![0xFFu8; TS_PACKET];
        p[0] = 0x47;
        p[1] = (if pusi { 0x40 } else { 0x00 }) | ((pid >> 8) as u8 & 0x1F);
        p[2] = (pid & 0xFF) as u8;
        let n = payload.len().min(183);
        if n < 183 {
            // 自适应字段填充剩余空间：4 字节头 + 1 字节长度 + 自适应 + n 字节载荷
            p[3] = 0x30;
            p[4] = (183 - n) as u8;
            p[TS_PACKET - n..].copy_from_slice(&payload[..n]);
        } else {
            p[3] = 0x10;
            p[4..4 + n].copy_from_slice(&payload[..n]);
        }
        p
    }

    fn adts_frame(len: usize, fill: u8) -> Vec<u8> {
        let mut f = vec![0u8; 7];
        f[0] = 0xFF;
        f[1] = 0xF1;
        f[2] = (1 << 6) | (4 << 2);
        f[3] = 2 << 6 | ((len >> 11) & 0x03) as u8;
        f[4] = ((len >> 3) & 0xFF) as u8;
        f[5] = (((len & 0x07) << 5) as u8) | 0x1F;
        f[6] = 0xFC;
        f.resize(len, fill);
        f
    }

    fn section(table_id: u8, body: &[u8]) -> Vec<u8> {
        let mut s = vec![table_id];
        let len = body.len() + 4;
        s.push(0xB0 | ((len >> 8) as u8 & 0x0F));
        s.push((len & 0xFF) as u8);
        s.extend_from_slice(body);
        s.extend_from_slice(&[0, 0, 0, 0]); // CRC（解析时不校验）
        s
    }

    fn pat() -> Vec<u8> {
        section(
            0x00,
            &[0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xE1, 0x00],
        )
    }

    fn pmt() -> Vec<u8> {
        section(
            0x02,
            &[
                0x00, 0x01, 0xC1, 0x00, 0x00, 0xE1, 0x01, 0xF0, 0x00, 0x0F, 0xE1, 0x02, 0xF0, 0x00,
            ],
        )
    }

    #[test]
    fn extracts_adts_from_ts() {
        let a = adts_frame(128, 1);
        let b = adts_frame(200, 2);
        let mut pes = vec![0, 0, 1, 0xC0, 0, 0, 0x80, 0x00, 0x00];
        pes.extend_from_slice(&a);
        pes.extend_from_slice(&b);

        let mut seg = Vec::new();
        let mut p = vec![0u8];
        p.extend_from_slice(&pat());
        seg.extend(ts_packet(0, true, &p));
        let mut p = vec![0u8];
        p.extend_from_slice(&pmt());
        seg.extend(ts_packet(0x100, true, &p));
        // 音频 PES 可能跨多个 TS 包
        let first = &pes[..180];
        let rest = &pes[180..];
        seg.extend(ts_packet(0x102, true, first));
        seg.extend(ts_packet(0x102, false, rest));

        let frames = extract_adts(&seg).unwrap();
        assert_eq!(frames, vec![a, b]);
    }

    #[test]
    fn reports_missing_audio() {
        let mut seg = Vec::new();
        let mut p = vec![0u8];
        p.extend_from_slice(&pat());
        seg.extend(ts_packet(0, true, &p));
        assert!(extract_adts(&seg).is_err());
    }
}
