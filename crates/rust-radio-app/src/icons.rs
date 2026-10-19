//! 图标：使用 Phosphor Icons（MIT）字体，不再手绘路径。
//!
//! 字体来自 egui-phosphor 的资源包；常量取上游 `regular.rs` / `fill.rs` 的码点。
//! 想换图标时改这里的一行即可，其余 UI 代码只认 `Icon` 枚举。

use egui::{Color32, Painter, Rect};

use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Play,
    Pause,
    Prev,
    Next,
    Heart,
    HeartFill,
    Search,
    Radio,
    Compass,
    MapPin,
    Clock,
    Gear,
    ChevronRight,
    ChevronDown,
    Grid,
    List,
    Refresh,
    Panel,
    Volume,
    VolumeLow,
    Mute,
    Mini,
    Timer,
    X,
    Minus,
    Square,
    Dots,
    Star,
    Rewind,
    Broadcast,
    Check,
    Back,
    Expand,
}

impl Icon {
    /// (Phosphor 码点, 是否用实心字重)
    fn glyph(self) -> (&'static str, bool) {
        match self {
            // 媒体控制用实心，视觉更稳
            Icon::Play => ("\u{E3D0}", true),          // PLAY
            Icon::Pause => ("\u{E39E}", true),         // PAUSE
            Icon::Prev => ("\u{E5A4}", true),          // SKIP_BACK
            Icon::Next => ("\u{E5A6}", true),          // SKIP_FORWARD
            Icon::Heart => ("\u{E2A8}", false),        // HEART
            Icon::HeartFill => ("\u{E2A8}", true),     // HEART (fill)
            Icon::Search => ("\u{E30C}", false),       // MAGNIFYING_GLASS
            Icon::Radio => ("\u{E77E}", false),        // RADIO
            Icon::Compass => ("\u{E1C8}", false),      // COMPASS
            Icon::MapPin => ("\u{E316}", false),       // MAP_PIN
            Icon::Clock => ("\u{E19A}", false),        // CLOCK
            Icon::Gear => ("\u{E270}", false),         // GEAR
            Icon::ChevronRight => ("\u{E13A}", false), // CARET_RIGHT
            Icon::ChevronDown => ("\u{E136}", false),  // CARET_DOWN
            Icon::Grid => ("\u{E464}", false),         // SQUARES_FOUR
            Icon::List => ("\u{E2F0}", false),         // LIST
            Icon::Refresh => ("\u{E094}", false),      // ARROWS_CLOCKWISE
            Icon::Panel => ("\u{EC24}", false),        // SIDEBAR_SIMPLE
            Icon::Volume => ("\u{E44A}", false),       // SPEAKER_HIGH
            Icon::VolumeLow => ("\u{E44C}", false),    // SPEAKER_LOW
            Icon::Mute => ("\u{E45A}", false),         // SPEAKER_SLASH
            Icon::Mini => ("\u{E64C}", false),         // PICTURE_IN_PICTURE
            Icon::Timer => ("\u{E492}", false),        // TIMER
            Icon::X => ("\u{E4F6}", false),            // X
            Icon::Minus => ("\u{E32A}", false),        // MINUS
            Icon::Square => ("\u{E45E}", false),       // SQUARE
            Icon::Dots => ("\u{E1FE}", false),         // DOTS_THREE
            Icon::Star => ("\u{E46A}", true),          // STAR (fill)
            Icon::Rewind => ("\u{E038}", false),       // ARROW_COUNTER_CLOCKWISE
            Icon::Broadcast => ("\u{E0F2}", false),    // BROADCAST
            Icon::Check => ("\u{E182}", false),        // CHECK
            Icon::Back => ("\u{E058}", false),         // ARROW_LEFT
            Icon::Expand => ("\u{E0A2}", false),       // ARROWS_OUT
        }
    }
}

/// 在矩形内居中绘制图标。`size` 是**字形像素高度**（不是按钮框大小）：
/// 行内/工具条 16，播放键 16–18，小圆钮 12–14，大图标 20。
pub fn draw(p: &Painter, rect: Rect, icon: Icon, color: Color32, size: f32) {
    if std::env::var_os("RUST_RADIO_NOICONS").is_some() {
        return;
    }
    let (glyph, fill) = icon.glyph();
    let size = size.max(6.0);
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        theme::icon_font(size, fill),
        color,
    );
}

/// 所有图标的清单（图标检查窗用）
pub const ALL: &[Icon] = &[
    Icon::Play,
    Icon::Pause,
    Icon::Prev,
    Icon::Next,
    Icon::Heart,
    Icon::HeartFill,
    Icon::Search,
    Icon::Radio,
    Icon::Compass,
    Icon::MapPin,
    Icon::Clock,
    Icon::Gear,
    Icon::ChevronRight,
    Icon::ChevronDown,
    Icon::Grid,
    Icon::List,
    Icon::Refresh,
    Icon::Panel,
    Icon::Volume,
    Icon::VolumeLow,
    Icon::Mute,
    Icon::Mini,
    Icon::Timer,
    Icon::X,
    Icon::Minus,
    Icon::Square,
    Icon::Dots,
    Icon::Star,
    Icon::Rewind,
    Icon::Broadcast,
    Icon::Check,
    Icon::Back,
    Icon::Expand,
];
