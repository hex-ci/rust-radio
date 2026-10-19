//! 音频链路自检：现取一个台的地址 → 解码 → 输出，打印状态与帧计数。
//! `cargo run -p rust-radio-audio --example check -- 639`

use std::time::{Duration, Instant};

use rust_radio_audio::{Engine, Source};

fn main() {
    let id = std::env::args().nth(1).unwrap_or_else(|| "639".into());
    let cache = dirs_cache();
    let client = rust_radio_core::Client::new(cache).expect("client");
    let (detail, target) = client
        .fresh_stream(
            &id,
            rust_radio_core::Quality::High,
            rust_radio_core::Line::Auto,
        )
        .expect("resolve stream");
    println!(
        "台：{} / 线路：{} / 类型：{:?}",
        detail.name, target.label, target.kind
    );
    println!("地址：{}", target.url);

    let engine = Engine::new().expect("engine");
    println!("解码器：{}", rust_radio_audio::decoder_name());
    engine.set_volume(0.5);
    engine.play(Source {
        url: target.url.clone(),
        kind: target.kind,
        title: detail.name.clone(),
        subtitle: String::new(),
        duration: None,
        start_at: 0.0,
    });

    let t0 = Instant::now();
    let mut last_frames = 0u64;
    let mut played_reports = 0;
    while t0.elapsed() < Duration::from_secs(8) {
        std::thread::sleep(Duration::from_millis(500));
        let s = engine.snapshot();
        println!(
            "[{:>4.1}s] {:?} pos={:.2}s level={:.3} err={:?}",
            t0.elapsed().as_secs_f32(),
            s.state,
            s.position,
            s.level,
            s.error
        );
        if s.state == rust_radio_audio::PlayState::Playing {
            played_reports += 1;
        }
        last_frames = (s.position * 1000.0) as u64;
    }
    engine.stop();
    if played_reports == 0 {
        eprintln!("FAIL：8 秒内没有进入播放状态");
        std::process::exit(1);
    }
    println!("PASS：进入播放状态，position≈{last_frames}ms");
}

fn dirs_cache() -> std::path::PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("rust-radio").join("cache")
}
