//! 端到端自检：`rust-radio --selftest`
//!
//! 不需要点界面，直接验证：接口 → 索引 → 搜索 → 直播解码输出 → 节目单 → 回听跳转。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rust_radio_audio::{Engine, PlayState, Source};
use rust_radio_core::{Client, Freshness, Line, Quality, StreamKind};

pub fn run(data_dir: PathBuf) -> i32 {
    let mut failures = 0;
    let mut total = 0;
    let cache = data_dir.join("cache");
    let client = match Client::new(cache) {
        Ok(c) => c,
        Err(e) => {
            println!("FAIL  客户端初始化：{e}");
            return 1;
        }
    };

    macro_rules! check {
        ($name:expr, $ok:expr, $detail:expr) => {{
            let ok = $ok;
            total += 1;
            println!(
                "{}  {:<28} {}",
                if ok { "PASS" } else { "FAIL" },
                $name,
                $detail
            );
            if !ok {
                failures += 1;
            }
        }};
    }

    let t0 = Instant::now();
    let provinces = client.provinces().unwrap_or_default();
    check!(
        "省份列表",
        provinces.len() == 31,
        format!("{} 个", provinces.len())
    );

    let national = client.national().unwrap_or_default();
    check!(
        "央广频率",
        national.len() == 19,
        format!("{} 个", national.len())
    );

    let (all, fresh) = match client.all_stations(false) {
        Ok(v) => v,
        Err(e) => {
            println!("FAIL  全国索引：{e}");
            return 1;
        }
    };
    check!(
        "全国索引",
        all.len() > 900,
        format!(
            "{} 个台 · {:?} · {:.1}s",
            all.len(),
            fresh,
            t0.elapsed().as_secs_f32()
        )
    );

    let (hit, _) = client.search_stations("交通", 0, 40).unwrap_or_default();
    check!(
        "服务端搜索（交通）",
        !hit.is_empty(),
        format!(
            "{} 条，首个 {}",
            hit.len(),
            hit.first().map(|s| s.title.as_str()).unwrap_or("-")
        )
    );
    let (hit, _) = client.search_stations("zgzs", 0, 40).unwrap_or_default();
    check!(
        "服务端搜索（拼音 zgzs）",
        hit.first().map(|s| s.content_id.as_str()) == Some("639"),
        format!(
            "首条 {}",
            hit.first().map(|s| s.title.as_str()).unwrap_or("-")
        )
    );

    // 服务端搜索（只搜电台）：tab=4 / contentType=5，必须带 productId 头
    let (server, _) = client.search_stations("新闻", 0, 40).unwrap_or_default();
    check!(
        "服务端搜索（电台）",
        !server.is_empty(),
        format!(
            "{} 条，首个 {}",
            server.len(),
            server.first().map(|s| s.title.as_str()).unwrap_or("-")
        )
    );

    let (detail, target) = match client.fresh_stream("639", Quality::High, Line::Auto) {
        Ok(v) => v,
        Err(e) => {
            println!("FAIL  解析播放地址：{e}");
            return 1;
        }
    };
    check!(
        "直播选流",
        target.kind == StreamKind::Hls && target.label.contains("高清"),
        format!("{} / {}", detail.name, target.label)
    );

    let engine = Engine::new().expect("audio engine");
    engine.set_volume(0.35);
    engine.play(Source {
        url: target.url.clone(),
        kind: target.kind,
        title: detail.name.clone(),
        subtitle: String::new(),
        duration: None,
        start_at: 0.0,
    });
    let played = wait_playing(&engine, Duration::from_secs(12));
    check!("直播出声", played, {
        let s = engine.snapshot();
        format!("{:?} pos={:.1}s level={:.3}", s.state, s.position, s.level)
    });
    engine.stop();
    std::thread::sleep(Duration::from_millis(400));

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let (programs, _) = client
        .programs("639", &today)
        .unwrap_or((Vec::new(), Freshness::Live));
    let playable = programs.iter().filter(|p| p.replay_url().is_some()).count();
    check!(
        "今日节目单",
        programs.len() > 10,
        format!("{} 档，{} 档可回听", programs.len(), playable)
    );

    let yesterday = (chrono::Local::now().date_naive() - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    let (yest, _) = client
        .programs("639", &yesterday)
        .unwrap_or((Vec::new(), Freshness::Live));
    check!("昨日节目单", yest.len() > 10, format!("{} 档", yest.len()));

    // 刚过零点时当天还没有生成回听地址，退回昨天找一档可回听的节目
    let replay_src = programs
        .iter()
        .find(|p| p.replay_url().is_some())
        .or_else(|| yest.iter().rev().find(|p| p.replay_url().is_some()))
        .cloned();

    if let Some(prog) = replay_src {
        let url = prog.replay_url().unwrap().to_string();
        engine.play(Source {
            url,
            kind: StreamKind::File,
            title: "中国之声".into(),
            subtitle: prog.program_name.clone(),
            duration: Some(prog.duration_secs() as f64),
            start_at: 60.0,
        });
        let ok = wait_playing(&engine, Duration::from_secs(15));
        let s = engine.snapshot();
        check!(
            "回听从 60s 起播",
            ok && s.position > 59.5,
            format!(
                "{} pos={:.1}s/{:.0}s",
                prog.program_name,
                s.position,
                s.duration.unwrap_or(0.0)
            )
        );

        engine.seek(300.0);
        std::thread::sleep(Duration::from_millis(1200));
        let s = engine.snapshot();
        check!(
            "回听拖动到 300s",
            s.position > 300.0,
            format!("pos={:.1}s", s.position)
        );
        engine.stop();
    } else {
        check!("回听", false, "今天和昨天都没有可回听节目");
    }

    println!();
    if failures == 0 {
        println!("全部通过（{total} 项）");
        0
    } else {
        println!("{failures} 项失败");
        1
    }
}

fn wait_playing(engine: &Engine, limit: Duration) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < limit {
        let s = engine.snapshot();
        if s.state == PlayState::Playing && s.position > 1.0 {
            return true;
        }
        if s.state == PlayState::Error {
            eprintln!("   错误：{:?}", s.error);
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}
