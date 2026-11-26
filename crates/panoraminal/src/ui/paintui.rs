//! Paint style controls of the brush card (1.3): styles, tip shapes with previews,
//! paint mixing, spray particles, paper texture and pressure options, plus a live
//! preview stroke rendered with the real brush engine.

use super::widgets::{slider_row, toggle};
use crate::app::PanoApp;
use crate::settings::UserStyle;
use crate::theme::{self, with_alpha, Palette};
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2};
use panoraminal_core::math::px_to_dir;
use panoraminal_core::{
    builtin_styles, BrushSettings, Document, PaperTexture, StrokeEngine, TipShape, ToolKind,
};
use panoraminal_i18n::tr;
use std::collections::HashMap;

/// UI state and texture caches of the paint style controls.
#[derive(Default)]
pub struct PaintUi {
    tips: HashMap<(TipShape, bool), egui::TextureHandle>,
    preview: Option<(String, egui::TextureHandle)>,
    /// Name being typed for a new user style.
    pub style_name: Option<String>,
}

fn pct(ui: &mut egui::Ui, label: &str, v: &mut f32, lo: f32, hi: f32) -> egui::Response {
    let mut x = *v * 100.0;
    let r = slider_row(ui, label, &mut x, lo..=hi, "%", false);
    if r.changed() {
        *v = x / 100.0;
    }
    r
}

/// Renders a short preview stroke of `settings` over two colour bands (so mixing,
/// dilution and erasing are visible). Returns `w`×`h` RGBA.
pub fn stroke_preview(
    tool: ToolKind,
    settings: &BrushSettings,
    rgb: [u8; 3],
    dark: bool,
    w: u32,
    h: u32,
) -> Vec<u8> {
    // Document wide enough that the preview spans ~0.9 rad at the equator.
    let dw = ((w as f64 / 0.9) * std::f64::consts::TAU).round() as u32 & !1;
    let dh = dw / 2;
    let mut doc = Document::empty(dw, dh);
    let l = doc.new_layer("preview");
    doc.active = l.id;
    let id = l.id;
    doc.layers.push(l);
    let x0 = dw / 2 - w / 2;
    let y0 = dh / 2 - h / 2;
    // colour bands on the painted layer
    if let Some(l) = doc.layer_mut(id) {
        let bands = [
            (0.22, 0.40, [0x1b, 0x4f, 0xd8]),
            (0.52, 0.70, [0xe0, 0x2a, 0x3c]),
        ];
        for (a, b, c) in bands {
            let xa = x0 + (w as f32 * a) as u32;
            let xb = x0 + (w as f32 * b) as u32;
            for y in y0..y0 + h {
                for x in xa..xb {
                    let i = ((y * dw + x) * 4) as usize;
                    l.pixels[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                }
            }
        }
    }
    let mut eng = StrokeEngine::new();
    eng.seed = 11;
    let mut s = *settings;
    s.size = s.size.clamp(3.0, h as f32 * 0.55);
    s.smoothing = 0.0;
    s.polar_correction = true;
    let pt = |t: f64| {
        let x = x0 as f64 + w as f64 * (0.06 + 0.88 * t);
        let y = y0 as f64 + h as f64 * (0.5 - 0.2 * (t * std::f64::consts::TAU).sin());
        px_to_dir(x, y, dw, dh)
    };
    let pr = |t: f64| (0.45 + 0.55 * (t * std::f64::consts::PI).sin()) as f32;
    let n = 90;
    if eng.begin(&doc, tool, s, rgb, pt(0.0), pr(0.0)) {
        for i in 1..=n {
            let t = i as f64 / n as f64;
            eng.add_point(pt(t), pr(t), i == n);
            eng.flush(&mut doc);
        }
        eng.end(&mut doc);
    }
    let bg = if dark {
        [0x05, 0x0b, 0x22]
    } else {
        [0xee, 0xf5, 0xfd]
    };
    let mut out = vec![0u8; (w * h * 4) as usize];
    if let Some(l) = doc.layer(id) {
        for y in 0..h {
            for x in 0..w {
                let si = (((y + y0) * dw + x + x0) * 4) as usize;
                let di = ((y * w + x) * 4) as usize;
                let a = l.pixels[si + 3] as f32 / 255.0;
                for k in 0..3 {
                    let v = l.pixels[si + k] as f32 * a + bg[k] as f32 * (1.0 - a);
                    out[di + k] = (v + 0.5) as u8;
                }
                out[di + 3] = 255;
            }
        }
    }
    out
}

fn style_name(id: &str) -> String {
    tr(&format!("style.{id}")).to_string()
}

/// Live preview of the current settings, plus the style chips and user styles.
pub fn styles(app: &mut PanoApp, ui: &mut egui::Ui, kind: ToolKind) {
    let pal = app.pal;
    let Some(b) = app.brushes.get(&kind).copied() else {
        return;
    };
    // live preview (re-rendered only when something changes)
    let pw = (ui.available_width().max(120.0)) as u32;
    let ph = 56u32;
    let key = format!("{kind:?}{b:?}{:?}{}{pw}", app.primary, pal.dark);
    if app
        .paint_ui
        .preview
        .as_ref()
        .map(|(k, _)| k != &key)
        .unwrap_or(true)
    {
        let px = stroke_preview(kind, &b, app.primary, pal.dark, pw, ph);
        let img = egui::ColorImage::from_rgba_unmultiplied([pw as usize, ph as usize], &px);
        let tex = ui
            .ctx()
            .load_texture("paint.preview", img, egui::TextureOptions::LINEAR);
        app.paint_ui.preview = Some((key, tex));
    }
    if let Some((_, tex)) = &app.paint_ui.preview {
        let (r, _) = ui.allocate_exact_size(Vec2::new(pw as f32, ph as f32), Sense::hover());
        ui.painter().image(
            tex.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        ui.painter().rect_stroke(
            r,
            8.0,
            Stroke::new(1.0, with_alpha(pal.neon, 0.5)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            r.right_bottom() - Vec2::new(6.0, 4.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("{:.0} px", b.size),
            egui::FontId::proportional(11.0),
            Color32::WHITE,
        );
    }
    ui.add_space(2.0);
    ui.label(
        RichText::new(tr("style.title"))
            .size(12.0)
            .color(pal.text_weak),
    );
    let mut apply: Option<BrushSettings> = None;
    let mut delete: Option<usize> = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for st in builtin_styles(kind) {
            let on = same_style(&b, &st.settings);
            if chip(ui, &pal, &style_name(st.id), on)
                .on_hover_text(tr(&format!("style.{}.desc", st.id)))
                .clicked()
            {
                apply = Some(keep_size(&b, &st.settings));
            }
        }
        for (i, u) in app.s.user_styles.iter().enumerate() {
            if u.tool != kind.id() {
                continue;
            }
            let on = same_style(&b, &u.settings);
            let r = chip(ui, &pal, &format!("• {}", u.name), on);
            if r.clicked() {
                apply = Some(keep_size(&b, &u.settings));
            }
            r.context_menu(|ui| {
                if ui.button(tr("style.delete")).clicked() {
                    delete = Some(i);
                    ui.close();
                }
            });
        }
    });
    if let Some(i) = delete {
        app.s.user_styles.remove(i);
        app.s.save();
    }
    if let Some(s) = apply {
        if let Some(bm) = app.brushes.get_mut(&kind) {
            *bm = s;
        }
    }
    // save the current settings as a style
    match app.paint_ui.style_name.take() {
        None => {
            if ui
                .small_button(format!("+ {}", tr("style.save")))
                .on_hover_text(tr("style.save_tip"))
                .clicked()
            {
                app.paint_ui.style_name = Some(String::new());
            }
        }
        Some(mut name) => {
            let mut keep = true;
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .hint_text(tr("style.name_hint"))
                        .desired_width(ui.available_width() - 70.0)
                        .char_limit(40),
                );
                r.request_focus();
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.small_button("✓").clicked() || enter) && !name.trim().is_empty() {
                    let settings = app.brushes.get(&kind).copied().unwrap_or(b);
                    let nm = name.trim().to_string();
                    app.s
                        .user_styles
                        .retain(|u| !(u.tool == kind.id() && u.name == nm));
                    app.s.user_styles.push(UserStyle {
                        tool: kind.id().to_string(),
                        name: nm,
                        settings,
                    });
                    app.s.save();
                    keep = false;
                }
                if ui.small_button("✕").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    keep = false;
                }
            });
            if keep {
                app.paint_ui.style_name = Some(name);
            }
        }
    }
}

/// Picking a style keeps the current brush size.
fn keep_size(cur: &BrushSettings, style: &BrushSettings) -> BrushSettings {
    BrushSettings {
        size: cur.size,
        ..*style
    }
}

/// Whether `cur` is `style` (any size).
fn same_style(cur: &BrushSettings, style: &BrushSettings) -> bool {
    keep_size(cur, style) == *cur
}

fn chip(ui: &mut egui::Ui, pal: &Palette, text: &str, on: bool) -> egui::Response {
    let font = egui::FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        font,
        if on { Color32::WHITE } else { pal.text },
    );
    let size = galley.size() + Vec2::new(16.0, 8.0);
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    let hover = ui.ctx().animate_bool(resp.id, resp.hovered());
    let p = ui.painter();
    if on {
        theme::gradient_pill(
            p,
            r,
            &[crate::theme::brand::ROYAL, crate::theme::brand::AZURE],
            9.0,
        );
    } else {
        p.rect_filled(r, 9.0, theme::lerp_color(pal.raised, pal.panel_hi, hover));
        p.rect_stroke(
            r,
            9.0,
            Stroke::new(1.0, theme::lerp_color(pal.stroke, pal.neon, hover * 0.7)),
            egui::StrokeKind::Inside,
        );
    }
    p.galley(r.center() - galley.size() * 0.5, galley, Color32::WHITE);
    resp
}

/// Grid of tip shapes with thumbnails, angle and "follow the stroke".
pub fn tip_shapes(app: &mut PanoApp, ui: &mut egui::Ui, kind: ToolKind) {
    let pal = app.pal;
    let dark = pal.dark;
    for shape in TipShape::ALL {
        let ctx = ui.ctx().clone();
        app.paint_ui.tips.entry((shape, dark)).or_insert_with(|| {
            let col = if dark {
                [0xe6, 0xf0, 0xff]
            } else {
                [0x0b, 0x14, 0x30]
            };
            let img = shape.image(96, 0.7).to_rgba_preview(40, col);
            let ci = egui::ColorImage::from_rgba_unmultiplied([40, 40], &img);
            ctx.load_texture(
                format!("tip.{}.{dark}", shape.id()),
                ci,
                egui::TextureOptions::LINEAR,
            )
        });
    }
    let Some(b) = app.brushes.get_mut(&kind) else {
        return;
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for shape in TipShape::ALL {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
            let on = b.tip_shape == shape;
            let hover = ui.ctx().animate_bool(resp.id, resp.hovered());
            let p = ui.painter();
            p.rect_filled(
                r,
                7.0,
                if on {
                    with_alpha(pal.accent, 0.25)
                } else {
                    theme::lerp_color(pal.raised, pal.panel_hi, hover)
                },
            );
            p.rect_stroke(
                r,
                7.0,
                if on {
                    Stroke::new(2.0, pal.neon)
                } else {
                    Stroke::new(1.0, with_alpha(pal.stroke, 0.9))
                },
                egui::StrokeKind::Inside,
            );
            if let Some(t) = app.paint_ui.tips.get(&(shape, dark)) {
                p.image(
                    t.id(),
                    r.shrink(3.0),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            let resp = resp.on_hover_text(tr(&format!("tip.{}", shape.id())));
            if resp.clicked() && !on {
                b.tip_shape = shape;
                b.follow_stroke = shape.follows_stroke_by_default();
            }
        }
    });
    if b.tip_shape != TipShape::Round {
        slider_row(
            ui,
            tr("tip.angle"),
            &mut b.tip_angle,
            -180.0..=180.0,
            "°",
            false,
        );
        toggle(ui, &pal, &mut b.follow_stroke, tr("tip.follow"));
    }
}

/// Paint mixing (blending, dilution, persistence…).
pub fn mixing(ui: &mut egui::Ui, pal: &Palette, b: &mut BrushSettings, pen_pressure: bool) {
    ui.label(
        RichText::new(tr("mix.hint"))
            .size(11.5)
            .color(pal.text_weak),
    );
    pct(ui, tr("mix.blending"), &mut b.blending, 0.0, 100.0).on_hover_text(tr("mix.blending_tip"));
    pct(ui, tr("mix.dilution"), &mut b.dilution, 0.0, 100.0).on_hover_text(tr("mix.dilution_tip"));
    pct(ui, tr("mix.persistence"), &mut b.persistence, 0.0, 100.0)
        .on_hover_text(tr("mix.persistence_tip"));
    pct(ui, tr("mix.irregularity"), &mut b.irregularity, 0.0, 100.0)
        .on_hover_text(tr("mix.irregularity_tip"));
    toggle(ui, pal, &mut b.pigment, tr("mix.pigment")).on_hover_text(tr("mix.pigment_tip"));
    ui.add_enabled_ui(pen_pressure, |ui| {
        toggle(ui, pal, &mut b.pressure_blend, tr("mix.pressure_blend"));
    });
}

/// Spray particles of the airbrush.
pub fn spray(ui: &mut egui::Ui, pal: &Palette, b: &mut BrushSettings) {
    ui.label(
        RichText::new(tr("spray.hint"))
            .size(11.5)
            .color(pal.text_weak),
    );
    pct(ui, tr("spray.amount"), &mut b.particles, 0.0, 100.0);
    ui.add_enabled_ui(b.particles > 0.0, |ui| {
        pct(ui, tr("spray.density"), &mut b.particle_density, 0.0, 100.0);
        pct(ui, tr("spray.size"), &mut b.particle_size, 0.5, 50.0);
        pct(
            ui,
            tr("spray.jitter"),
            &mut b.particle_size_jitter,
            0.0,
            100.0,
        );
        pct(ui, tr("spray.spread"), &mut b.spread, 20.0, 300.0);
        pct(ui, tr("spray.falloff"), &mut b.spread_falloff, 0.0, 100.0);
    });
}

/// Paper / canvas texture.
pub fn texture(ui: &mut egui::Ui, pal: &Palette, b: &mut BrushSettings) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for t in PaperTexture::ALL {
            if chip(ui, pal, tr(&format!("tex.{}", t.id())), b.texture == t).clicked() {
                b.texture = t;
            }
        }
    });
    if b.texture != PaperTexture::None {
        pct(ui, tr("tex.strength"), &mut b.texture_strength, 0.0, 100.0);
        slider_row(
            ui,
            tr("tex.scale"),
            &mut b.texture_scale,
            0.25..=8.0,
            "×",
            true,
        );
    }
}

/// Extra pressure options (curve and minimum density).
pub fn pressure_extra(ui: &mut egui::Ui, b: &mut BrushSettings) {
    let mut f = b.pressure_feel * 100.0;
    if slider_row(
        ui,
        tr("brush.pressure_feel"),
        &mut f,
        -100.0..=100.0,
        "",
        false,
    )
    .on_hover_text(tr("brush.pressure_feel_tip"))
    .changed()
    {
        b.pressure_feel = f / 100.0;
    }
    if b.pressure_opacity {
        pct(ui, tr("brush.min_density"), &mut b.min_density, 0.0, 100.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_are_deterministic_and_show_mixing() {
        let oil = BrushSettings::preset(ToolKind::Brush);
        let a = stroke_preview(ToolKind::Brush, &oil, [255, 216, 26], true, 240, 56);
        let b = stroke_preview(ToolKind::Brush, &oil, [255, 216, 26], true, 240, 56);
        assert_eq!(a, b);
        let classic = BrushSettings::classic(ToolKind::Brush);
        let c = stroke_preview(ToolKind::Brush, &classic, [255, 216, 26], true, 240, 56);
        assert_ne!(a, c, "oil and classic must look different");
        // somewhere over the blue band the oil stroke is neither blue nor yellow
        let mixed = (0..56u32).any(|y| {
            (60..90u32).any(|x| {
                let i = ((y * 240 + x) * 4) as usize;
                let p = &a[i..i + 3];
                let dy = (p[0] as i32 - 255).abs()
                    + (p[1] as i32 - 216).abs()
                    + (p[2] as i32 - 26).abs();
                let db = (p[0] as i32 - 0x1b).abs()
                    + (p[1] as i32 - 0x4f).abs()
                    + (p[2] as i32 - 0xd8).abs();
                dy > 60 && db > 60
            })
        });
        assert!(mixed);
    }
}
