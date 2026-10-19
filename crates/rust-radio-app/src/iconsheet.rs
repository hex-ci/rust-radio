//! 图标检查窗：把所有图标按实际尺寸（16 / 24 / 64）铺开，便于肉眼核对。

use egui::{CornerRadius, Rect, RichText, Sense, Stroke, Vec2};

use crate::icons::{self, ALL};
use crate::theme;

#[derive(Default)]
pub struct IconSheet;

impl eframe::App for IconSheet {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(ui, |ui| {
                ui.label(
                    RichText::new("图标检查 · 64 / 24 / 16 px")
                        .size(14.0)
                        .strong(),
                );
                ui.add_space(10.0);
                let cols = 5;
                let cell = Vec2::new(176.0, 104.0);
                for chunk in ALL.chunks(cols) {
                    ui.horizontal(|ui| {
                        for icon in chunk {
                            let (rect, _) = ui.allocate_exact_size(cell, Sense::hover());
                            let p = ui.painter();
                            p.rect(
                                rect.shrink(4.0),
                                CornerRadius::same(10),
                                theme::PANEL,
                                Stroke::new(1.0, theme::white_alpha(16)),
                                egui::StrokeKind::Inside,
                            );
                            let big = Rect::from_min_size(
                                rect.min + Vec2::new(14.0, 14.0),
                                Vec2::splat(56.0),
                            );
                            icons::draw(p, big, *icon, theme::TEXT, 1.6);
                            let mid = Rect::from_min_size(
                                rect.min + Vec2::new(84.0, 20.0),
                                Vec2::splat(24.0),
                            );
                            icons::draw(p, mid, *icon, theme::DIM, 1.7);
                            let small = Rect::from_min_size(
                                rect.min + Vec2::new(120.0, 24.0),
                                Vec2::splat(16.0),
                            );
                            icons::draw(p, small, *icon, theme::DIM, 1.7);
                            p.text(
                                rect.min + Vec2::new(14.0, 80.0),
                                egui::Align2::LEFT_CENTER,
                                format!("{icon:?}"),
                                theme::regular(10.5),
                                theme::WEAK,
                            );
                        }
                    });
                    ui.add_space(6.0);
                }
            });
    }
}
