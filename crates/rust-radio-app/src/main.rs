#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod audit;
mod icons;
mod iconsheet;
mod images;
mod selftest;
mod store;
mod theme;
mod widgets;

fn app_icon() -> egui::IconData {
    // 和 .app 里的 AppIcon.icns 用同一张图（scripts/make-icon.swift 生成），
    // 避免启动弹跳时 bundle 图标与运行时图标不一致、落定后“缩一下”。
    let bytes = include_bytes!("../assets/app-icon-1024.png");
    let img = image::load_from_memory(bytes)
        .expect("内置应用图标损坏")
        .to_rgba8();
    let (width, height) = img.dimensions();
    egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }
}

fn main() -> eframe::Result<()> {
    let data_dir = store::data_dir();

    // `rust-radio --station 639` 启动即播（也方便自动化验收）
    if std::env::args().any(|a| a == "--selftest") {
        std::process::exit(selftest::run(data_dir));
    }

    let mut args = std::env::args().skip(1);
    let mut autoplay: Option<String> = None;
    let mut mini = false;
    let mut start_view: Option<String> = None;
    while let Some(arg) = args.next() {
        if arg == "--station" {
            autoplay = args.next();
        } else if let Some(rest) = arg.strip_prefix("--station=") {
            autoplay = Some(rest.to_string());
        } else if let Some(rest) = arg.strip_prefix("--size=") {
            if let Some((w, h)) = rest.split_once('x') {
                if let (Ok(w), Ok(h)) = (w.parse::<f32>(), h.parse::<f32>()) {
                    std::env::set_var("RUST_RADIO_SIZE", format!("{w}x{h}"));
                }
            }
        } else if arg == "--view" {
            start_view = args.next();
        } else if arg == "--audit" {
            audit::set_enabled(true);
        } else if arg == "--mini" {
            mini = true;
        } else if arg == "--icon-sheet" {
            let opts = eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default()
                    .with_inner_size([980.0, 620.0])
                    .with_title("图标检查"),
                ..Default::default()
            };
            return eframe::run_native(
                "图标检查",
                opts,
                Box::new(|cc| {
                    theme::install_fonts(&cc.egui_ctx);
                    theme::install_style(&cc.egui_ctx);
                    Ok(Box::new(iconsheet::IconSheet))
                }),
            );
        }
    }
    let mut size = if mini {
        [420.0, 140.0]
    } else {
        [1280.0, 800.0]
    };
    if let Ok(v) = std::env::var("RUST_RADIO_SIZE") {
        if let Some((w, h)) = v.split_once('x') {
            if let (Ok(w), Ok(h)) = (w.trim().parse::<f32>(), h.trim().parse::<f32>()) {
                size = [w, h];
            }
        }
    }

    // 无系统标题栏：macOS 用「透明标题栏 + 全尺寸内容」保留原生红绿灯；
    // Windows / Linux 用无边框窗口，标题与最小化/最大化/关闭按钮由应用自绘。
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(size)
        .with_min_inner_size(if mini { [400.0, 120.0] } else { [900.0, 560.0] })
        .with_title("云听电台")
        .with_icon(app_icon());
    if cfg!(target_os = "macos") {
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_titlebar_buttons_shown(true);
    } else {
        // Windows/Linux 没有 .app 图标，窗口图标由程序提供；macOS 用 bundle 里的 AppIcon
        viewport = viewport.with_decorations(false);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "云听电台",
        options,
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            theme::install_style(&cc.egui_ctx);
            Ok(Box::new(app::App::new(
                cc,
                data_dir,
                autoplay.clone(),
                start_view.clone(),
            )))
        }),
    )
}
