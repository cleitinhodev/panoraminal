//! On-canvas handles and the floating panel of an image being placed.

use crate::app::PanoApp;
use crate::icons::Glyph;
use crate::placing::{DragState, Handle};
use crate::theme::{brand, with_alpha};
use crate::ui::widgets::{icon_button, primary_button, slider_row, toggle};
use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Stroke, Vec2};
use panoraminal_core::place::Projection;
use panoraminal_core::Vec3;
use panoraminal_i18n::tr;

const HIT: f32 = 10.0;

/// Screen positions of the handles: 4 corners, 4 edge midpoints, rotation knob, centre.
pub struct Handles {
    pub corners: [Option<Pos2>; 4],
    pub edges: [Option<Pos2>; 4],
    pub rotate: Option<Pos2>,
    pub center: Option<Pos2>,
}

fn handles(app: &PanoApp) -> Option<Handles> {
    let f = app.floating.as_ref()?;
    let p = f.p;
    let c0 = app.canvas_rect.center();
    let center = app.dir_to_screen(p.center, c0);
    let near = center.unwrap_or(c0);
    let at = |x: f64, y: f64| app.dir_to_screen(p.dir_at(x, y), near);
    let corners = [at(-1.0, 1.0), at(1.0, 1.0), at(1.0, -1.0), at(-1.0, -1.0)];
    let edges = [at(0.0, 1.0), at(1.0, 0.0), at(0.0, -1.0), at(-1.0, 0.0)];
    let rotate = match (edges[0], center) {
        (Some(t), Some(c)) => {
            let d = t - c;
            let dir = if d.length() > 1.0 {
                d.normalized()
            } else {
                Vec2::new(0.0, -1.0)
            };
            Some(t + dir * 28.0)
        }
        _ => None,
    };
    Some(Handles {
        corners,
        edges,
        rotate,
        center,
    })
}

/// The handle under `pos`, if any (Move when inside the image).
pub fn hit_test(app: &PanoApp, pos: Pos2) -> Option<Handle> {
    let h = handles(app)?;
    let close = |q: Option<Pos2>| q.map(|q| q.distance(pos) <= HIT).unwrap_or(false);
    if close(h.rotate) {
        return Some(Handle::Rotate);
    }
    for (i, c) in h.corners.iter().enumerate() {
        if close(*c) {
            return Some(Handle::Corner(i));
        }
    }
    for (i, e) in h.edges.iter().enumerate() {
        if close(*e) {
            return Some(Handle::Edge(i));
        }
    }
    let d = app.screen_to_dir(pos)?;
    let (x, y) = app.floating.as_ref()?.p.local(d)?;
    (x.abs() <= 1.0 && y.abs() <= 1.0).then_some(Handle::Move)
}

pub fn cursor_for(h: Option<Handle>, dragging: bool) -> CursorIcon {
    match h {
        Some(Handle::Move) if dragging => CursorIcon::Grabbing,
        Some(Handle::Move) => CursorIcon::Move,
        Some(Handle::Corner(i)) => {
            if i % 2 == 0 {
                CursorIcon::ResizeNwSe
            } else {
                CursorIcon::ResizeNeSw
            }
        }
        Some(Handle::Edge(i)) => {
            if i % 2 == 0 {
                CursorIcon::ResizeVertical
            } else {
                CursorIcon::ResizeHorizontal
            }
        }
        Some(Handle::Rotate) => CursorIcon::Alias,
        None => CursorIcon::Default,
    }
}

/// Starts dragging handle `h` with the pointer at `d`.
pub fn begin_drag(app: &mut PanoApp, h: Handle, d: Vec3) {
    let Some(f) = app.floating.as_mut() else {
        return;
    };
    f.checkpoint();
    let local0 = match h {
        Handle::Corner(i) => [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)][i],
        Handle::Edge(i) => [(0.0, 1.0), (1.0, 0.0), (0.0, -1.0), (-1.0, 0.0)][i],
        _ => f.p.local(d).unwrap_or((0.0, 0.0)),
    };
    f.drag = Some(DragState {
        handle: h,
        start: f.p,
        from: d,
        local0,
    });
}

/// Outline and handles over the canvas.
pub fn draw(app: &PanoApp, painter: &egui::Painter, hover: Option<Handle>) {
    let Some(f) = app.floating.as_ref() else {
        return;
    };
    let pal = app.pal;
    let p = f.p;
    let rect = app.canvas_rect;
    let near0 = app
        .dir_to_screen(p.center, rect.center())
        .unwrap_or(rect.center());
    // Outline: sampled edges, split where a point is behind the camera or jumps.
    let n = 32;
    let mut runs: Vec<Vec<Pos2>> = vec![Vec::new()];
    let mut last: Option<Pos2> = None;
    let path = (0..4).flat_map(|e| {
        (0..n).map(move |i| {
            let t = i as f64 / n as f64 * 2.0 - 1.0;
            match e {
                0 => (t, 1.0),
                1 => (1.0, -t),
                2 => (-t, -1.0),
                _ => (-1.0, t),
            }
        })
    });
    let mut pts: Vec<(f64, f64)> = path.collect();
    pts.push((-1.0, 1.0));
    for (x, y) in pts {
        let q = app.dir_to_screen(p.dir_at(x, y), last.unwrap_or(near0));
        match (q, last) {
            (Some(q), Some(l)) if q.distance(l) > rect.width().max(rect.height()) * 0.6 => {
                runs.push(vec![q]);
            }
            (Some(q), _) => runs.last_mut().unwrap().push(q),
            (None, _) => runs.push(Vec::new()),
        }
        last = q;
    }
    for r in runs.into_iter().filter(|r| r.len() > 1) {
        painter.add(egui::Shape::line(
            r.clone(),
            Stroke::new(3.0, Color32::from_black_alpha(140)),
        ));
        painter.add(egui::Shape::dashed_line(
            &r,
            Stroke::new(1.4, with_alpha(pal.neon, 0.95)),
            7.0,
            4.0,
        ));
    }
    let Some(h) = handles(app) else { return };
    let knob = |q: Pos2, hot: bool, round: bool| {
        let r = if hot { 6.5 } else { 5.0 };
        if round {
            painter.circle_filled(q, r + 1.5, Color32::from_black_alpha(150));
            painter.circle_filled(q, r, if hot { brand::CYAN } else { Color32::WHITE });
        } else {
            let rr = Rect::from_center_size(q, Vec2::splat(r * 2.0));
            painter.rect_filled(rr.expand(1.5), 2.0, Color32::from_black_alpha(150));
            painter.rect_filled(rr, 2.0, if hot { brand::CYAN } else { Color32::WHITE });
        }
    };
    if let (Some(t), Some(r)) = (h.edges[0], h.rotate) {
        painter.line_segment([t, r], Stroke::new(1.2, with_alpha(pal.neon, 0.9)));
    }
    for (i, c) in h.corners.iter().enumerate() {
        if let Some(c) = c {
            knob(*c, hover == Some(Handle::Corner(i)), false);
        }
    }
    for (i, e) in h.edges.iter().enumerate() {
        if let Some(e) = e {
            knob(*e, hover == Some(Handle::Edge(i)), false);
        }
    }
    if let Some(r) = h.rotate {
        knob(r, hover == Some(Handle::Rotate), true);
    }
    if let Some(c) = h.center {
        painter.line_segment(
            [c - Vec2::new(6.0, 0.0), c + Vec2::new(6.0, 0.0)],
            Stroke::new(1.2, Color32::WHITE),
        );
        painter.line_segment(
            [c - Vec2::new(0.0, 6.0), c + Vec2::new(0.0, 6.0)],
            Stroke::new(1.2, Color32::WHITE),
        );
    }
}

/// The floating panel with the placement options.
pub fn panel(app: &mut PanoApp, ctx: &egui::Context) {
    if app.floating.is_none() || app.ui_hidden {
        return;
    }
    let pal = app.pal;
    let rect = app.canvas_rect;
    let mut merge = None;
    let mut cancel = false;
    let mut reaim = false;
    let mut proj_change = None;
    egui::Window::new(tr("place.title"))
        .id(egui::Id::unique("place_panel"))
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .default_pos(Pos2::new(rect.left() + 14.0, rect.bottom() - 14.0))
        .default_width(260.0)
        .constrain_to(rect)
        .show(ctx, |ui| {
            let Some(f) = app.floating.as_mut() else {
                return;
            };
            ui.set_max_width(300.0);
            ui.label(
                egui::RichText::new(format!("{} · {}×{}", f.name, f.img.width, f.img.height))
                    .size(12.0)
                    .color(pal.text_weak),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for (proj, key, tip) in [
                    (Projection::Wrap, "place.mode.wrap", "place.mode.wrap.desc"),
                    (
                        Projection::Gnomonic,
                        "place.mode.persistent",
                        "place.mode.persistent.desc",
                    ),
                ] {
                    if ui
                        .selectable_label(f.p.projection == proj, tr(key))
                        .on_hover_text(tr(tip))
                        .clicked()
                    {
                        proj_change = Some(proj);
                    }
                }
            });
            ui.add_space(4.0);
            let before = f.p;
            let mut op = f.p.opacity * 100.0;
            let r1 = slider_row(ui, tr("place.opacity"), &mut op, 0.0..=100.0, "%", false);
            f.p.opacity = op / 100.0;
            let mut deg = f.p.rotation.to_degrees();
            if deg > 180.0 {
                deg -= 360.0;
            }
            let r2 = slider_row(
                ui,
                tr("place.rotation"),
                &mut deg,
                -180.0..=180.0,
                "°",
                false,
            );
            f.p.rotation = deg.to_radians();
            let mut size = (f.p.half_w.max(f.p.half_h) * 2.0).to_degrees();
            let max = match f.p.projection {
                Projection::Wrap => 360.0,
                Projection::Gnomonic => 170.0,
            };
            let old = size;
            let r3 = slider_row(ui, tr("place.size"), &mut size, 0.5..=max, "°", true);
            if (size - old).abs() > 1e-9 && old > 0.0 {
                let k = size / old;
                match f.p.projection {
                    Projection::Wrap => {
                        f.p.half_w *= k;
                        f.p.half_h *= k;
                    }
                    Projection::Gnomonic => {
                        f.p.half_w = (f.p.half_w.tan() * k).atan();
                        f.p.half_h = (f.p.half_h.tan() * k).atan();
                    }
                }
            }
            ui.horizontal(|ui| {
                let mut fx = f.p.flip_x;
                let mut fy = f.p.flip_y;
                toggle(ui, &pal, &mut fx, tr("place.flip_h"));
                toggle(ui, &pal, &mut fy, tr("place.flip_v"));
                f.p.flip_x = fx;
                f.p.flip_y = fy;
            });
            // One undo step per slider drag / click.
            let started = [&r1, &r2, &r3].iter().any(|r| r.drag_started());
            let clicked = [&r1, &r2, &r3].iter().any(|r| r.changed() && !r.dragged());
            if f.p != before {
                let now = f.p;
                if started
                    || clicked
                    || (before.flip_x != now.flip_x || before.flip_y != now.flip_y)
                {
                    f.p = before;
                    f.checkpoint();
                    f.p = now;
                }
                f.p.clamp();
            }
            if f.p.projection == Projection::Gnomonic {
                ui.add_space(2.0);
                if ui
                    .button(tr("place.reaim"))
                    .on_hover_text(tr("place.reaim.desc"))
                    .clicked()
                {
                    reaim = true;
                }
            }
            ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                if primary_button(ui, &pal, tr("place.merge"), app.s.glow.factor())
                    .on_hover_text(format!("{} (Enter)", tr("place.merge.desc")))
                    .clicked()
                {
                    merge = Some(false);
                }
                if icon_button(ui, &pal, Glyph::Plus, false, tr("place.as_layer"), 30.0).clicked() {
                    merge = Some(true);
                }
                if icon_button(
                    ui,
                    &pal,
                    Glyph::Clear,
                    false,
                    &format!("{} (Esc)", tr("place.cancel")),
                    30.0,
                )
                .clicked()
                {
                    cancel = true;
                }
            });
            ui.add(
                egui::Label::new(
                    egui::RichText::new(tr("place.hint"))
                        .size(11.5)
                        .color(pal.text_weak),
                )
                .wrap(),
            );
        });
    if let Some(p) = proj_change {
        app.set_place_projection(p);
    }
    if reaim {
        app.reaim_placement();
    }
    if let Some(nl) = merge {
        app.commit_placement(ctx, nl);
    } else if cancel {
        app.cancel_placement();
    }
}
