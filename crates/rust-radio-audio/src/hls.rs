//! HLS 直播拉流：解析 m3u8、从倒数第 3 段起播、后台线程预取分片。
//!
//! 云听主/备线路都是 HLS；主线路分片是 `.aac`（ADTS，5 秒），备用是 `.ts`（10 秒）。
//! 下载放在独立线程（队列 3 段），解码线程只从队列取，避免网络抖动把环形缓冲掏空。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::Duration;

use m3u8_rs::{MediaPlaylist, Playlist};

pub struct Segment {
    pub data: Vec<u8>,
    pub is_ts: bool,
}

pub struct HlsSource {
    rx: Receiver<Result<Segment, String>>,
    cancel: Arc<AtomicBool>,
}

impl HlsSource {
    pub fn open(url: &str, cancel: Arc<AtomicBool>) -> Result<Self, String> {
        let client = build_client()?;
        let (playlist_url, playlist) = fetch_media_playlist(&client, url)?;
        if playlist.segments.is_empty() {
            return Err("播放列表为空".into());
        }
        let (tx, rx) = mpsc::sync_channel(3);
        let flag = cancel.clone();
        std::thread::Builder::new()
            .name("hls-fetch".into())
            .spawn(move || download_loop(client, playlist_url, playlist, tx, flag))
            .map_err(|e| format!("启动拉流线程失败：{e}"))?;
        Ok(Self { rx, cancel })
    }

    /// 取下一段（后台已下载好的）；被取消或流结束返回 `Ok(None)`。
    pub fn next_segment(&mut self) -> Result<Option<Segment>, String> {
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(seg)) => return Ok(Some(seg)),
                Ok(Err(e)) => return Err(e),
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Ok(None),
            }
        }
    }
}

fn download_loop(
    client: reqwest::blocking::Client,
    playlist_url: String,
    mut playlist: MediaPlaylist,
    tx: SyncSender<Result<Segment, String>>,
    cancel: Arc<AtomicBool>,
) {
    // 直播从倒数第 3 段起播（DESIGN 第 6 条），避免一上来就落后十几秒
    let mut next_seq = playlist.media_sequence + playlist.segments.len().saturating_sub(3) as u64;
    let target = Duration::from_secs(playlist.target_duration.max(2));
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        if playlist.media_sequence > next_seq {
            // 落后太多（暂停/长时间卡顿后恢复）：直接追到最新倒数第 2 段，不补旧内容
            let jump = playlist.segments.len().saturating_sub(2);
            next_seq = playlist.media_sequence + jump as u64;
        }
        let idx = next_seq.saturating_sub(playlist.media_sequence) as usize;
        if idx < playlist.segments.len() {
            let seg = &playlist.segments[idx];
            let uri = resolve(&playlist_url, &seg.uri);
            let is_ts = seg.uri.split('?').next().unwrap_or("").ends_with(".ts");
            let data = match fetch(&client, &uri) {
                Ok(d) => d,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            if tx.send(Ok(Segment { data, is_ts })).is_err() {
                return; // 解码端已退出
            }
            next_seq += 1;
            continue;
        }
        if playlist.end_list {
            return;
        }
        // 追平：等半个分片再刷新列表
        let wait = (target / 2).clamp(Duration::from_millis(300), Duration::from_secs(3));
        sleep_cancellable(wait, &cancel);
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        match load_media_playlist(&client, &playlist_url) {
            Ok(p) => playlist = p,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        }
    }
}

fn build_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .connect_timeout(Duration::from_secs(5))
        .user_agent("Mozilla/5.0")
        .build()
        .map_err(|e| format!("创建网络客户端失败：{e}"))
}

fn load_media_playlist(
    client: &reqwest::blocking::Client,
    url: &str,
) -> Result<MediaPlaylist, String> {
    let text = get_text(client, url)?;
    match m3u8_rs::parse_playlist(text.as_bytes()) {
        Ok((_, Playlist::MediaPlaylist(m))) => Ok(m),
        Ok((_, Playlist::MasterPlaylist(_))) => Err("分片地址不是媒体播放列表".into()),
        Err(e) => Err(format!("播放列表解析失败：{e}")),
    }
}

fn get_text(client: &reqwest::blocking::Client, url: &str) -> Result<String, String> {
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("播放列表请求失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "播放列表请求失败（HTTP {}）",
            resp.status().as_u16()
        ));
    }
    resp.text().map_err(|e| format!("播放列表读取失败：{e}"))
}

fn fetch(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>, String> {
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("分片请求失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("分片请求失败（HTTP {}）", resp.status().as_u16()));
    }
    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| format!("分片读取失败：{e}"))
}

/// 首次打开：支持 master playlist（取码率最高的变体）与直接的媒体列表。
fn fetch_media_playlist(
    client: &reqwest::blocking::Client,
    url: &str,
) -> Result<(String, MediaPlaylist), String> {
    let text = get_text(client, url)?;
    match m3u8_rs::parse_playlist(text.as_bytes()) {
        Ok((_, Playlist::MediaPlaylist(m))) => Ok((url.to_string(), m)),
        Ok((_, Playlist::MasterPlaylist(master))) => {
            let best = master
                .variants
                .iter()
                .max_by_key(|v| v.bandwidth)
                .ok_or_else(|| "master 播放列表里没有变体".to_string())?;
            let child = resolve(url, &best.uri);
            fetch_media_playlist(client, &child)
        }
        Err(e) => Err(format!("播放列表解析失败：{e}")),
    }
}

pub fn resolve(base: &str, uri: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri.to_string();
    }
    let cut = base.rfind('/').map(|i| i + 1).unwrap_or(0);
    format!("{}{}", &base[..cut], uri)
}

fn sleep_cancellable(dur: Duration, cancel: &AtomicBool) {
    let step = Duration::from_millis(100);
    let mut left = dur;
    while !left.is_zero() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let s = left.min(step);
        std::thread::sleep(s);
        left -= s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_segments() {
        assert_eq!(
            resolve("http://a/b/index.m3u8?x=1", "seg1.aac"),
            "http://a/b/seg1.aac"
        );
        assert_eq!(
            resolve("http://a/b/index.m3u8", "http://c/d.aac"),
            "http://c/d.aac"
        );
    }

    #[test]
    fn parses_media_playlist_and_start_index() {
        let text = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:5\n#EXT-X-MEDIA-SEQUENCE:9259\n#EXTINF:5.0,\nsegment-9259.aac\n#EXTINF:5.0,\nsegment-9260.aac\n#EXTINF:5.0,\nsegment-9261.aac\n#EXTINF:5.0,\nsegment-9262.aac\n#EXTINF:5.0,\nsegment-9263.aac\n";
        let Ok((_, Playlist::MediaPlaylist(m))) = m3u8_rs::parse_playlist(text.as_bytes()) else {
            panic!("not media");
        };
        assert_eq!(m.media_sequence, 9259);
        let start = m.segments.len().saturating_sub(3);
        assert_eq!(start, 2);
        assert_eq!(m.media_sequence + start as u64, 9261);
    }
}
