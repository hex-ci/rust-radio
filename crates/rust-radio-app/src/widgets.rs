//! 绘制原语：文字截断、圆角图片、渐变、胶囊、进度条。

use egui::{
    Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Shape, Stroke, TextureHandle, Ui,
    Vec2,
};

use crate::{audit, icons, icons::Icon, theme};

/// galley 里字形实际墨迹的外框（相对 galley 原点）。
///
/// `epaint::RowVisuals::mesh_bounds` 不含首尾空白，正好是「字看起来占的地方」，
/// 拿它做垂直居中比用行盒（ascent + descent）更接近视觉中心。
pub fn ink_rect(galley: &egui::Galley) -> Rect {
    let mut ink = Rect::NOTHING;
    for placed in &galley.rows {
        let bounds = placed.row.visuals.mesh_bounds;
        if bounds.is_positive() {
            ink = ink.union(bounds.translate(placed.pos.to_vec2()));
        }
    }
    if ink.is_positive() {
        ink
    } else {
        Rect::from_min_size(Pos2::ZERO, galley.size())
    }
}

/// 「图标 + 文字」：两者都按各自的墨迹中心对齐到同一条水平中线。
///
/// 横向锚点（图标中心 x、文字左缘 x）由调用方给，保持各视图自己的视觉节奏；
/// 垂直方向统一走这里，所以不会再出现「有的正好、有的偏 1px」。
/// 图标与文字可以不同色（例如品牌方块里的图标是 ON_ACCENT、文字是正文色）。
#[allow(clippy::too_many_arguments)]
pub fn icon_label(
    p: &Painter,
    center_y: f32,
    icon: Icon,
    icon_color: Color32,
    text: &str,
    font: FontId,
    text_color: Color32,
    icon_cx: f32,
    text_left: f32,
    icon_size: f32,
) -> Rect {
    icons::draw_ink_centered(p, Pos2::new(icon_cx, center_y), icon, icon_color, icon_size);

    let galley = p.layout_no_wrap(text.to_owned(), font, text_color);
    let ink = ink_rect(&galley);
    let pos = Pos2::new(text_left, center_y - ink.center().y);
    let text_rect = Rect::from_min_size(pos, galley.size()).intersect(p.clip_rect());
    p.galley(pos, galley, text_color);
    audit::text_rect(text, text_rect);
    text_rect
}

/// 单行截断：超出宽度时加省略号
pub fn ellipsize(ctx: &egui::Context, s: &str, font: &FontId, max_w: f32) -> String {
    let width = |t: &str| -> f32 {
        ctx.fonts_mut(|f| {
            f.layout_no_wrap(t.to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
    };
    if width(s) <= max_w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        if width(&candidate) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let mut out: String = chars[..lo].iter().collect();
    if lo < chars.len() {
        out.push('…');
    }
    out
}

pub fn gradient_v(p: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    let idx = mesh.vertices.len() as u32;
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(idx, idx + 1, idx + 2);
    mesh.add_triangle(idx, idx + 2, idx + 3);
    p.add(Shape::mesh(mesh));
}

pub fn gradient_h(p: &Painter, rect: Rect, left: Color32, right: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(Shape::mesh(mesh));
}

/// 圆角封面；没有图时画首字占位
pub fn cover(ui: &mut Ui, rect: Rect, radius: u8, tex: Option<TextureHandle>, title: &str) {
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(radius), theme::ELEV2);
    match tex {
        Some(t) => {
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            child.add(
                egui::Image::new((t.id(), rect.size()))
                    .corner_radius(CornerRadius::same(radius))
                    .maintain_aspect_ratio(false),
            );
        }
        None => {
            let ch = title
                .chars()
                .next()
                .map(|c| c.to_string())
                .unwrap_or_default();
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                ch,
                theme::bold(rect.width() * 0.32),
                theme::white_alpha(30),
            );
        }
    }
}

/// 直播红点的呼吸动画
pub fn live_dot(p: &Painter, center: Pos2, radius: f32, t: f32, color: Color32) {
    let pulse = 0.5 + 0.5 * (t * 2.2).sin();
    p.circle_filled(center, radius, color);
    p.circle_stroke(
        center,
        radius + 1.5 + pulse * 2.5,
        Stroke::new(1.0, color.gamma_multiply(0.35 * (1.0 - pulse) + 0.1)),
    );
}

/// 播放中的四段电平条
pub fn eq_bars(p: &Painter, rect: Rect, level: f32, t: f32, color: Color32) {
    let n = 4;
    let gap = rect.width() * 0.14;
    let bw = (rect.width() - gap * (n as f32 - 1.0)) / n as f32;
    for i in 0..n {
        let phase = t * (3.2 + i as f32 * 0.7) + i as f32;
        let amp = 0.35 + 0.65 * (0.5 + 0.5 * phase.sin()) * (0.4 + level * 2.0).min(1.0);
        let h = rect.height() * amp.clamp(0.12, 1.0);
        let x = rect.left() + i as f32 * (bw + gap);
        p.rect_filled(
            Rect::from_min_max(
                Pos2::new(x, rect.bottom() - h),
                Pos2::new(x + bw, rect.bottom()),
            ),
            bw * 0.35,
            color,
        );
    }
}

/// 细进度条（可拖动由调用方处理）
pub fn progress(p: &Painter, rect: Rect, frac: f32, bg: Color32, fg: Color32, knob: bool) {
    p.rect_filled(rect, rect.height() * 0.5, bg);
    let w = (rect.width() * frac.clamp(0.0, 1.0)).max(0.0);
    if w > 0.5 {
        p.rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(w, rect.height())),
            rect.height() * 0.5,
            fg,
        );
    }
    if knob && frac > 0.0 {
        p.circle_filled(
            Pos2::new(rect.left() + w, rect.center().y),
            (rect.height() * 1.6).max(3.5),
            Color32::WHITE,
        );
    }
}

/// 竖直渐隐遮罩（滚动区域上下边缘）
pub fn fade_mask(p: &Painter, rect: Rect, top: bool, color: Color32) {
    if top {
        gradient_v(p, rect, color, Color32::TRANSPARENT);
    } else {
        gradient_v(p, rect, Color32::TRANSPARENT, color);
    }
}

pub fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as i64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

pub fn fmt_clock(ts: &chrono::DateTime<chrono::Local>) -> String {
    use chrono::Timelike;
    format!("{:02}:{:02}", ts.hour(), ts.minute())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::Icon;
    use crate::theme;

    /// 在无窗口的 Context 里排版一次，验证「图标 + 文字」的墨迹中心都落在中线
    /// 上——这正是肉眼看到的 1px 偏移的量；顺带钉住它和行盒居中确实不是一个值。
    #[test]
    fn icon_label_centers_on_ink_not_line_box() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut checked = 0;

        let mut output = ctx.run_pass(egui::RawInput::default(), |ctx| {
            let p = ctx.layer_painter(egui::LayerId::background());
            let cases: [(Icon, &str, f32, f32); 3] = [
                (Icon::Play, "收听", 12.5, 13.0),
                (Icon::Pause, "正在收听", 12.5, 13.0),
                (Icon::Refresh, "换一个", 10.5, 12.0),
            ];

            for (icon, text, font_size, icon_size) in cases {
                let rect = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(160.0, 36.0));
                let center_y = rect.center().y;

                let text_rect = icon_label(
                    &p,
                    center_y,
                    icon,
                    theme::ON_ACCENT,
                    text,
                    theme::bold(font_size),
                    theme::ON_ACCENT,
                    rect.left() + 24.0,
                    rect.left() + 40.0,
                    icon_size,
                );

                let text_galley =
                    p.layout_no_wrap(text.to_owned(), theme::bold(font_size), theme::ON_ACCENT);
                let text_ink = ink_rect(&text_galley);
                assert!(
                    (text_rect.top() + text_ink.center().y - center_y).abs() < 0.01,
                    "{text} 的墨迹中心没对齐：{} vs {center_y}",
                    text_rect.top() + text_ink.center().y
                );

                // 同样的文字按行盒居中会偏多少：证明这不是 0 偏移的空操作
                let line_box_offset =
                    (text_rect.top() + text_galley.size().y * 0.5 - center_y).abs();
                assert!(
                    line_box_offset > 0.2,
                    "{text} 行盒与墨迹中心恰好一致（{line_box_offset}），测试失去意义"
                );

                let (glyph, fill) = icon.glyph();
                let icon_galley = p.layout_no_wrap(
                    glyph.to_owned(),
                    theme::icon_font(icon_size, fill),
                    theme::ON_ACCENT,
                );
                let icon_ink = ink_rect(&icon_galley);
                // 图标由 icon_label 内部绘制，这里只验证它用的墨迹框本身是贴合的
                assert!(icon_ink.height() > 0.0, "图标墨迹框为空");
                checked += 1;
            }
        });

        // 测试里没有渲染器消费纹理增量，丢弃前必须 clear，否则 epaint 会 panic
        output.textures_delta.clear();
        assert_eq!(checked, 3);
    }
}
