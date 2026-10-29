//! Colour picker card and brush settings card.

use super::widgets::{slider_row, toggle};
use crate::app::{hex, parse_hex, PanoApp, Tool};
use crate::theme::{self, with_alpha};
use eframe::egui::ecolor::Hsva;
use eframe::egui::{self, Color32, Mesh, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use panoraminal_i18n::tr;

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = Color32::from(Hsva::new(h, s, v, 1.0));
    [c.r(), c.g(), c.b()]
}

fn rgb_to_hsv(c: [u8; 3]) -> [f32; 3] {
    let h = Hsva::from(Color32::from_rgb(c[0], c[1], c[2]));
    [h.h, h.s, h.v]
}

pub fn color_card(app: &mut PanoApp, ui: &mut egui::Ui) {
    let pal = app.pal;
    let id = ui.scope_id().with("hsv");
    let mut hsv: [f32; 3] = ui
        .data(|d| d.get_temp(id))
        .unwrap_or_else(|| rgb_to_hsv(app.primary));
    if hsv_to_rgb(hsv[0], hsv[1], hsv[2]) != app.primary {
        let n = rgb_to_hsv(app.primary);
        // Keep hue when the colour is grey or black.
        hsv = [
            if n[1] < 1e-3 || n[2] < 1e-3 {
                hsv[0]
            } else {
                n[0]
            },
            if n[2] < 1e-3 { hsv[1] } else { n[1] },
            n[2],
        ];
    }
    let w = ui.available_width();
    // Saturation / value square.
    let (sq, resp) = ui.allocate_exact_size(Vec2::new(w, 132.0), Sense::click_and_drag());
    let n = 14;
    let mut mesh = Mesh::default();
    for j in 0..=n {
        for i in 0..=n {
            let s = i as f32 / n as f32;
            let v = 1.0 - j as f32 / n as f32;
            let c = hsv_to_rgb(hsv[0], s, v);
            mesh.colored_vertex(
                Pos2::new(
                    sq.left() + sq.width() * s,
                    sq.top() + sq.height() * (1.0 - v),
                ),
                rgb(c),
            );
        }
    }
    for j in 0..n {
        for i in 0..n {
            let a = (j * (n + 1) + i) as u32;
            let b = a + 1;
            let c = a + (n + 1) as u32;
            let d = c + 1;
            mesh.add_triangle(a, b, c);
            mesh.add_triangle(b, d, c);
        }
    }
    ui.painter().add(Shape::mesh(mesh));
    ui.painter().rect_stroke(
        sq,
        6.0,
        Stroke::new(1.0, pal.stroke),
        egui::StrokeKind::Outside,
    );
    let mut changed = false;
    if let Some(p) = resp.interact_pointer_pos() {
        if resp.is_pointer_button_down_on() {
            hsv[1] = ((p.x - sq.left()) / sq.width()).clamp(0.0, 1.0);
            hsv[2] = 1.0 - ((p.y - sq.top()) / sq.height()).clamp(0.0, 1.0);
            changed = true;
        }
    }
    let knob = Pos2::new(
        sq.left() + sq.width() * hsv[1],
        sq.top() + sq.height() * (1.0 - hsv[2]),
    );
    ui.painter()
        .circle_stroke(knob, 7.0, Stroke::new(3.0, Color32::from_black_alpha(140)));
    ui.painter()
        .circle_stroke(knob, 7.0, Stroke::new(1.6, Color32::WHITE));
    ui.add_space(6.0);
    // Hue strip.
    let (hr, hresp) = ui.allocate_exact_size(Vec2::new(w, 14.0), Sense::click_and_drag());
    let stops: Vec<Color32> = (0..=12)
        .map(|i| rgb(hsv_to_rgb(i as f32 / 12.0, 1.0, 1.0)))
        .collect();
    theme::gradient_pill(ui.painter(), hr, &stops, 7.0);
    if let Some(p) = hresp.interact_pointer_pos() {
        if hresp.is_pointer_button_down_on() {
            hsv[0] = ((p.x - hr.left()) / hr.width()).clamp(0.0, 0.9999);
            changed = true;
        }
    }
    let hx = hr.left() + hr.width() * hsv[0];
    ui.painter().circle_filled(
        Pos2::new(hx, hr.center().y),
        8.0,
        rgb(hsv_to_rgb(hsv[0], 1.0, 1.0)),
    );
    ui.painter().circle_stroke(
        Pos2::new(hx, hr.center().y),
        8.0,
        Stroke::new(2.0, Color32::WHITE),
    );
    if changed {
        app.set_primary(hsv_to_rgb(hsv[0], hsv[1], hsv[2]));
    }
    if resp.drag_stopped() || hresp.drag_stopped() || resp.clicked() || hresp.clicked() {
        app.commit_color();
    }
    ui.data_mut(|d| d.insert_temp(id, hsv));
    ui.add_space(6.0);

    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::new(30.0, 22.0), Sense::hover());
        theme::glow_rect(
            ui.painter(),
            r,
            6.0,
            rgb(app.primary),
            0.4 * app.s.glow.factor(),
        );
        ui.painter().rect_filled(r, 6.0, rgb(app.primary));
        let te = ui.add(
            egui::TextEdit::singleline(&mut app.hex_edit)
                .desired_width(78.0)
                .font(egui::TextStyle::Monospace),
        );
        if te.lost_focus() || te.changed() {
            if let Some(c) = parse_hex(&app.hex_edit) {
                if c != app.primary {
                    app.primary = c;
                    if te.lost_focus() {
                        app.commit_color();
                    }
                }
            }
        }
        if te.lost_focus() {
            app.hex_edit = hex(app.primary);
        }
        let mut c = egui::Color32::from_rgb(app.primary[0], app.primary[1], app.primary[2]);
        if egui::color_picker::color_edit_button_srgba(
            ui,
            &mut c,
            egui::color_picker::Alpha::Opaque,
        )
        .changed()
        {
            app.set_primary([c.r(), c.g(), c.b()]);
        }
    });
    ui.add_space(6.0);
    // Swatches: fixed palette + recent colours.
    let fixed: [[u8; 3]; 16] = [
        [0x00, 0x00, 0x00],
        [0xff, 0xff, 0xff],
        [0x8f, 0xb0, 0xd8],
        [0x4a, 0x63, 0x90],
        [0x01, 0x30, 0xa0],
        [0x06, 0x60, 0xe9],
        [0x1b, 0x99, 0xf9],
        [0x5f, 0xd0, 0xfd],
        [0xa1, 0xdb, 0xfb],
        [0x8b, 0x5c, 0xf6],
        [0xe8, 0x4a, 0xc0],
        [0xff, 0x4d, 0x6d],
        [0xff, 0x9f, 0x43],
        [0xff, 0xd6, 0x4d],
        [0x3d, 0xdc, 0x97],
        [0x1f, 0x7a, 0x4d],
    ];
    swatch_row(app, ui, &fixed, tr("color.palette"));
    let recent = app.s.swatches.clone();
    if !recent.is_empty() {
        swatch_row(app, ui, &recent, tr("color.recent"));
    }
    let _ = pal;
}

fn swatch_row(app: &mut PanoApp, ui: &mut egui::Ui, colors: &[[u8; 3]], tip: &str) {
    let pal = app.pal;
    let w = ui.available_width();
    let cell = 18.0f32;
    let gap = 4.0;
    let per_row = (((w + gap) / (cell + gap)).floor() as usize).max(1);
    for chunk in colors.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for c in chunk {
                let (r, resp) = ui.allocate_exact_size(Vec2::splat(cell), Sense::click());
                let hover = ui.ctx().animate_bool(resp.id, resp.hovered());
                let rr = r.expand(hover * 1.5);
                ui.painter().rect_filled(rr, 5.0, rgb(*c));
                let sel = *c == app.primary;
                let st = if sel {
                    Stroke::new(2.0, pal.neon)
                } else {
                    Stroke::new(1.0, with_alpha(pal.stroke, 0.9))
                };
                ui.painter()
                    .rect_stroke(rr, 5.0, st, egui::StrokeKind::Outside);
                let resp = resp.on_hover_text(format!("{tip}: {}", hex(*c)));
                if resp.clicked() {
                    app.set_primary(*c);
                }
                if resp.secondary_clicked() {
                    app.secondary = *c;
                }
            }
        });
    }
}

pub fn brush_card(app: &mut PanoApp, ui: &mut egui::Ui) {
    let pal = app.pal;
    match app.tool {
        Tool::Paint(kind) => {
            let pen_pressure = app.s.pen_pressure;
            let Some(b) = app.brush_mut() else { return };
            // Preview of the brush tip.
            let (r, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::hover());
            brush_preview(ui, r, b.size, b.hardness, b.opacity, kind.is_eraser(), &pal);
            slider_row(ui, tr("brush.size"), &mut b.size, 1.0..=1000.0, " px", true);
            let mut op = b.opacity * 100.0;
            if slider_row(ui, tr("brush.opacity"), &mut op, 1.0..=100.0, "%", false).changed() {
                b.opacity = op / 100.0;
            }
            let mut fl = b.flow * 100.0;
            if slider_row(ui, tr("brush.flow"), &mut fl, 1.0..=100.0, "%", false).changed() {
                b.flow = fl / 100.0;
            }
            let mut hd = b.hardness * 100.0;
            if slider_row(ui, tr("brush.hardness"), &mut hd, 0.0..=100.0, "%", false).changed() {
                b.hardness = hd / 100.0;
            }
            egui::CollapsingHeader::new(tr("brush.advanced"))
                .id_salt("brush_adv")
                .show(ui, |ui| {
                    let mut sp = b.spacing * 100.0;
                    if slider_row(ui, tr("brush.spacing"), &mut sp, 2.0..=100.0, "%", false)
                        .changed()
                    {
                        b.spacing = sp / 100.0;
                    }
                    let mut sm = b.smoothing * 100.0;
                    if slider_row(ui, tr("brush.smoothing"), &mut sm, 0.0..=95.0, "%", false)
                        .changed()
                    {
                        b.smoothing = sm / 100.0;
                    }
                    ui.add_enabled_ui(pen_pressure, |ui| {
                        toggle(ui, &pal, &mut b.pressure_size, tr("brush.pressure_size"));
                        toggle(
                            ui,
                            &pal,
                            &mut b.pressure_opacity,
                            tr("brush.pressure_opacity"),
                        );
                        let mut ms = b.min_size * 100.0;
                        if slider_row(ui, tr("brush.min_size"), &mut ms, 0.0..=100.0, "%", false)
                            .changed()
                        {
                            b.min_size = ms / 100.0;
                        }
                    });
                    toggle(ui, &pal, &mut b.polar_correction, tr("brush.polar"))
                        .on_hover_text(tr("brush.polar_tip"));
                    if ui.small_button(tr("brush.reset")).clicked() {
                        *b = panoraminal_core::BrushSettings::preset(kind);
                    }
                });
            b.clamp();
        }
        Tool::Fill => {
            let f = &mut app.s.fill;
            let mut t = f.tolerance * 100.0;
            if slider_row(ui, tr("fill.tolerance"), &mut t, 0.0..=100.0, "%", false).changed() {
                f.tolerance = t / 100.0;
            }
            let mut o = f.opacity * 100.0;
            if slider_row(ui, tr("brush.opacity"), &mut o, 1.0..=100.0, "%", false).changed() {
                f.opacity = o / 100.0;
            }
            slider_row(ui, tr("fill.expand"), &mut f.expand, 0..=8, " px", false);
            toggle(ui, &pal, &mut f.sample_merged, tr("fill.sample_merged"));
            ui.label(
                egui::RichText::new(tr("fill.hint"))
                    .size(12.0)
                    .color(pal.text_weak),
            );
        }
        Tool::Eyedropper => {
            ui.label(
                egui::RichText::new(tr("eyedropper.hint"))
                    .size(12.5)
                    .color(pal.text_weak),
            );
        }
    }
}

fn brush_preview(
    ui: &egui::Ui,
    r: Rect,
    size: f32,
    hardness: f32,
    opacity: f32,
    eraser: bool,
    pal: &crate::theme::Palette,
) {
    let p = ui.painter();
    p.rect_filled(
        r,
        8.0,
        if pal.dark {
            Color32::from_rgb(0x05, 0x0b, 0x22)
        } else {
            Color32::from_rgb(0xee, 0xf5, 0xfd)
        },
    );
    let rad = (size * 0.5).clamp(1.0, r.height() * 0.45);
    let c = r.center();
    let col = if eraser { pal.text_weak } else { pal.accent };
    let steps = 12;
    for i in (0..steps).rev() {
        let k = (i + 1) as f32 / steps as f32;
        let edge = hardness.clamp(0.0, 0.99);
        let a = if k <= edge {
            1.0
        } else {
            1.0 - (k - edge) / (1.0 - edge)
        };
        p.circle_filled(
            c,
            rad * k,
            with_alpha(col, (a * opacity / steps as f32 * 3.0).min(1.0)),
        );
    }
    p.circle_stroke(c, rad, Stroke::new(1.0, with_alpha(pal.neon, 0.7)));
    p.text(
        r.right_center() - Vec2::new(8.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        format!("{:.0} px", size),
        egui::FontId::proportional(12.0),
        pal.text_weak,
    );
}
