//! Line layer UI: on-canvas handles and the line tools panel.

use super::widgets::{slider_row, toggle};
use crate::app::PanoApp;
use crate::linetools::{screen_polylines, Hit, LineTool, Op, GRAB};
use crate::theme::with_alpha;
use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Stroke, Vec2};
use panoraminal_core::lines::{self, LinePoint, LineStroke};
use panoraminal_core::Vec3;
use panoraminal_i18n::{tr, trf};

pub fn cursor_for(t: LineTool, hover: Option<Hit>, dragging: bool) -> CursorIcon {
    match t {
        LineTool::Curve | LineTool::Freehand => CursorIcon::Crosshair,
        LineTool::Edit => match (hover, dragging) {
            (Some(Hit::Point { .. }), _) => CursorIcon::Move,
            (Some(Hit::Segment { .. }), true) => CursorIcon::Grabbing,
            (Some(Hit::Segment { .. }), false) => CursorIcon::Grab,
            (None, _) => CursorIcon::Default,
        },
        LineTool::Thickness => match hover {
            Some(_) => CursorIcon::ResizeNeSw,
            None => CursorIcon::Default,
        },
    }
}

fn c32(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Draws the strokes' centre lines and control points of the active line layer.
pub fn draw_overlay(
    app: &PanoApp,
    painter: &egui::Painter,
    ptr: Option<Pos2>,
    mods: egui::Modifiers,
) {
    let Some(tool) = app.is_line_tool() else {
        return;
    };
    let pal = app.pal;
    let painter = painter.with_clip_rect(app.canvas_rect);
    let px_angle = app.angle_per_point();
    // Size ring for the drawing tools.
    if let (Some(p), true) = (ptr, tool.draws() && app.drag != crate::app::Drag::Rotate) {
        let r = (app.s.lines.width as f64 * 0.5 * std::f64::consts::TAU
            / app.doc.width as f64
            / px_angle) as f32;
        if r > 2.5 {
            painter.circle_stroke(p, r, Stroke::new(2.5, Color32::from_black_alpha(110)));
            painter.circle_stroke(p, r, Stroke::new(1.1, with_alpha(pal.neon, 0.9)));
        }
        painter.circle_filled(p, 1.6, Color32::WHITE);
    }
    let Some(art) = app.active_line_art() else {
        return;
    };
    let step = app.line_screen_step();
    let proj = |d: Vec3, near: Pos2| app.dir_to_screen(d, near);
    let near0 = ptr.unwrap_or(app.canvas_rect.center());
    let sel = app.lines.sel.filter(|s| s.layer == app.doc.active);
    let hover = app.lines.hover;
    let draft = match &app.lines.active {
        Some(a) if a.layer == app.doc.active => match a.op {
            Op::Curve { stroke, dragging } => Some((stroke, dragging)),
            _ => None,
        },
        _ => None,
    };
    let editing = matches!(tool, LineTool::Edit | LineTool::Thickness);
    let all_points = art.point_count() <= 4000;
    let shadow = Color32::from_black_alpha(if pal.dark { 120 } else { 90 });
    for (si, s) in art.strokes.iter().enumerate() {
        let is_sel = sel.is_some_and(|x| x.stroke == si);
        let is_hover = hover.is_some_and(|h| h.stroke() == si);
        let is_draft = draft.is_some_and(|d| d.0 == si);
        if !(editing || is_sel || is_hover || is_draft) {
            continue;
        }
        let (col, w) = if is_sel || is_draft {
            (pal.neon, 1.6)
        } else if is_hover {
            (pal.accent, 1.5)
        } else {
            (with_alpha(Color32::WHITE, 0.55), 1.0)
        };
        for pl in screen_polylines(s, step, near0, &proj) {
            if pl.len() < 2 {
                continue;
            }
            painter.add(egui::Shape::line(pl.clone(), Stroke::new(w + 2.0, shadow)));
            painter.add(egui::Shape::line(pl, Stroke::new(w, col)));
        }
        let show_points = is_draft || (editing && (all_points || is_sel || is_hover));
        if !show_points {
            continue;
        }
        let n = s.points.len();
        let half = (s.base_radius(app.doc.width) / px_angle) as f32;
        for (pi, p) in s.points.iter().enumerate() {
            let Some(sp) = proj(p.dir(), near0) else {
                continue;
            };
            if !app.canvas_rect.expand(8.0).contains(sp) {
                continue;
            }
            let sel_pt = is_sel && sel.and_then(|x| x.point) == Some(pi);
            let hov_pt = hover
                == Some(Hit::Point {
                    stroke: si,
                    point: pi,
                });
            if tool == LineTool::Thickness && (is_sel || hov_pt) {
                let r = half * p.w;
                if r > 3.0 {
                    painter.circle_stroke(sp, r, Stroke::new(1.0, with_alpha(pal.neon, 0.55)));
                }
            }
            let end = !s.closed && (pi == 0 || pi + 1 == n);
            let r = if sel_pt || hov_pt {
                5.0
            } else if end {
                4.2
            } else {
                3.6
            };
            let fill = if sel_pt {
                pal.neon
            } else if hov_pt {
                pal.accent
            } else if is_sel || is_draft {
                Color32::WHITE
            } else {
                Color32::from_gray(225)
            };
            let outline = Stroke::new(1.3, Color32::from_rgb(10, 18, 40));
            if p.sharp {
                let rect = Rect::from_center_size(sp, Vec2::splat(r * 1.9));
                painter.rect_filled(rect, 1.0, fill);
                painter.rect_stroke(rect, 1.0, outline, egui::StrokeKind::Outside);
            } else {
                painter.circle_filled(sp, r, fill);
                painter.circle_stroke(sp, r, outline);
            }
            if end {
                painter.circle_stroke(sp, r + 2.6, Stroke::new(1.0, with_alpha(fill, 0.8)));
            }
        }
    }
    // Curve tool: preview of the next segment and the "close" target.
    if let (Some((si, false)), Some(p), Some(cd)) = (draft, ptr, app.cursor_dir) {
        if let Some(s) = art.strokes.get(si) {
            let n = s.points.len();
            let close =
                n >= 3 && proj(s.points[0].dir(), p).is_some_and(|q| (q - p).length() <= GRAB);
            if close {
                if let Some(q) = proj(s.points[0].dir(), p) {
                    painter.circle_stroke(q, 9.0, Stroke::new(2.0, pal.neon));
                }
            }
            let mut tmp = LineStroke {
                points: s.points[n.saturating_sub(2)..].to_vec(),
                ..s.clone()
            };
            tmp.closed = false;
            let mut np = LinePoint::new(cd, 1.0);
            np.sharp = mods.shift;
            tmp.points.push(np);
            let seg = tmp.segment_count().saturating_sub(1);
            if tmp.segment_count() > 0 {
                let mut pts = Vec::new();
                let mut last = p;
                for j in 0..=48 {
                    if let Some(q) = proj(tmp.eval(seg, j as f64 / 48.0), last) {
                        pts.push(q);
                        last = q;
                    }
                }
                if pts.len() > 1 {
                    painter.add(egui::Shape::dashed_line(
                        &pts,
                        Stroke::new(1.5, with_alpha(pal.neon, 0.9)),
                        6.0,
                        4.0,
                    ));
                }
            }
        }
    }
}

fn hint(ui: &mut egui::Ui, pal: &crate::theme::Palette, text: &str) {
    ui.label(egui::RichText::new(text).size(12.0).color(pal.text_weak));
}

fn pct_row(ui: &mut egui::Ui, label: &str, v: &mut f32, lo: f32, hi: f32) -> bool {
    let mut x = *v * 100.0;
    let r = slider_row(ui, label, &mut x, lo..=hi, "%", false);
    if r.changed() {
        *v = x / 100.0;
        true
    } else {
        false
    }
}

/// The line tools panel (shown in place of the brush settings).
pub fn panel(app: &mut PanoApp, ui: &mut egui::Ui, tool: LineTool) {
    let pal = app.pal;
    let ctx = ui.ctx().clone();
    hint(
        ui,
        &pal,
        tr(match tool {
            LineTool::Curve => "lines.hint.curve",
            LineTool::Freehand => "lines.hint.free",
            LineTool::Edit => "lines.hint.edit",
            LineTool::Thickness => "lines.hint.width",
        }),
    );
    ui.add_space(4.0);
    // Layer status.
    let stats = app
        .active_line_art()
        .map(|a| (a.strokes.len(), a.point_count()));
    match stats {
        Some((ns, np)) => {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(trf("lines.stats", &[("n", &ns), ("p", &np)]))
                        .size(12.0)
                        .color(pal.accent),
                );
                if ui
                    .small_button(tr("lines.rasterize"))
                    .on_hover_text(tr("lines.rasterize_tip"))
                    .clicked()
                {
                    app.rasterize_line_layer(&ctx);
                }
            });
        }
        None => {
            hint(
                ui,
                &pal,
                tr(if tool.draws() {
                    "lines.auto_layer"
                } else {
                    "lines.no_layer"
                }),
            );
            if ui.small_button(tr("menu.layer.new_lines")).clicked() {
                app.new_line_layer(&ctx);
            }
        }
    }
    ui.add_space(4.0);

    // Options for new strokes.
    egui::CollapsingHeader::new(tr("lines.new_stroke"))
        .id_salt("lines_new")
        .default_open(tool.draws())
        .show(ui, |ui| {
            let pen = app.s.pen_pressure;
            let l = &mut app.s.lines;
            slider_row(
                ui,
                tr("brush.size"),
                &mut l.width,
                lines::MIN_WIDTH..=lines::MAX_WIDTH,
                " px",
                true,
            );
            pct_row(ui, tr("brush.opacity"), &mut l.opacity, 1.0, 100.0);
            pct_row(ui, tr("lines.taper_start"), &mut l.taper_start, 0.0, 100.0);
            pct_row(ui, tr("lines.taper_end"), &mut l.taper_end, 0.0, 100.0);
            toggle(ui, &pal, &mut l.smooth, tr("lines.smooth"))
                .on_hover_text(tr("lines.smooth_tip"));
            if tool == LineTool::Freehand {
                pct_row(ui, tr("brush.smoothing"), &mut l.smoothing, 0.0, 100.0);
                ui.add_enabled_ui(pen, |ui| {
                    toggle(ui, &pal, &mut l.pressure, tr("lines.pressure"));
                });
            }
            if tool == LineTool::Edit {
                toggle(ui, &pal, &mut l.snap_join, tr("lines.snap_join"))
                    .on_hover_text(tr("lines.snap_join_tip"));
            }
            l.clamp();
        });

    // The selected stroke.
    let Some(sel) = app.lines.sel.filter(|s| s.layer == app.doc.active) else {
        if matches!(tool, LineTool::Edit | LineTool::Thickness) && stats.is_some() {
            hint(ui, &pal, tr("lines.select_hint"));
        }
        return;
    };
    let Some(orig) = app
        .active_line_art()
        .and_then(|a| a.strokes.get(sel.stroke))
        .cloned()
    else {
        return;
    };
    ui.add_space(4.0);
    egui::CollapsingHeader::new(tr("lines.selected"))
        .id_salt("lines_sel")
        .default_open(true)
        .show(ui, |ui| {
            let mut s = orig.clone();
            ui.horizontal(|ui| {
                let (r, resp) = ui.allocate_exact_size(Vec2::new(22.0, 18.0), egui::Sense::hover());
                ui.painter().rect_filled(r, 4.0, c32(s.color));
                ui.painter().rect_stroke(
                    r,
                    4.0,
                    Stroke::new(1.0, pal.stroke),
                    egui::StrokeKind::Outside,
                );
                resp.on_hover_text(crate::app::hex(s.color));
                if ui.small_button(tr("lines.apply_color")).clicked() {
                    s.color = app.primary;
                }
                if ui
                    .small_button(tr("lines.apply_settings"))
                    .on_hover_text(tr("lines.apply_settings_tip"))
                    .clicked()
                {
                    let l = app.s.lines;
                    s.width = l.width;
                    s.opacity = l.opacity;
                    s.taper_start = l.taper_start;
                    s.taper_end = l.taper_end;
                    s.color = app.primary;
                }
            });
            slider_row(
                ui,
                tr("brush.size"),
                &mut s.width,
                lines::MIN_WIDTH..=lines::MAX_WIDTH,
                " px",
                true,
            );
            pct_row(ui, tr("brush.opacity"), &mut s.opacity, 0.0, 100.0);
            pct_row(ui, tr("lines.taper_start"), &mut s.taper_start, 0.0, 100.0);
            pct_row(ui, tr("lines.taper_end"), &mut s.taper_end, 0.0, 100.0);
            toggle(ui, &pal, &mut s.smooth, tr("lines.smooth"));
            ui.add_enabled_ui(s.points.len() >= 3, |ui| {
                toggle(ui, &pal, &mut s.closed, tr("lines.closed"));
            });
            let mut delete_stroke = false;
            let mut split = None;
            let mut delete_point = None;
            if let Some(pi) = sel.point.filter(|p| *p < s.points.len()) {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(trf(
                        "lines.point",
                        &[("n", &(pi + 1)), ("of", &s.points.len())],
                    ))
                    .size(12.5)
                    .strong(),
                );
                let p = &mut s.points[pi];
                let mut w = p.w * 100.0;
                if slider_row(
                    ui,
                    tr("lines.point_width"),
                    &mut w,
                    lines::MIN_POINT_WIDTH * 100.0..=lines::MAX_POINT_WIDTH * 100.0,
                    "%",
                    false,
                )
                .changed()
                {
                    p.w = w / 100.0;
                }
                toggle(ui, &pal, &mut p.sharp, tr("lines.corner"))
                    .on_hover_text(tr("lines.corner_tip"));
                ui.horizontal_wrapped(|ui| {
                    let n = s.points.len();
                    let can_split = s.closed || (pi > 0 && pi + 1 < n);
                    if ui
                        .add_enabled(can_split, egui::Button::new(tr("lines.split")).small())
                        .on_hover_text(tr("lines.split_tip"))
                        .clicked()
                    {
                        split = Some(pi);
                    }
                    if ui.small_button(tr("lines.delete_point")).clicked() {
                        delete_point = Some(pi);
                    }
                });
            }
            ui.add_space(2.0);
            if ui.small_button(tr("lines.delete_stroke")).clicked() {
                delete_stroke = true;
            }
            s.sanitize();
            let si = sel.stroke;
            if delete_stroke {
                app.lines.sel = Some(crate::linetools::Sel { point: None, ..sel });
                app.line_delete();
            } else if delete_point.is_some() {
                app.line_delete();
            } else if let Some(pi) = split {
                app.line_panel_edit(|a| {
                    lines::split_at(a, si, pi);
                });
                app.lines.sel = Some(crate::linetools::Sel { point: None, ..sel });
            } else if s != orig {
                app.line_panel_edit(|a| {
                    if let Some(t) = a.strokes.get_mut(si) {
                        *t = s;
                    }
                });
            }
        });
}
