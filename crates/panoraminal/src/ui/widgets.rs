//! Small custom widgets in the Panoraminal style.

use crate::icons::{self, Glyph};
use crate::theme::{self, brand, with_alpha, Palette};
use eframe::egui::{self, Color32, FontId, Rect, Response, Sense, Stroke, Vec2};

/// Opens an https link in the system browser. Anything else is refused.
pub fn open_url(ctx: &egui::Context, url: &str) {
    if url.starts_with("https://") {
        ctx.open_url(egui::OpenUrl::new_tab(url));
    } else {
        log::warn!("refusing to open non-https url {url}");
    }
}

/// A square icon button with gradient fill when selected.
pub fn icon_button(
    ui: &mut egui::Ui,
    pal: &Palette,
    glyph: Glyph,
    selected: bool,
    tooltip: &str,
    size: f32,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let resp = resp.on_hover_text(tooltip);
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let hover = ui
            .ctx()
            .animate_bool(resp.id.with("h"), resp.hovered() && enabled);
        let sel = ui.ctx().animate_bool(resp.id.with("s"), selected);
        let p = ui.painter();
        let r = (size * 0.26).round();
        if sel > 0.0 {
            let stops = [with_alpha(brand::ROYAL, sel), with_alpha(brand::AZURE, sel)];
            p.rect_filled(
                rect,
                r,
                theme::lerp_color(Color32::TRANSPARENT, brand::ROYAL, sel),
            );
            theme::gradient_pill(p, rect.shrink(1.0), &stops, r);
        }
        if hover > 0.0 && !selected {
            p.rect_filled(
                rect,
                r,
                with_alpha(theme::lerp_color(pal.raised, pal.accent, 0.25), hover),
            );
            p.rect_stroke(
                rect,
                r,
                Stroke::new(1.0, with_alpha(pal.neon, hover * 0.8)),
                egui::StrokeKind::Inside,
            );
        }
        let fg = if selected {
            Color32::WHITE
        } else if !enabled {
            with_alpha(pal.text_weak, 0.45)
        } else {
            theme::lerp_color(pal.text_weak, pal.text, hover.max(0.25))
        };
        let g = Rect::from_center_size(rect.center(), Vec2::splat(size * 0.56));
        icons::draw(p, g, glyph, fg);
    }
    resp
}

pub fn section_title(ui: &mut egui::Ui, pal: &Palette, text: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::new(4.0, 14.0), Sense::hover());
        theme::gradient_rect(ui.painter(), r, &[brand::CYAN, brand::ROYAL], false);
        ui.label(
            egui::RichText::new(text.to_uppercase())
                .font(FontId::proportional(12.5))
                .strong()
                .color(if pal.dark { brand::ICE } else { brand::DEEP }),
        );
    });
    ui.add_space(4.0);
}

/// A prominent gradient call-to-action button.
pub fn primary_button(ui: &mut egui::Ui, pal: &Palette, text: &str, glow: f32) -> Response {
    let font = FontId::proportional(14.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let size = Vec2::new(galley.size().x + 28.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool(resp.id, resp.hovered());
        let p = ui.painter();
        theme::glow_rect(p, rect, 10.0, pal.neon, (0.25 + 0.5 * hover) * glow);
        let stops = if resp.is_pointer_button_down_on() {
            [brand::DEEP, brand::ROYAL]
        } else {
            [
                brand::ROYAL,
                theme::lerp_color(brand::AZURE, brand::CYAN, hover * 0.5),
            ]
        };
        theme::gradient_pill(p, rect, &stops, 10.0);
        p.galley(rect.center() - galley.size() * 0.5, galley, Color32::WHITE);
    }
    resp
}

/// iOS-like toggle switch with a neon knob.
pub fn toggle(ui: &mut egui::Ui, pal: &Palette, on: &mut bool, label: &str) -> Response {
    let resp = ui
        .horizontal(|ui| {
            let size = Vec2::new(34.0, 18.0);
            let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
            if resp.clicked() {
                *on = !*on;
                resp.mark_changed();
            }
            let t = ui.ctx().animate_bool(resp.id, *on);
            let p = ui.painter();
            let bg = theme::lerp_color(pal.raised, pal.accent2, t);
            p.rect_filled(rect, 9.0, bg);
            p.rect_stroke(
                rect,
                9.0,
                Stroke::new(1.0, theme::lerp_color(pal.stroke, pal.neon, t)),
                egui::StrokeKind::Inside,
            );
            let x = egui::lerp(rect.left() + 9.0..=rect.right() - 9.0, t);
            let c = egui::pos2(x, rect.center().y);
            if t > 0.5 {
                theme::glow_circle(p, c, 6.0, pal.neon, 0.4 * t);
            }
            p.circle_filled(
                c,
                6.5,
                if t > 0.5 {
                    Color32::WHITE
                } else {
                    pal.text_weak
                },
            );
            let l = ui.add(egui::Label::new(label).sense(Sense::click()));
            if l.clicked() {
                *on = !*on;
                resp.mark_changed();
            }
            resp
        })
        .inner;
    resp
}

/// Label + slider on one row, the slider filling the remaining width.
pub fn slider_row<N: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut N,
    range: std::ops::RangeInclusive<N>,
    suffix: &str,
    log: bool,
) -> Response {
    ui.horizontal(|ui| {
        ui.add_sized(
            [86.0, 18.0],
            egui::Label::new(egui::RichText::new(label).size(12.5)).truncate(),
        );
        ui.spacing_mut().slider_width = (ui.available_width() - 62.0).max(60.0);
        let mut s = egui::Slider::new(value, range).suffix(suffix);
        if log {
            s = s.logarithmic(true);
        }
        ui.add(s)
    })
    .inner
}
