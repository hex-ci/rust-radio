//! 音频引擎：内置解码（libfdk-aac + symphonia）→ ringbuf → cpal。
//!
//! 不再依赖外部 ffmpeg 进程。三条播放路径：
//! - HLS 直播：主线路 `.aac`（HE-AAC v2，ADTS），备用 `.ts`（AAC-LC，MPEG-TS）
//! - 回听：`.m4a`（AAC-LC），HTTP Range + symphonia 解复用，可 seek
//! - 直连 MP3：兜底分支，symphonia 解码
//!
//! 解码线程与 cpal 回调之间只有一个 2 秒的 f32 环形缓冲；UI 线程绝不碰网络/解码。

mod adts;
mod decode;
mod hls;
mod mp3;
mod mp4;
mod resample;
mod ts;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use rust_radio_core::StreamKind;

use crate::adts::AdtsFramer;
use crate::decode::fdk::AacDecoder;
use crate::resample::StreamResampler;

pub fn decoder_name() -> &'static str {
    "内置解码器（FDK AAC / Symphonia）"
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("没有可用的音频输出设备：{0}")]
    NoDevice(String),
    #[error("音频设备启动失败：{0}")]
    Device(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    Idle,
    Buffering,
    Playing,
    Paused,
    Error,
}

impl PlayState {
    pub fn label(&self) -> &'static str {
        match self {
            PlayState::Idle => "空闲",
            PlayState::Buffering => "缓冲中",
            PlayState::Playing => "播放中",
            PlayState::Paused => "已暂停",
            PlayState::Error => "连接失败",
        }
    }
    pub fn is_on(&self) -> bool {
        matches!(
            self,
            PlayState::Playing | PlayState::Buffering | PlayState::Paused
        )
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub state: PlayState,
    pub error: Option<String>,
    pub position: f64,
    pub duration: Option<f64>,
    pub sample_rate: u32,
    pub channels: u16,
    /// 0..1 瞬时电平
    pub level: f32,
}

#[derive(Debug, Clone)]
pub struct Source {
    pub url: String,
    pub kind: StreamKind,
    pub title: String,
    pub subtitle: String,
    /// 回听时长（秒）
    pub duration: Option<f64>,
    /// 回听起播位置（秒）
    pub start_at: f64,
}

impl Source {
    pub fn is_file(&self) -> bool {
        self.kind == StreamKind::File
    }
}

enum Cmd {
    Play(Box<Source>),
    Stop,
    SetPaused(bool),
    Seek(f64),
    Shutdown,
}

struct Shared {
    state: Mutex<(PlayState, Option<String>)>,
    volume: AtomicU32,
    paused: AtomicBool,
    frames: AtomicU64,
    base_ms: AtomicU64,
    rate: AtomicU32,
    channels: AtomicU32,
    level: AtomicU32,
    gen: AtomicU64,
    duration_ms: AtomicU64,
}

impl Shared {
    fn set_state(&self, st: PlayState, err: Option<String>) {
        *self.state.lock().unwrap() = (st, err);
    }
    fn state(&self) -> (PlayState, Option<String>) {
        self.state.lock().unwrap().clone()
    }
    fn bump_gen(&self) {
        self.gen.fetch_add(1, Ordering::SeqCst);
        self.frames.store(0, Ordering::Relaxed);
        self.level.store(0f32.to_bits(), Ordering::Relaxed);
    }
}

pub struct Engine {
    cmd: Sender<Cmd>,
    shared: Arc<Shared>,
    _stream: cpal::Stream,
    worker: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn new() -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| AudioError::NoDevice("未找到默认输出设备".into()))?;
        let supported = device
            .default_output_config()
            .map_err(|e| AudioError::Device(e.to_string()))?;
        let sample_rate = supported.sample_rate();
        let channels = supported.channels();
        let format = supported.sample_format();
        let cfg: cpal::StreamConfig = supported.into();

        let shared = Arc::new(Shared {
            state: Mutex::new((PlayState::Idle, None)),
            volume: AtomicU32::new(0.8f32.to_bits()),
            paused: AtomicBool::new(false),
            frames: AtomicU64::new(0),
            base_ms: AtomicU64::new(0),
            rate: AtomicU32::new(sample_rate),
            channels: AtomicU32::new(channels as u32),
            level: AtomicU32::new(0f32.to_bits()),
            gen: AtomicU64::new(0),
            duration_ms: AtomicU64::new(0),
        });

        // 2 秒环形缓冲
        let rb = HeapRb::<f32>::new(sample_rate as usize * channels as usize * 2);
        let (prod, cons) = rb.split();
        let prod = Arc::new(Mutex::new(prod));

        let err_fn = |e| eprintln!("[audio] {e}");
        let sh = shared.clone();
        let stream = match format {
            cpal::SampleFormat::F32 => build_stream::<f32>(&device, cfg, sh, cons, err_fn),
            cpal::SampleFormat::I16 => build_stream::<i16>(&device, cfg, sh, cons, err_fn),
            cpal::SampleFormat::U16 => build_stream::<u16>(&device, cfg, sh, cons, err_fn),
            other => return Err(AudioError::Device(format!("不支持的采样格式 {other:?}"))),
        }
        .map_err(|e| AudioError::Device(e.to_string()))?;
        stream
            .play()
            .map_err(|e| AudioError::Device(e.to_string()))?;

        let (tx, rx) = mpsc::channel();
        let shared_w = shared.clone();
        let prod_w = prod.clone();
        let worker = std::thread::Builder::new()
            .name("audio-worker".into())
            .spawn(move || worker(rx, prod_w, shared_w, sample_rate, channels as usize))
            .map_err(|e| AudioError::Device(e.to_string()))?;

        Ok(Self {
            cmd: tx,
            shared,
            _stream: stream,
            worker: Some(worker),
        })
    }

    pub fn play(&self, source: Source) {
        self.shared.paused.store(false, Ordering::SeqCst);
        self.shared.base_ms.store(
            (source.start_at.max(0.0) * 1000.0) as u64,
            Ordering::Relaxed,
        );
        match source.duration {
            Some(d) => self
                .shared
                .duration_ms
                .store((d * 1000.0) as u64, Ordering::Relaxed),
            None => self.shared.duration_ms.store(0, Ordering::Relaxed),
        }
        let _ = self.cmd.send(Cmd::Play(Box::new(source)));
    }

    pub fn stop(&self) {
        let _ = self.cmd.send(Cmd::Stop);
    }

    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::SeqCst);
        let _ = self.cmd.send(Cmd::SetPaused(paused));
    }

    pub fn seek(&self, secs: f64) {
        let secs = secs.max(0.0);
        self.shared
            .base_ms
            .store((secs * 1000.0) as u64, Ordering::Relaxed);
        let _ = self.cmd.send(Cmd::Seek(secs));
    }

    pub fn set_volume(&self, v: f32) {
        self.shared
            .volume
            .store(v.clamp(0.0, 1.5).to_bits(), Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.shared.volume.load(Ordering::Relaxed))
    }

    pub fn snapshot(&self) -> Snapshot {
        let (state, error) = self.shared.state();
        let rate = self.shared.rate.load(Ordering::Relaxed).max(1);
        let frames = self.shared.frames.load(Ordering::Relaxed);
        let base = self.shared.base_ms.load(Ordering::Relaxed) as f64 / 1000.0;
        let dur_ms = self.shared.duration_ms.load(Ordering::Relaxed);
        Snapshot {
            state,
            error,
            position: base + frames as f64 / rate as f64,
            duration: (dur_ms > 0).then(|| dur_ms as f64 / 1000.0),
            sample_rate: rate,
            channels: self.shared.channels.load(Ordering::Relaxed) as u16,
            level: f32::from_bits(self.shared.level.load(Ordering::Relaxed)),
        }
    }

    pub fn is_active(&self) -> bool {
        self.snapshot().state.is_on()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Shutdown);
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

// ------------------------------------------------------------------ 输出流

fn build_stream<T>(
    device: &cpal::Device,
    cfg: cpal::StreamConfig,
    shared: Arc<Shared>,
    mut cons: HeapCons<f32>,
    err_fn: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    let mut my_gen = shared.gen.load(Ordering::SeqCst);
    let mut gain = f32::from_bits(shared.volume.load(Ordering::Relaxed));
    let mut scratch: Vec<f32> = Vec::new();
    let ch = shared.channels.load(Ordering::Relaxed).max(1) as usize;

    device.build_output_stream(
        cfg,
        move |data: &mut [T], _| {
            let gen = shared.gen.load(Ordering::SeqCst);
            if gen != my_gen {
                while cons.try_pop().is_some() {}
                my_gen = gen;
            }
            let paused = shared.paused.load(Ordering::Relaxed);
            let (state, _) = shared.state();
            let target = f32::from_bits(shared.volume.load(Ordering::Relaxed));

            if paused || state == PlayState::Idle || state == PlayState::Error {
                for s in data.iter_mut() {
                    *s = T::from_sample(0.0f32);
                }
                shared.level.store(0f32.to_bits(), Ordering::Relaxed);
                gain = target;
                return;
            }

            scratch.clear();
            scratch.resize(data.len(), 0.0);
            let n = cons.pop_slice(&mut scratch);
            let mut energy = 0f32;
            for (i, s) in data.iter_mut().enumerate() {
                gain += (target - gain) * 0.002;
                let v = if i < n { scratch[i] } else { 0.0 } * gain;
                energy += v * v;
                *s = T::from_sample(v);
            }
            if n > 0 {
                shared
                    .frames
                    .fetch_add((n / ch.max(1)) as u64, Ordering::Relaxed);
            }
            let rms = (energy / data.len().max(1) as f32).sqrt();
            let prev = f32::from_bits(shared.level.load(Ordering::Relaxed));
            shared.level.store(
                (prev * 0.72 + rms * 0.28).clamp(0.0, 1.0).to_bits(),
                Ordering::Relaxed,
            );
        },
        err_fn,
        None,
    )
}

// ------------------------------------------------------------------ 解码线程

struct Active {
    cancel: Arc<AtomicBool>,
    _handle: JoinHandle<()>,
}

fn worker(
    rx: Receiver<Cmd>,
    prod: Arc<Mutex<HeapProd<f32>>>,
    shared: Arc<Shared>,
    rate: u32,
    channels: usize,
) {
    let mut current: Option<Source> = None;
    let mut active: Option<Active> = None;

    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Shutdown => {
                if let Some(a) = active.take() {
                    a.cancel.store(true, Ordering::SeqCst);
                }
                shared.set_state(PlayState::Idle, None);
                break;
            }
            Cmd::Stop => {
                if let Some(a) = active.take() {
                    a.cancel.store(true, Ordering::SeqCst);
                }
                shared.set_state(PlayState::Idle, None);
                shared.bump_gen();
                current = None;
            }
            Cmd::SetPaused(p) => {
                let (st, _) = shared.state();
                if p && st != PlayState::Error {
                    shared.set_state(PlayState::Paused, None);
                } else if !p && st == PlayState::Paused {
                    shared.set_state(PlayState::Playing, None);
                }
            }
            Cmd::Seek(pos) => {
                if let Some(mut src) = current.clone() {
                    if let Some(a) = active.take() {
                        a.cancel.store(true, Ordering::SeqCst);
                    }
                    shared.bump_gen();
                    shared.set_state(PlayState::Buffering, None);
                    src.start_at = pos;
                    active = Some(spawn_decoder(src, &prod, &shared, rate, channels));
                }
            }
            Cmd::Play(src) => {
                if let Some(a) = active.take() {
                    a.cancel.store(true, Ordering::SeqCst);
                }
                shared.bump_gen();
                shared.set_state(PlayState::Buffering, None);
                current = Some(*src);
                active = Some(spawn_decoder(
                    current.clone().expect("just set"),
                    &prod,
                    &shared,
                    rate,
                    channels,
                ));
            }
        }
    }

    if let Some(a) = active.take() {
        a.cancel.store(true, Ordering::SeqCst);
    }
    shared.set_state(PlayState::Idle, None);
}

fn spawn_decoder(
    src: Source,
    prod: &Arc<Mutex<HeapProd<f32>>>,
    shared: &Arc<Shared>,
    dev_rate: u32,
    dev_ch: usize,
) -> Active {
    let cancel = Arc::new(AtomicBool::new(false));
    let gen = shared.gen.load(Ordering::SeqCst);
    let c = cancel.clone();
    let p = prod.clone();
    let sh = shared.clone();
    let handle = std::thread::Builder::new()
        .name("audio-decode".into())
        .spawn(move || {
            let mut sink = Sink::new(p, sh.clone(), gen, c.clone(), dev_rate, dev_ch);
            let result = match src.kind {
                StreamKind::Hls => run_hls(&src, &mut sink, &c),
                StreamKind::File => run_m4a(&src, &mut sink, &c),
                StreamKind::Mp3 => run_mp3(&src, &mut sink, &c),
            };
            if c.load(Ordering::SeqCst) || sink.stale() {
                return;
            }
            match result {
                Ok(Ending::Eof) => sink.finish(),
                Ok(Ending::Stopped) => {}
                Err(e) => sink.fail(e),
            }
        })
        .ok();
    match handle {
        Some(h) => Active { cancel, _handle: h },
        None => {
            shared.set_state(PlayState::Error, Some("无法启动解码线程".into()));
            Active {
                cancel,
                _handle: std::thread::spawn(|| {}),
            }
        }
    }
}

enum Ending {
    Eof,
    Stopped,
}

struct Sink {
    prod: Arc<Mutex<HeapProd<f32>>>,
    shared: Arc<Shared>,
    gen: u64,
    cancel: Arc<AtomicBool>,
    dev_rate: u32,
    dev_ch: usize,
    rs: Option<StreamResampler>,
    in_rate: u32,
    in_ch: usize,
    started: bool,
    out: Vec<f32>,
}

impl Sink {
    fn new(
        prod: Arc<Mutex<HeapProd<f32>>>,
        shared: Arc<Shared>,
        gen: u64,
        cancel: Arc<AtomicBool>,
        dev_rate: u32,
        dev_ch: usize,
    ) -> Self {
        Self {
            prod,
            shared,
            gen,
            cancel,
            dev_rate,
            dev_ch,
            rs: None,
            in_rate: 0,
            in_ch: 0,
            started: false,
            out: Vec::with_capacity(8192),
        }
    }

    /// 交给 HLS 拉流线程用的取消标记
    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    fn stale(&self) -> bool {
        self.shared.gen.load(Ordering::SeqCst) != self.gen
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed) || self.stale()
    }

    /// 解码器输出的交错 PCM → 重采样 → 环形缓冲。返回 Err 表示被取消/已过期。
    fn push(&mut self, samples: &[f32], rate: u32, channels: usize) -> Result<(), ()> {
        if samples.is_empty() || rate == 0 || channels == 0 {
            return Ok(());
        }
        if self.rs.is_none() || self.in_rate != rate || self.in_ch != channels {
            self.rs = Some(StreamResampler::new(
                rate,
                self.dev_rate,
                channels,
                self.dev_ch,
            ));
            self.in_rate = rate;
            self.in_ch = channels;
        }
        self.out.clear();
        if let Some(rs) = &mut self.rs {
            rs.push(samples, &mut self.out);
        }
        if self.out.is_empty() {
            return Ok(());
        }
        let mut off = 0;
        while off < self.out.len() {
            if self.cancelled() {
                return Err(());
            }
            let wrote = {
                let mut p = self.prod.lock().unwrap();
                p.push_slice(&self.out[off..])
            };
            if wrote == 0 {
                std::thread::sleep(Duration::from_millis(5));
            } else {
                off += wrote;
            }
        }
        if !self.started {
            // 先攒 200 ms 再切 Playing，避免回调一上来就 underrun
            let buffered = {
                let p = self.prod.lock().unwrap();
                p.occupied_len()
            };
            let need = (self.dev_rate as usize * self.dev_ch / 5).max(1);
            if buffered >= need {
                self.started = true;
                if !self.cancelled() && !self.shared.paused.load(Ordering::Relaxed) {
                    self.shared.set_state(PlayState::Playing, None);
                }
            }
        }
        Ok(())
    }

    fn finish(&self) {
        if self.stale() {
            return;
        }
        let (st, _) = self.shared.state();
        if matches!(st, PlayState::Playing | PlayState::Buffering) {
            self.shared.set_state(PlayState::Idle, None);
        }
    }

    fn fail(&self, msg: String) {
        if self.stale() {
            return;
        }
        let (st, _) = self.shared.state();
        if matches!(
            st,
            PlayState::Playing | PlayState::Buffering | PlayState::Paused
        ) {
            self.shared.set_state(PlayState::Error, Some(msg));
        }
    }
}

fn run_hls(src: &Source, sink: &mut Sink, _cancel: &AtomicBool) -> Result<Ending, String> {
    let mut hls = hls::HlsSource::open(&src.url, sink.cancel_flag())?;
    let mut framer = AdtsFramer::new();
    let mut decoder: Option<AacDecoder> = None;
    let mut bad_frames = 0;
    loop {
        if sink.cancelled() {
            return Ok(Ending::Stopped);
        }
        let Some(seg) = hls.next_segment()? else {
            return Ok(Ending::Eof);
        };
        let frames: Vec<Vec<u8>> = if seg.is_ts {
            ts::extract_adts(&seg.data)?
        } else {
            framer.frames_from(&seg.data)
        };
        for frame in frames {
            if decoder.is_none() {
                decoder = Some(AacDecoder::open_adts()?);
            }
            match decoder.as_mut().unwrap().decode(&frame) {
                Ok(list) => {
                    bad_frames = 0;
                    for df in list {
                        if sink.push(&df.samples, df.rate, df.channels).is_err() {
                            return Ok(Ending::Stopped);
                        }
                    }
                }
                Err(e) => {
                    // 分片边界上的残帧：跳过；连续失败才认为是故障
                    bad_frames += 1;
                    if bad_frames > 8 {
                        return Err(e);
                    }
                }
            }
        }
    }
}

fn run_m4a(src: &Source, sink: &mut Sink, _cancel: &AtomicBool) -> Result<Ending, String> {
    let mut m = mp4::M4aSource::open(&src.url, src.start_at)?;
    let mut decoder = AacDecoder::open_asc(m.asc())?;
    loop {
        if sink.cancelled() {
            return Ok(Ending::Stopped);
        }
        let Some(packet) = m.next_packet()? else {
            return Ok(Ending::Eof);
        };
        for df in decoder.decode(&packet)? {
            if sink.push(&df.samples, df.rate, df.channels).is_err() {
                return Ok(Ending::Stopped);
            }
        }
    }
}

fn run_mp3(src: &Source, sink: &mut Sink, _cancel: &AtomicBool) -> Result<Ending, String> {
    let mut m = mp3::Mp3Source::open(&src.url)?;
    loop {
        if sink.cancelled() {
            return Ok(Ending::Stopped);
        }
        let Some((pcm, rate, ch)) = m.next_pcm()? else {
            return Ok(Ending::Eof);
        };
        if sink.push(&pcm, rate, ch).is_err() {
            return Ok(Ending::Stopped);
        }
    }
}
