//! 设计系统：配色、字体（含 Phosphor 图标字体）、圆角、间距。

use std::sync::Arc;

use egui::{
    Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke,
    TextStyle, Theme, ThemePreference,
};

// ---------------------------------------------------------------- 颜色

pub const BG: Color32 = Color32::from_rgb(0x08, 0x09, 0x0b);
pub const PANEL: Color32 = Color32::from_rgb(0x0f, 0x11, 0x15);
pub const ELEV: Color32 = Color32::from_rgb(0x15, 0x17, 0x1c);
pub const ELEV2: Color32 = Color32::from_rgb(0x1b, 0x1e, 0x24);
pub const TEXT: Color32 = Color32::from_rgb(0xe9, 0xea, 0xec);
pub const DIM: Color32 = Color32::from_rgb(0xa8, 0xae, 0xb6);
pub const WEAK: Color32 = Color32::from_rgb(0x8a, 0x91, 0x99);
pub const FAINT: Color32 = Color32::from_rgb(0x5e, 0x64, 0x6c);
pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0x5a, 0x3c);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0xff, 0x70, 0x55);
pub const AMBER: Color32 = Color32::from_rgb(0xff, 0x9a, 0x3d);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x1a, 0x09, 0x06);

pub fn accent_alpha(a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(0xff, 0x5a, 0x3c, a)
}
pub fn amber_alpha(a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(0xff, 0x9a, 0x3d, a)
}
pub fn white_alpha(a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(255, 255, 255, a)
}

// ---------------------------------------------------------------- 圆角 / 尺寸

pub const R_CARD: u8 = 12;
pub const R_ART: u8 = 9;
pub const R_BTN: u8 = 8;
pub const R_PILL: u8 = 24;

pub const SIDEBAR_W: f32 = 232.0;
pub const PANEL_W: f32 = 300.0;
pub const PLAYER_H: f32 = 80.0;
pub const TOOLBAR_H: f32 = 56.0;

// ---------------------------------------------------------------- 字体

pub const F_BOLD: &str = "bold";
pub const F_ICONS: &str = "icons";
pub const F_ICONS_FILL: &str = "icons-fill";

/// 系统字体（拉丁 + 中文回退 + 真粗体）+ Phosphor 图标字体。
pub fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();

    let (latin, latin_bold, cjk, cjk_bold) = system_font_candidates();

    let mut prop: Vec<String> = Vec::new();
    let mut bold: Vec<String> = Vec::new();

    if let Some((bytes, idx)) = read_first(&latin) {
        let mut fd = FontData::from_owned(bytes);
        fd.index = idx;
        fonts.font_data.insert("sans".into(), Arc::new(fd));
        prop.push("sans".into());
        bold.push("sans".into());
    }
    if let Some((bytes, idx)) = read_first(&cjk) {
        let mut fd = FontData::from_owned(bytes);
        fd.index = idx;
        fonts.font_data.insert("cjk".into(), Arc::new(fd));
        prop.push("cjk".into());
        bold.push("cjk".into());
    }
    if let Some((bytes, idx)) = read_first(&latin_bold) {
        let mut fd = FontData::from_owned(bytes);
        fd.index = idx;
        fonts.font_data.insert("sans-bold".into(), Arc::new(fd));
        bold.insert(0, "sans-bold".into());
    }
    if let Some((bytes, idx)) = read_first(&cjk_bold) {
        let mut fd = FontData::from_owned(bytes);
        fd.index = idx;
        fonts.font_data.insert("cjk-bold".into(), Arc::new(fd));
        bold.insert(0, "cjk-bold".into());
    }

    // 图标字体（Phosphor，MIT）
    fonts.font_data.insert(
        "phosphor".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/phosphor/Phosphor.ttf"
        ))),
    );
    fonts.font_data.insert(
        "phosphor-bold".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/phosphor/Phosphor-Bold.ttf"
        ))),
    );
    fonts.font_data.insert(
        "phosphor-fill".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/phosphor/Phosphor-Fill.ttf"
        ))),
    );

    let default_prop = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();

    let mut prop_final = prop.clone();
    prop_final.extend(default_prop.clone());
    let mut bold_final = bold.clone();
    bold_final.extend(prop.clone());
    bold_final.extend(default_prop);

    fonts.families.insert(FontFamily::Proportional, prop_final);
    fonts
        .families
        .insert(FontFamily::Name(F_BOLD.into()), bold_final);
    fonts.families.insert(
        FontFamily::Name(F_ICONS.into()),
        vec!["phosphor-bold".into(), "phosphor".into()],
    );
    fonts.families.insert(
        FontFamily::Name(F_ICONS_FILL.into()),
        vec!["phosphor-fill".into()],
    );

    ctx.set_fonts(fonts);
}

type FontChoice = (std::path::PathBuf, u32);

fn read_first(paths: &[FontChoice]) -> Option<(Vec<u8>, u32)> {
    for (p, idx) in paths {
        if let Ok(bytes) = std::fs::read(p) {
            if !bytes.is_empty() {
                return Some((bytes, *idx));
            }
        }
    }
    None
}

fn choices(list: &[(&str, u32)]) -> Vec<FontChoice> {
    list.iter()
        .map(|(p, i)| (std::path::PathBuf::from(p), *i))
        .collect()
}

/// macOS 26 把 PingFang 挪到了 AssetsV2，固定路径已经不存在；找不到就退回 STHeiti。
#[cfg(target_os = "macos")]
fn pingfang() -> Option<std::path::PathBuf> {
    let fixed = std::path::PathBuf::from("/System/Library/Fonts/PingFang.ttc");
    if fixed.is_file() {
        return Some(fixed);
    }
    let base = std::path::Path::new("/System/Library/AssetsV2");
    for entry in std::fs::read_dir(base).ok()?.flatten() {
        let dir = entry.path();
        if !dir
            .file_name()
            .map(|n| {
                n.to_string_lossy()
                    .starts_with("com_apple_MobileAsset_Font")
            })
            .unwrap_or(false)
        {
            continue;
        }
        for sub in std::fs::read_dir(&dir).ok().into_iter().flatten().flatten() {
            let p = sub.path().join("AssetData").join("PingFang.ttc");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// 非 macOS 平台不会走到这个分支，但 `cfg!` 是运行时判断，函数仍需存在。
#[cfg(not(target_os = "macos"))]
fn pingfang() -> Option<std::path::PathBuf> {
    None
}

fn system_font_candidates() -> (
    Vec<FontChoice>,
    Vec<FontChoice>,
    Vec<FontChoice>,
    Vec<FontChoice>,
) {
    if cfg!(windows) {
        (
            choices(&[
                (r"C:\Windows\Fonts\segoeui.ttf", 0),
                (r"C:\Windows\Fonts\segoeuil.ttf", 0),
            ]),
            choices(&[
                (r"C:\Windows\Fonts\segoeuib.ttf", 0),
                (r"C:\Windows\Fonts\seguisb.ttf", 0),
            ]),
            choices(&[
                (r"C:\Windows\Fonts\msyh.ttc", 0),
                (r"C:\Windows\Fonts\msyhl.ttc", 0),
                (r"C:\Windows\Fonts\simhei.ttf", 0),
            ]),
            choices(&[
                (r"C:\Windows\Fonts\msyhbd.ttc", 0),
                (r"C:\Windows\Fonts\msyh.ttc", 0),
            ]),
        )
    } else if cfg!(target_os = "macos") {
        let cjk = pingfang();
        // PingFang.ttc 里 SC Regular = 3，SC Semibold = 11
        let cjk_regular = cjk
            .clone()
            .map(|p| vec![(p, 3)])
            .unwrap_or_else(|| choices(&[("/System/Library/Fonts/STHeiti Medium.ttc", 1)]));
        let cjk_bold = cjk
            .map(|p| vec![(p, 11)])
            .unwrap_or_else(|| choices(&[("/System/Library/Fonts/STHeiti Medium.ttc", 1)]));
        (
            choices(&[
                ("/System/Library/Fonts/SFNS.ttf", 0),
                ("/System/Library/Fonts/HelveticaNeue.ttc", 0),
            ]),
            choices(&[
                ("/System/Library/Fonts/HelveticaNeue.ttc", 1),
                ("/System/Library/Fonts/Helvetica.ttc", 1),
            ]),
            cjk_regular,
            cjk_bold,
        )
    } else {
        (
            choices(&[
                ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0),
                ("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf", 0),
            ]),
            choices(&[
                ("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 0),
                ("/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf", 0),
            ]),
            choices(&[
                ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
                ("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc", 0),
                ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 0),
                ("/usr/share/fonts/truetype/wqy/wqy-microhei.ttc", 0),
            ]),
            choices(&[
                ("/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc", 0),
                ("/usr/share/fonts/truetype/noto/NotoSansCJK-Bold.ttc", 0),
                ("/usr/share/fonts/noto-cjk/NotoSansCJK-Bold.ttc", 0),
            ]),
        )
    }
}

// ---------------------------------------------------------------- 字号

/// 字号取整：1x 屏上按整像素栅格化，避免半像素字号的模糊/锯齿
fn px(size: f32) -> f32 {
    size.round().max(10.0)
}
pub fn regular(size: f32) -> FontId {
    FontId::new(px(size), FontFamily::Proportional)
}
pub fn bold(size: f32) -> FontId {
    FontId::new(px(size), FontFamily::Name(F_BOLD.into()))
}
pub fn icon_font(size: f32, fill: bool) -> FontId {
    let fam = if fill { F_ICONS_FILL } else { F_ICONS };
    FontId::new(px(size), FontFamily::Name(fam.into()))
}

pub const T_SMALL: f32 = 11.5;
pub const T_BODY: f32 = 13.0;
pub const T_H3: f32 = 14.0;

// ---------------------------------------------------------------- 样式

pub fn install_style(ctx: &Context) {
    ctx.set_theme(ThemePreference::Dark);

    let mut style = (*ctx.style_of(Theme::Dark)).clone();

    style.visuals.dark_mode = true;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = ELEV;
    style.visuals.extreme_bg_color = Color32::from_rgb(0x0b, 0x0d, 0x10);
    style.visuals.faint_bg_color = ELEV;
    style.visuals.code_bg_color = ELEV2;
    style.visuals.window_stroke = Stroke::new(1.0, white_alpha(18));
    style.visuals.selection.bg_fill = accent_alpha(60);
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    style.visuals.hyperlink_color = AMBER;

    let w = &mut style.visuals.widgets;
    for state in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        state.corner_radius = CornerRadius::same(R_BTN);
    }
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, white_alpha(16));
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);

    w.inactive.bg_fill = ELEV;
    w.inactive.weak_bg_fill = ELEV;
    w.inactive.bg_stroke = Stroke::new(1.0, white_alpha(18));
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);

    w.hovered.bg_fill = ELEV2;
    w.hovered.weak_bg_fill = ELEV2;
    w.hovered.bg_stroke = Stroke::new(1.0, white_alpha(30));
    w.hovered.fg_stroke = Stroke::new(1.0, TEXT);

    w.active.bg_fill = ELEV2;
    w.active.weak_bg_fill = ELEV2;
    w.active.bg_stroke = Stroke::new(1.0, accent_alpha(120));
    w.active.fg_stroke = Stroke::new(1.0, TEXT);

    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.menu_margin = egui::Margin::same(6);
    style.spacing.scroll.bar_width = 6.0;
    style.spacing.scroll.floating = false;
    style.spacing.interact_size.y = 28.0;

    style.text_styles = [
        (TextStyle::Small, regular(T_SMALL)),
        (TextStyle::Body, regular(T_BODY)),
        (TextStyle::Button, regular(T_BODY)),
        (TextStyle::Heading, bold(T_H3)),
        (
            TextStyle::Monospace,
            FontId::new(T_SMALL, FontFamily::Monospace),
        ),
    ]
    .into();

    ctx.set_style_of(Theme::Dark, style);
}
