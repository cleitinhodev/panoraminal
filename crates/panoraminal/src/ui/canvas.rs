//! The painting viewport: GPU paint callback, pointer/pen input and brush cursor.

use crate::app::{Drag, PanoApp, ToastKind, Tool};
use crate::theme::with_alpha;
use eframe::egui::{
    self, Color32, CursorIcon, Event, Key, PointerButton, Pos2, Rect, Sense, Stroke, TouchPhase,
    Vec2,
};
use panoraminal_core::math::{dir_to_px, px_to_dir};
use panoraminal_core::Vec3;
use panoraminal_i18n::tr;
use panoraminal_render::{CanvasCallback, DisplayParams, DisplayStyle, ViewMode};

fn c3(c: Color32) -> [f32; 3] {
    [
        c.r() as f32 / 255.0,
        c.g() as f32 / 255.0,
        c.b() as f32 / 255.0,
    ]
}

impl PanoApp {
    /// Screen position (points) → direction on the sphere.
    pub fn screen_to_dir(&self, pos: Pos2) -> Option<Vec3> {
        let r = self.canvas_rect;
        if r.width() < 1.0 || r.height() < 1.0 {
            return None;
        }
        let (lx, ly) = ((pos.x - r.left()) as f64, (pos.y - r.top()) as f64);
        let (vw, vh) = (r.width() as f64, r.height() as f64);
        match self.mode {
            ViewMode::Sphere => Some(self.cam.ray_screen(lx, ly, vw, vh)),
            ViewMode::Flat => {
                let (u, v) = self.flat.screen_to_uv(lx, ly, vw, vh);
                if !(-0.02..=1.02).contains(&v) {
                    return None;
                }
                let (w, h) = (self.doc.width, self.doc.height);
                let x = u.rem_euclid(1.0) * w as f64;
                let y = v.clamp(0.0, 0.99999) * h as f64;
                Some(px_to_dir(x, y, w, h))
            }
        }
    }

    /// Direction → screen position (points); for the flat view the wrapped copy
    /// closest to `near` is chosen.
    pub fn dir_to_screen(&self, d: Vec3, near: Pos2) -> Option<Pos2> {
        let r = self.canvas_rect;
        let (vw, vh) = (r.width() as f64, r.height() as f64);
        match self.mode {
            ViewMode::Sphere => self
                .cam
                .project_screen(d, vw, vh)
                .map(|(x, y)| Pos2::new(r.left() + x as f32, r.top() + y as f32)),
            ViewMode::Flat => {
                let p = dir_to_px(d, self.doc.width, self.doc.height);
                let u = p.x / self.doc.width as f64;
                let v = p.y / self.doc.height as f64;
                let (nu, _) = self.flat.screen_to_uv(
                    (near.x - r.left()) as f64,
                    (near.y - r.top()) as f64,
                    vw,
                    vh,
                );
                let u = u + (nu - u).round();
                let (x, y) = self.flat.uv_to_screen(u, v, vw, vh);
                Some(Pos2::new(r.left() + x as f32, r.top() + y as f32))
            }
        }
    }

    fn pressure_now(&self) -> f32 {
        if self.s.pen_pressure {
            self.pressure.unwrap_or(1.0)
        } else {
            1.0
        }
    }

    fn pick_color(&mut self, d: Vec3) {
        let p = dir_to_px(d, self.doc.width, self.doc.height);
        let x = (p.x.floor() as i64).rem_euclid(self.doc.width as i64) as u32;
        let y = (p.y.floor() as i64).clamp(0, self.doc.height as i64 - 1) as u32;
        let c = self.doc.composite_pixel(x, y);
        if c[3] > 0 {
            self.set_primary([c[0], c[1], c[2]]);
        }
    }

    fn begin_paint(&mut self, ctx: &egui::Context, d: Vec3, shift: bool) {
        let Tool::Paint(kind) = self.tool else { return };
        let b = self.brush();
        let pr = self.pressure_now();
        let ok = match (shift, self.stroke.last_end) {
            (true, Some(a)) => {
                let ok = self.stroke.line(&self.doc, kind, b, self.primary, a, d, pr);
                if ok {
                    self.finish_stroke();
                }
                return self.after_begin(ctx, ok, false);
            }
            _ => self.stroke.begin(&self.doc, kind, b, self.primary, d, pr),
        };
        self.after_begin(ctx, ok, true);
    }

    fn after_begin(&mut self, ctx: &egui::Context, ok: bool, drag: bool) {
        if !ok {
            let hidden = self.doc.active_layer().map(|l| !l.visible).unwrap_or(false);
            self.toast(
                ctx,
                ToastKind::Error,
                tr(if hidden {
                    "toast.layer_hidden"
                } else {
                    "toast.no_layer"
                }),
            );
            self.drag = Drag::None;
        } else if drag {
            self.drag = Drag::Paint;
            self.commit_color();
        }
    }

    fn do_fill(&mut self, ctx: &egui::Context, d: Vec3) {
        let p = dir_to_px(d, self.doc.width, self.doc.height);
        let x = (p.x.floor() as i64).rem_euclid(self.doc.width as i64) as u32;
        let y = (p.y.floor() as i64).clamp(0, self.doc.height as i64 - 1) as u32;
        if self.doc.active_layer().map(|l| !l.visible).unwrap_or(true) {
            self.toast(ctx, ToastKind::Error, tr("toast.layer_hidden"));
            return;
        }
        let a = self.doc.active;
        if let Some(e) =
            panoraminal_core::fill::flood_fill(&mut self.doc, x, y, self.primary, self.s.fill)
        {
            self.hist.push(e, "hist.fill", a, a);
            self.stale_thumbs.insert(a);
            self.commit_color();
        }
    }
}

pub fn show(app: &mut PanoApp, ui: &mut egui::Ui, now: f64) {
    let ctx = ui.ctx().clone();
    let pal = app.pal;
    let rect = ui.max_rect();
    let first_layout = app.canvas_rect == Rect::NOTHING;
    app.canvas_rect = rect;
    if first_layout && app.mode == ViewMode::Flat {
        app.flat = panoraminal_core::FlatView::fit(rect.width() as f64, rect.height() as f64);
    }
    let resp = ui.interact(rect, ui.scope_id().with("canvas"), Sense::click_and_drag());

    // ---- draw
    let ppp = ctx.pixels_per_point();
    let style = DisplayStyle {
        checker_a: if pal.dark {
            [0.16, 0.19, 0.27]
        } else {
            [0.86, 0.89, 0.94]
        },
        checker_b: if pal.dark {
            [0.22, 0.26, 0.36]
        } else {
            [0.96, 0.97, 0.99]
        },
        checker_cell: 10.0 * ppp,
        background: c3(pal.canvas_bg),
        wrap_dim: if pal.dark { 0.45 } else { 0.55 },
        grid: app.s.show_grid,
        grid_color: c3(pal.neon),
        grid_alpha: if pal.dark { 0.55 } else { 0.7 },
    };
    let params = DisplayParams {
        camera: app.cam,
        flat: app.flat,
        mode: app.mode,
        viewport_px: [
            rect.left() * ppp,
            rect.top() * ppp,
            rect.width() * ppp,
            rect.height() * ppp,
        ],
        pixels_per_point: ppp,
        style,
    };
    if app.gpu_ok {
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            CanvasCallback(params),
        ));
    } else {
        ui.painter().rect_filled(rect, 0.0, pal.canvas_bg);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            tr("error.no_gpu"),
            egui::FontId::proportional(16.0),
            pal.danger,
        );
    }

    // ---- input
    let blocked = app.splash_until > now
        || app.dlg.unsaved.is_some()
        || app.dlg.new.is_some()
        || app.dlg.error.is_some()
        || app.dlg.recovery.is_some()
        || app.dlg.offset.is_some()
        || app.dlg.export_jpg;
    let typing = ctx.egui_wants_keyboard_input();
    let (events, space, mods, dt) = ctx.input(|i| {
        (
            i.events.clone(),
            i.key_down(Key::Space) && !typing,
            i.modifiers,
            i.stable_dt.min(0.1),
        )
    });
    let hovered = resp.hovered() || resp.contains_pointer();
    let (vw, vh) = (rect.width() as f64, rect.height() as f64);
    let invert = if app.s.invert_drag { -1.0 } else { 1.0 };

    if !blocked {
        for ev in &events {
            match ev {
                Event::Touch {
                    force: Some(f),
                    phase,
                    ..
                } => {
                    if matches!(phase, TouchPhase::Start | TouchPhase::Move) {
                        app.pressure = Some(f.clamp(0.0, 1.0));
                    } else {
                        app.pressure = None;
                    }
                }
                Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    modifiers,
                } if hovered && app.drag == Drag::None && rect.contains(*pos) => {
                    app.last_pointer = Some(*pos);
                    match button {
                        PointerButton::Primary if space => app.drag = Drag::Rotate,
                        PointerButton::Primary => {
                            let Some(d) = app.screen_to_dir(*pos) else {
                                continue;
                            };
                            match app.tool {
                                Tool::Paint(_) if modifiers.alt => {
                                    app.pick_color(d);
                                    app.drag = Drag::Pick;
                                }
                                Tool::Paint(_) => app.begin_paint(&ctx, d, modifiers.shift),
                                Tool::Eyedropper => {
                                    app.pick_color(d);
                                    app.drag = Drag::Pick;
                                }
                                Tool::Fill => app.do_fill(&ctx, d),
                            }
                        }
                        PointerButton::Secondary | PointerButton::Middle => app.drag = Drag::Rotate,
                        _ => {}
                    }
                }
                Event::PointerMoved(pos) => {
                    let last = app.last_pointer.unwrap_or(*pos);
                    match app.drag {
                        Drag::Paint => {
                            if let Some(d) = app.screen_to_dir(*pos) {
                                let pr = app.pressure_now();
                                app.stroke.add_point(d, pr, false);
                            }
                        }
                        Drag::Pick => {
                            if let Some(d) = app.screen_to_dir(*pos) {
                                app.pick_color(d);
                            }
                        }
                        Drag::Rotate => {
                            let dl = *pos - last;
                            match app.mode {
                                ViewMode::Sphere => {
                                    app.cam.drag(dl.x as f64 * invert, dl.y as f64 * invert, vh)
                                }
                                ViewMode::Flat => app.flat.pan(dl.x as f64, dl.y as f64, vw),
                            }
                            app.view_anim = None;
                        }
                        Drag::None => {}
                    }
                    app.last_pointer = Some(*pos);
                }
                Event::PointerButton {
                    pos,
                    pressed: false,
                    ..
                } => {
                    if app.drag == Drag::Paint {
                        if let Some(d) = app.screen_to_dir(*pos) {
                            let pr = app.pressure_now();
                            app.stroke.add_point(d, pr, true);
                        }
                        app.finish_stroke();
                    } else if app.drag == Drag::Pick {
                        app.commit_color();
                    }
                    app.drag = Drag::None;
                }
                Event::PointerGone => {
                    if app.drag == Drag::Paint {
                        app.finish_stroke();
                    }
                    app.drag = Drag::None;
                }
                _ => {}
            }
        }
        // Make sure a lost release (e.g. window lost focus) ends the stroke.
        if app.drag != Drag::None && !ctx.input(|i| i.pointer.any_down()) {
            if app.drag == Drag::Paint {
                app.finish_stroke();
            }
            app.drag = Drag::None;
        }

        // Pinch / two-finger gestures.
        if let Some(mt) = ctx.multi_touch() {
            if app.stroke.is_active() {
                app.stroke.cancel();
                app.drag = Drag::None;
            }
            match app.mode {
                ViewMode::Sphere => {
                    app.cam.zoom(1.0 / mt.zoom_delta.max(0.01) as f64);
                    app.cam.drag(
                        mt.translation_delta.x as f64,
                        mt.translation_delta.y as f64,
                        vh,
                    );
                }
                ViewMode::Flat => {
                    let c = mt.center_pos - rect.min;
                    app.flat
                        .zoom_at(mt.zoom_delta as f64, c.x as f64, c.y as f64, vw, vh);
                    app.flat.pan(
                        mt.translation_delta.x as f64,
                        mt.translation_delta.y as f64,
                        vw,
                    );
                }
            }
        }

        // Wheel / pinch-zoom.
        if hovered {
            let (scroll, zoom_d, ptr) =
                ctx.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
            let speed = app.s.wheel_zoom_speed as f64;
            let mut f = (-(scroll.y as f64) * 0.0018 * speed).exp();
            if (zoom_d - 1.0).abs() > 1e-4 {
                f /= zoom_d as f64;
            }
            if (f - 1.0).abs() > 1e-5 {
                app.view_anim = None;
                match app.mode {
                    ViewMode::Sphere => app.cam.zoom(f),
                    ViewMode::Flat => {
                        let p = ptr.unwrap_or(rect.center()) - rect.min;
                        app.flat.zoom_at(1.0 / f, p.x as f64, p.y as f64, vw, vh);
                    }
                }
            }
        }

        // Arrow keys rotate (or pan) continuously.
        if !typing {
            let (l, r, u, d) = ctx.input(|i| {
                (
                    i.key_down(Key::ArrowLeft),
                    i.key_down(Key::ArrowRight),
                    i.key_down(Key::ArrowUp),
                    i.key_down(Key::ArrowDown),
                )
            });
            let ax = (r as i32 - l as i32) as f64;
            let ay = (d as i32 - u as i32) as f64;
            if ax != 0.0 || ay != 0.0 {
                let step = 300.0 * dt as f64;
                match app.mode {
                    ViewMode::Sphere => app.cam.drag(-ax * step, -ay * step, vh),
                    ViewMode::Flat => app.flat.pan(-ax * step, -ay * step, vw),
                }
                ctx.request_repaint();
            }
        }
    }

    // ---- cursor
    let ptr = ctx
        .input(|i| i.pointer.hover_pos())
        .filter(|p| rect.contains(*p) && hovered);
    app.cursor_dir = ptr.and_then(|p| app.screen_to_dir(p));
    if hovered && !blocked {
        let icon = if app.drag == Drag::Rotate {
            CursorIcon::Grabbing
        } else if space {
            CursorIcon::Grab
        } else {
            match app.tool {
                Tool::Paint(_) if mods.alt => CursorIcon::Crosshair,
                Tool::Paint(_) => CursorIcon::None,
                Tool::Eyedropper | Tool::Fill => CursorIcon::Crosshair,
            }
        };
        ctx.set_cursor_icon(icon);
    }
    if let (Some(p), Some(d), Tool::Paint(_)) = (ptr, app.cursor_dir, app.tool) {
        if !space && app.drag != Drag::Rotate && !mods.alt {
            draw_brush_ring(app, ui.painter(), p, d);
        }
    }
    if let (Some(p), Tool::Eyedropper) = (ptr, app.tool) {
        if let Some(d) = app.cursor_dir {
            let px = dir_to_px(d, app.doc.width, app.doc.height);
            let c = app.doc.composite_pixel(
                (px.x as u32).min(app.doc.width - 1),
                (px.y as u32).min(app.doc.height - 1),
            );
            let pr = ui.painter();
            pr.circle_filled(
                p + Vec2::new(18.0, -18.0),
                10.0,
                Color32::from_rgb(c[0], c[1], c[2]),
            );
            pr.circle_stroke(
                p + Vec2::new(18.0, -18.0),
                10.0,
                Stroke::new(2.0, Color32::WHITE),
            );
        }
    }
    // Shift: preview straight line from the last stroke end.
    if let (Some(p), Some(a), Tool::Paint(_), true) = (
        ptr,
        app.stroke.last_end,
        app.tool,
        mods.shift && app.drag == Drag::None,
    ) {
        let mut pts = Vec::new();
        if let Some(b) = app.cursor_dir {
            for i in 0..=48 {
                let t = i as f64 / 48.0;
                let q = panoraminal_core::math::slerp(a, b, t);
                if let Some(s) = app.dir_to_screen(q, p) {
                    pts.push(s);
                }
            }
        }
        if pts.len() > 1 {
            ui.painter().add(egui::Shape::dashed_line(
                &pts,
                Stroke::new(1.5, with_alpha(pal.neon, 0.9)),
                6.0,
                4.0,
            ));
        }
    }

    // Keep painting smoothly while a stroke is active.
    if app.drag != Drag::None {
        ctx.request_repaint();
    }
}

fn draw_brush_ring(app: &PanoApp, painter: &egui::Painter, p: Pos2, d: Vec3) {
    let b = app.brush();
    let (w, h) = (app.doc.width, app.doc.height);
    let px = dir_to_px(d, w, h);
    let mut pressure = app.pressure_now();
    if !b.pressure_size {
        pressure = 1.0;
    }
    let size_k = if b.pressure_size {
        b.min_size + (1.0 - b.min_size) * pressure
    } else {
        1.0
    };
    let r = (b.size * size_k * 0.5).max(0.5) as f64;
    let lat_cos = px.lat.cos().max(1.0 / panoraminal_core::brush::MAX_STRETCH);
    let rx = if b.polar_correction { r / lat_cos } else { r };
    let ry = r * (2.0 * h as f64 / w as f64);
    let n = 64;
    let mut pts = Vec::with_capacity(n + 1);
    let mut broken = false;
    for i in 0..=n {
        let t = i as f64 / n as f64 * std::f64::consts::TAU;
        let x = (px.x + rx * t.cos()).rem_euclid(w as f64);
        let y = (px.y + ry * t.sin()).clamp(0.0, h as f64 - 1e-3);
        match app.dir_to_screen(px_to_dir(x, y, w, h), p) {
            Some(s) => pts.push(s),
            None => broken = true,
        }
    }
    let span = pts
        .iter()
        .fold(Rect::NOTHING, |r, q| r.union(Rect::from_min_max(*q, *q)));
    let tiny = span.width() < 4.0;
    if broken || tiny || pts.len() < 3 {
        // Cross-hair fallback.
        let k = 6.0;
        for (a, b2) in [
            (p - Vec2::new(k, 0.0), p + Vec2::new(k, 0.0)),
            (p - Vec2::new(0.0, k), p + Vec2::new(0.0, k)),
        ] {
            painter.line_segment([a, b2], Stroke::new(3.0, Color32::from_black_alpha(150)));
            painter.line_segment([a, b2], Stroke::new(1.2, Color32::WHITE));
        }
        if tiny || pts.len() < 3 {
            return;
        }
    }
    painter.add(egui::Shape::line(
        pts.clone(),
        Stroke::new(3.0, Color32::from_black_alpha(130)),
    ));
    let col = if b.hardness > 0.0 {
        with_alpha(app.pal.neon, 0.95)
    } else {
        Color32::WHITE
    };
    painter.add(egui::Shape::line(pts, Stroke::new(1.3, col)));
    painter.circle_filled(p, 1.6, Color32::WHITE);
}
