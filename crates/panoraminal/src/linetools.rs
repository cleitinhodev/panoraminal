//! Line layer tools (1.4): curve, freehand vector line, point editing and thickness.
//!
//! The strokes live in the core ([`panoraminal_core::lines`]); this module turns pointer
//! input into edits. Every edit is applied live (the layer's raster cache is updated
//! immediately, so the GPU view is exact) and recorded as one undo step when it ends.

use crate::app::{PanoApp, ToastKind};
use eframe::egui::{self, Pos2};
use panoraminal_core::history::ops;
use panoraminal_core::lines::{self, LineArt, LinePoint, LineStroke, Rotation};
use panoraminal_core::math::angle_between;
use panoraminal_core::{LayerId, Vec3};
use panoraminal_i18n::{tr, trf};
use serde::{Deserialize, Serialize};

/// The four line tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineTool {
    /// Click to place control points; the curve goes through them.
    Curve,
    /// Draw freely; the stroke becomes editable control points.
    Freehand,
    /// Move, insert, delete, split and join points; move whole strokes.
    Edit,
    /// Drag a point (its width) or a stroke (the whole width).
    Thickness,
}

impl LineTool {
    pub const ALL: [LineTool; 4] = [
        LineTool::Curve,
        LineTool::Freehand,
        LineTool::Edit,
        LineTool::Thickness,
    ];
    pub fn label(self) -> &'static str {
        match self {
            LineTool::Curve => "tool.line_curve",
            LineTool::Freehand => "tool.line_free",
            LineTool::Edit => "tool.line_edit",
            LineTool::Thickness => "tool.line_width",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            LineTool::Curve => "line-curve",
            LineTool::Freehand => "line-free",
            LineTool::Edit => "line-edit",
            LineTool::Thickness => "line-width",
        }
    }
    /// Tools that create strokes (they make a line layer if needed).
    pub fn draws(self) -> bool {
        matches!(self, LineTool::Curve | LineTool::Freehand)
    }
}

/// Options for new strokes (persisted in the settings).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LineSettings {
    /// Diameter in document pixels.
    pub width: f32,
    pub opacity: f32,
    pub taper_start: f32,
    pub taper_end: f32,
    /// Freehand: how much the input is smoothed / simplified (0–1).
    pub smoothing: f32,
    /// Curves through the points (`false`: straight great-circle segments).
    pub smooth: bool,
    /// Freehand: pen pressure sets the point widths.
    pub pressure: bool,
    /// Dropping an end point on another end point joins the strokes.
    pub snap_join: bool,
}

impl Default for LineSettings {
    fn default() -> Self {
        Self {
            width: 6.0,
            opacity: 1.0,
            taper_start: 0.08,
            taper_end: 0.2,
            smoothing: 0.5,
            smooth: true,
            pressure: true,
            snap_join: true,
        }
    }
}

impl LineSettings {
    pub fn clamp(&mut self) {
        let d = Self::default();
        let f =
            |v: f32, lo: f32, hi: f32, def: f32| if v.is_finite() { v.clamp(lo, hi) } else { def };
        self.width = f(self.width, lines::MIN_WIDTH, lines::MAX_WIDTH, d.width);
        self.opacity = f(self.opacity, 0.01, 1.0, d.opacity);
        self.taper_start = f(self.taper_start, 0.0, 1.0, d.taper_start);
        self.taper_end = f(self.taper_end, 0.0, 1.0, d.taper_end);
        self.smoothing = f(self.smoothing, 0.0, 1.0, d.smoothing);
    }
    /// An empty stroke with these options.
    pub fn stroke(&self, color: [u8; 3]) -> LineStroke {
        LineStroke {
            points: Vec::new(),
            color,
            opacity: self.opacity,
            width: self.width,
            taper_start: self.taper_start,
            taper_end: self.taper_end,
            smooth: self.smooth,
            closed: false,
        }
    }
}

/// Selected stroke (and point) of the active line layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sel {
    pub layer: LayerId,
    pub stroke: usize,
    pub point: Option<usize>,
}

/// What is under the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Point { stroke: usize, point: usize },
    Segment { stroke: usize, seg: usize, t: f64 },
}

impl Hit {
    pub fn stroke(&self) -> usize {
        match *self {
            Hit::Point { stroke, .. } | Hit::Segment { stroke, .. } => stroke,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Op {
    /// A curve being drawn; `dragging` = the point just placed follows the pointer.
    Curve {
        stroke: usize,
        dragging: bool,
    },
    Free {
        stroke: usize,
        raw: Vec<(Vec3, f32)>,
    },
    MovePoint {
        stroke: usize,
        point: usize,
    },
    MoveStroke {
        stroke: usize,
        start: Vec3,
        orig: LineStroke,
    },
    PointWidth {
        stroke: usize,
        point: usize,
        start: Pos2,
        w0: f32,
    },
    StrokeWidth {
        stroke: usize,
        start: Pos2,
        w0: f32,
    },
}

/// An edit in progress: applied live, recorded when it ends.
#[derive(Clone, Debug)]
pub struct Active {
    pub layer: LayerId,
    pub before: LineArt,
    pub label: &'static str,
    pub op: Op,
}

#[derive(Default)]
pub struct LineState {
    pub sel: Option<Sel>,
    pub hover: Option<Hit>,
    pub active: Option<Active>,
    /// Strokes before a change made from the panel (committed when the pointer is up).
    pub panel_before: Option<(LayerId, LineArt)>,
}

/// What a press on the canvas started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    None,
    /// A line edit that follows the pointer until release.
    Drag,
    /// Nothing under the pointer: rotate / pan the view instead.
    Rotate,
}

/// Screen distance (points) for grabbing points and lines.
pub const GRAB: f32 = 8.0;

// ------------------------------------------------------------------ geometry helpers

/// Projects a stroke onto the screen as polylines (split where it leaves the view).
/// `step` is the sampling angle; `near` picks the wrapped copy in the flat view.
pub fn screen_polylines(
    s: &LineStroke,
    step: f64,
    near: Pos2,
    proj: &dyn Fn(Vec3, Pos2) -> Option<Pos2>,
) -> Vec<Vec<Pos2>> {
    let mut out: Vec<Vec<Pos2>> = Vec::new();
    let mut cur: Vec<Pos2> = Vec::new();
    let mut last = near;
    for smp in s.centerline(step) {
        match proj(smp.dir, last) {
            Some(p) if p.x.is_finite() && p.y.is_finite() => {
                if let Some(q) = cur.last() {
                    // A jump means the curve went behind the camera.
                    if (p - *q).length() > 4000.0 {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                cur.push(p);
                last = p;
            }
            _ => {
                if cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> (f32, f32) {
    let ab = b - a;
    let l2 = ab.length_sq();
    let t = if l2 < 1e-9 {
        0.0
    } else {
        ((p - a).dot(ab) / l2).clamp(0.0, 1.0)
    };
    ((a + ab * t - p).length(), t)
}

/// Finds the point or line under `pos`. Points win over lines; the `prefer` stroke
/// (the selection) wins ties. `step` = sampling angle, `px_angle` = angle of one
/// screen point (for line widths), `doc_w` = document width.
pub fn hit_test(
    art: &LineArt,
    pos: Pos2,
    prefer: Option<usize>,
    step: f64,
    px_angle: f64,
    doc_w: u32,
    proj: &dyn Fn(Vec3, Pos2) -> Option<Pos2>,
) -> Option<Hit> {
    let mut order: Vec<usize> = (0..art.strokes.len()).rev().collect();
    if let Some(p) = prefer.filter(|p| *p < art.strokes.len()) {
        order.retain(|i| *i != p);
        order.insert(0, p);
    }
    let mut best: Option<(f32, Hit)> = None;
    for &si in &order {
        for (pi, p) in art.strokes[si].points.iter().enumerate() {
            if let Some(sp) = proj(p.dir(), pos) {
                let d = (sp - pos).length();
                if d <= GRAB && best.is_none_or(|(bd, _)| d < bd - 0.5) {
                    best = Some((
                        d,
                        Hit::Point {
                            stroke: si,
                            point: pi,
                        },
                    ));
                }
            }
        }
    }
    if best.is_some() {
        return best.map(|b| b.1);
    }
    for &si in &order {
        let s = &art.strokes[si];
        let half = (s.base_radius(doc_w) / px_angle.max(1e-12)) as f32;
        let n = s.points.len();
        if n == 1 {
            if let Some(sp) = proj(s.points[0].dir(), pos) {
                let d = (sp - pos).length();
                if d <= GRAB.max(half * s.points[0].w) {
                    best = Some((
                        d,
                        Hit::Point {
                            stroke: si,
                            point: 0,
                        },
                    ));
                }
            }
            continue;
        }
        for seg in 0..s.segment_count() {
            let a = s.points[seg % n].dir();
            let b = s.points[(seg + 1) % n].dir();
            let k = ((angle_between(a, b) * 1.3 / step).ceil() as usize).clamp(2, 512);
            let mut prev: Option<(Pos2, f64)> = None;
            for j in 0..=k {
                let t = j as f64 / k as f64;
                let Some(sp) = proj(s.eval(seg, t), pos) else {
                    prev = None;
                    continue;
                };
                if let Some((pp, pt)) = prev {
                    if (sp - pp).length() < 2000.0 {
                        let (d, u) = seg_dist(pos, pp, sp);
                        let tol = GRAB.max(half * s.width_at(seg, t) as f32 + 2.0);
                        if d <= tol && best.is_none_or(|(bd, _)| d < bd) {
                            best = Some((
                                d,
                                Hit::Segment {
                                    stroke: si,
                                    seg,
                                    t: pt + (t - pt) * u as f64,
                                },
                            ));
                        }
                    }
                }
                prev = Some((sp, t));
            }
        }
        if best.is_some() && Some(si) == prefer {
            break;
        }
    }
    best.map(|b| b.1)
}

/// Another open end point within `GRAB` of point `(si, pi)` (which must be an end of an
/// open stroke): `(stroke, is_last_point)`.
pub fn join_target(
    art: &LineArt,
    si: usize,
    pi: usize,
    proj: &dyn Fn(Vec3, Pos2) -> Option<Pos2>,
    near: Pos2,
) -> Option<(usize, bool)> {
    let s = art.strokes.get(si)?;
    let n = s.points.len();
    if s.closed || n < 1 || (pi != 0 && pi != n - 1) {
        return None;
    }
    let me = proj(s.points[pi].dir(), near)?;
    let mut best: Option<(f32, usize, bool)> = None;
    for (oi, o) in art.strokes.iter().enumerate() {
        if o.closed || o.points.is_empty() {
            continue;
        }
        let m = o.points.len();
        for (end, idx) in [(false, 0usize), (true, m - 1)] {
            if oi == si && (idx == pi || n < 4) {
                continue;
            }
            if oi != si && m == 1 && end {
                continue;
            }
            if let Some(sp) = proj(o.points[idx].dir(), near) {
                let d = (sp - me).length();
                if d <= GRAB * 1.25 && best.is_none_or(|b| d < b.0) {
                    best = Some((d, oi, end));
                }
            }
        }
    }
    best.map(|b| (b.1, b.2))
}

// ------------------------------------------------------------------ app integration

impl PanoApp {
    /// Angle (radians) covered by one screen point at the view centre.
    pub fn angle_per_point(&self) -> f64 {
        let r = self.canvas_rect;
        match self.mode {
            panoraminal_render::ViewMode::Sphere => {
                self.cam.fov.to_radians() / (r.height().max(1.0) as f64)
            }
            panoraminal_render::ViewMode::Flat => {
                let s = self.flat.scale(r.width().max(1.0) as f64, self.doc.width);
                std::f64::consts::TAU / (self.doc.width as f64 * s.max(1e-9))
            }
        }
    }

    /// Sampling angle for drawing / hit-testing lines on screen (~3 points).
    pub fn line_screen_step(&self) -> f64 {
        (self.angle_per_point() * 3.0).max(1e-5)
    }

    pub fn active_line_art(&self) -> Option<&LineArt> {
        self.doc.active_layer().and_then(|l| l.lines.as_ref())
    }

    pub fn is_line_tool(&self) -> Option<LineTool> {
        match self.tool {
            crate::app::Tool::Line(t) => Some(t),
            _ => None,
        }
    }

    /// True while a line edit (drag or curve) is in progress.
    pub fn line_busy(&self) -> bool {
        self.lines.active.is_some()
    }

    fn proj(&self) -> impl Fn(Vec3, Pos2) -> Option<Pos2> + '_ {
        move |d, near| self.dir_to_screen(d, near)
    }

    /// The point or line under the pointer on the active line layer.
    pub fn line_hit(&self, pos: Pos2) -> Option<Hit> {
        let art = self.active_line_art()?;
        let prefer = self
            .lines
            .sel
            .filter(|s| s.layer == self.doc.active)
            .map(|s| s.stroke);
        let proj = self.proj();
        hit_test(
            art,
            pos,
            prefer,
            self.line_screen_step(),
            self.angle_per_point(),
            self.doc.width,
            &proj,
        )
    }

    /// Applies new strokes to a line layer (live, no history).
    pub fn line_apply(&mut self, id: LayerId, art: LineArt) {
        if lines::apply(&mut self.doc, id, art) {
            self.stale_thumbs.insert(id);
        }
    }

    fn line_art_clone(&self, id: LayerId) -> Option<LineArt> {
        self.doc.layer(id).and_then(|l| l.lines.clone())
    }

    /// The active line layer, or a new one when a drawing tool is used on a raster layer.
    fn line_layer_for_drawing(&mut self, ctx: &egui::Context) -> Option<LayerId> {
        if let Some(l) = self.doc.active_layer() {
            if l.is_lines() {
                if !l.visible {
                    self.toast(ctx, ToastKind::Error, tr("toast.layer_hidden"));
                    return None;
                }
                return Some(l.id);
            }
        }
        let n = self.doc.layers.iter().filter(|l| l.is_lines()).count() + 1;
        let name = trf("layer.lines_name", &[("n", &n)]);
        match ops::add_line_layer(&mut self.doc, &mut self.hist, &name) {
            Some(id) => {
                self.stale_thumbs.insert(id);
                self.toast(ctx, ToastKind::Info, tr("toast.line_layer_created"));
                Some(id)
            }
            None => {
                self.toast(ctx, ToastKind::Error, tr("toast.max_layers"));
                None
            }
        }
    }

    /// Creates a new empty line layer (menu / layers panel).
    pub fn new_line_layer(&mut self, ctx: &egui::Context) {
        self.finish_stroke();
        let n = self.doc.layers.iter().filter(|l| l.is_lines()).count() + 1;
        let name = trf("layer.lines_name", &[("n", &n)]);
        match ops::add_line_layer(&mut self.doc, &mut self.hist, &name) {
            Some(id) => {
                self.stale_thumbs.insert(id);
            }
            None => self.toast(ctx, ToastKind::Error, tr("toast.max_layers")),
        }
    }

    /// Converts the active line layer into a normal raster layer.
    pub fn rasterize_line_layer(&mut self, ctx: &egui::Context) {
        self.finish_stroke();
        let id = self.doc.active;
        if ops::rasterize_lines(&mut self.doc, &mut self.hist, id) {
            self.lines.sel = None;
            self.toast(ctx, ToastKind::Ok, tr("toast.lines_rasterized"));
        }
    }

    fn line_begin(&mut self, layer: LayerId, label: &'static str, op: Op) -> bool {
        let Some(before) = self.line_art_clone(layer) else {
            return false;
        };
        self.lines.active = Some(Active {
            layer,
            before,
            label,
            op,
        });
        true
    }

    /// Ends the current line edit and records it.
    pub fn line_finish(&mut self) {
        let Some(a) = self.lines.active.take() else {
            return;
        };
        let mut sel_stroke = None;
        if let Op::Curve { stroke, .. } = a.op {
            // A curve needs two points.
            if let Some(mut art) = self.line_art_clone(a.layer) {
                if art.strokes.get(stroke).is_some_and(|s| s.points.len() < 2) {
                    art.strokes.remove(stroke);
                    self.line_apply(a.layer, art);
                } else {
                    sel_stroke = Some(stroke);
                }
            }
        }
        if ops::commit_lines(&mut self.doc, &mut self.hist, a.layer, a.before, a.label) {
            self.stale_thumbs.insert(a.layer);
            if let Some(s) = sel_stroke {
                self.lines.sel = Some(Sel {
                    layer: a.layer,
                    stroke: s,
                    point: None,
                });
            }
        }
    }

    /// Cancels the current line edit (restores the strokes).
    pub fn line_cancel(&mut self) {
        if let Some(a) = self.lines.active.take() {
            self.line_apply(a.layer, a.before);
        }
    }

    /// Records changes made from the panel once the pointer is released.
    pub fn line_panel_commit(&mut self, pointer_down: bool) {
        if pointer_down {
            return;
        }
        if let Some((id, before)) = self.lines.panel_before.take() {
            ops::commit_lines(&mut self.doc, &mut self.hist, id, before, "hist.line_style");
        }
    }

    /// Changes the active line layer from the panel (live; recorded on release).
    pub fn line_panel_edit(&mut self, f: impl FnOnce(&mut LineArt)) {
        let id = self.doc.active;
        let Some(mut art) = self.line_art_clone(id) else {
            return;
        };
        let before = art.clone();
        f(&mut art);
        if art == before {
            return;
        }
        match &self.lines.panel_before {
            Some((pid, _)) if *pid == id => {}
            _ => {
                self.line_panel_commit(false);
                self.lines.panel_before = Some((id, before));
            }
        }
        self.line_apply(id, art);
    }

    /// Drops a selection that no longer exists (after undo, layer change…).
    pub fn line_validate(&mut self) {
        if let Some(s) = self.lines.sel {
            let ok = s.layer == self.doc.active
                && self
                    .active_line_art()
                    .and_then(|a| a.strokes.get(s.stroke))
                    .is_some_and(|st| s.point.is_none_or(|p| p < st.points.len()));
            if !ok {
                self.lines.sel = None;
            }
        }
        if let Some(a) = &self.lines.active {
            if self.doc.layer(a.layer).is_none_or(|l| !l.is_lines()) {
                self.lines.active = None;
            }
        }
    }

    /// Mouse / pen press with a line tool.
    pub fn line_press(
        &mut self,
        ctx: &egui::Context,
        tool: LineTool,
        pos: Pos2,
        d: Vec3,
        mods: egui::Modifiers,
        pressure: f32,
    ) -> Press {
        match tool {
            LineTool::Curve => self.curve_press(ctx, pos, d, mods.shift),
            LineTool::Freehand => {
                self.line_finish();
                let Some(id) = self.line_layer_for_drawing(ctx) else {
                    return Press::None;
                };
                let Some(mut art) = self.line_art_clone(id) else {
                    return Press::None;
                };
                let w = if self.s.lines.pressure { pressure } else { 1.0 };
                let mut st = self.s.lines.stroke(self.primary);
                st.smooth = false;
                st.points
                    .push(LinePoint::new(d, w.max(lines::MIN_POINT_WIDTH)));
                art.strokes.push(st);
                let stroke = art.strokes.len() - 1;
                self.line_begin(
                    id,
                    "hist.line_add",
                    Op::Free {
                        stroke,
                        raw: vec![(d, w)],
                    },
                );
                self.line_apply(id, art);
                self.commit_color();
                Press::Drag
            }
            LineTool::Edit | LineTool::Thickness => self.edit_press(ctx, tool, pos, d, mods),
        }
    }

    fn curve_press(&mut self, ctx: &egui::Context, pos: Pos2, d: Vec3, shift: bool) -> Press {
        // Continue the curve being drawn on this layer.
        if let Some(Active {
            layer,
            op: Op::Curve { stroke, .. },
            ..
        }) = self.lines.active.clone()
        {
            if layer == self.doc.active {
                let Some(mut art) = self.line_art_clone(layer) else {
                    return Press::None;
                };
                let Some(s) = art.strokes.get_mut(stroke) else {
                    self.lines.active = None;
                    return Press::None;
                };
                let n = s.points.len();
                let near = |p: &LinePoint| {
                    self.dir_to_screen(p.dir(), pos)
                        .is_some_and(|q| (q - pos).length() <= GRAB)
                };
                if n >= 3 && near(&s.points[0]) {
                    s.closed = true;
                    self.line_apply(layer, art);
                    self.line_finish();
                    return Press::None;
                }
                if n >= 1 && near(&s.points[n - 1]) {
                    // Clicking the last point again (double-click) finishes.
                    self.line_finish();
                    return Press::None;
                }
                if n >= lines::MAX_POINTS {
                    return Press::None;
                }
                let mut p = LinePoint::new(d, 1.0);
                p.sharp = shift;
                s.points.push(p);
                self.line_apply(layer, art);
                if let Some(a) = self.lines.active.as_mut() {
                    a.op = Op::Curve {
                        stroke,
                        dragging: true,
                    };
                }
                return Press::Drag;
            }
            self.line_finish();
        }
        let Some(id) = self.line_layer_for_drawing(ctx) else {
            return Press::None;
        };
        let Some(mut art) = self.line_art_clone(id) else {
            return Press::None;
        };
        let mut st = self.s.lines.stroke(self.primary);
        let mut p = LinePoint::new(d, 1.0);
        p.sharp = shift;
        st.points.push(p);
        art.strokes.push(st);
        let stroke = art.strokes.len() - 1;
        self.line_begin(
            id,
            "hist.line_add",
            Op::Curve {
                stroke,
                dragging: true,
            },
        );
        self.line_apply(id, art);
        self.lines.sel = Some(Sel {
            layer: id,
            stroke,
            point: None,
        });
        self.commit_color();
        Press::Drag
    }

    fn edit_press(
        &mut self,
        ctx: &egui::Context,
        tool: LineTool,
        pos: Pos2,
        d: Vec3,
        mods: egui::Modifiers,
    ) -> Press {
        self.line_finish();
        let id = self.doc.active;
        if !self.doc.active_is_lines() {
            self.toast(ctx, ToastKind::Error, tr("toast.not_line_layer"));
            return Press::None;
        }
        let Some(hit) = self.line_hit(pos) else {
            self.lines.sel = None;
            return Press::Rotate;
        };
        let Some(mut art) = self.line_art_clone(id) else {
            return Press::None;
        };
        let stroke = hit.stroke();
        let ctrl = mods.command || mods.ctrl;
        if tool == LineTool::Thickness && ctrl {
            // Apply the current width / colour / opacity / tapers to the stroke.
            let ls = self.s.lines;
            let s = &mut art.strokes[stroke];
            s.width = ls.width;
            s.opacity = ls.opacity;
            s.taper_start = ls.taper_start;
            s.taper_end = ls.taper_end;
            s.color = self.primary;
            ops::set_lines(&mut self.doc, &mut self.hist, id, art, "hist.line_style");
            self.stale_thumbs.insert(id);
            self.lines.sel = Some(Sel {
                layer: id,
                stroke,
                point: None,
            });
            return Press::None;
        }
        match (tool, hit) {
            (LineTool::Edit, Hit::Point { stroke, point }) => {
                if ctrl {
                    let p = &mut art.strokes[stroke].points[point];
                    p.sharp = !p.sharp;
                    ops::set_lines(&mut self.doc, &mut self.hist, id, art, "hist.line_edit");
                    self.lines.sel = Some(Sel {
                        layer: id,
                        stroke,
                        point: Some(point),
                    });
                    return Press::None;
                }
                if mods.alt {
                    if lines::split_at(&mut art, stroke, point) {
                        ops::set_lines(&mut self.doc, &mut self.hist, id, art, "hist.line_split");
                        self.stale_thumbs.insert(id);
                    }
                    self.lines.sel = Some(Sel {
                        layer: id,
                        stroke,
                        point: None,
                    });
                    return Press::None;
                }
                self.lines.sel = Some(Sel {
                    layer: id,
                    stroke,
                    point: Some(point),
                });
                self.line_begin(id, "hist.line_edit", Op::MovePoint { stroke, point });
                Press::Drag
            }
            (LineTool::Edit, Hit::Segment { stroke, seg, t }) => {
                if ctrl {
                    let Some(point) = lines::insert_point(&mut art, stroke, seg, t) else {
                        return Press::None;
                    };
                    self.line_begin(id, "hist.line_insert", Op::MovePoint { stroke, point });
                    self.line_apply(id, art);
                    self.lines.sel = Some(Sel {
                        layer: id,
                        stroke,
                        point: Some(point),
                    });
                    return Press::Drag;
                }
                self.lines.sel = Some(Sel {
                    layer: id,
                    stroke,
                    point: None,
                });
                let orig = art.strokes[stroke].clone();
                self.line_begin(
                    id,
                    "hist.line_move",
                    Op::MoveStroke {
                        stroke,
                        start: d,
                        orig,
                    },
                );
                Press::Drag
            }
            (_, Hit::Point { stroke, point }) => {
                self.lines.sel = Some(Sel {
                    layer: id,
                    stroke,
                    point: Some(point),
                });
                let w0 = art.strokes[stroke].points[point].w;
                self.line_begin(
                    id,
                    "hist.line_width",
                    Op::PointWidth {
                        stroke,
                        point,
                        start: pos,
                        w0,
                    },
                );
                Press::Drag
            }
            (_, Hit::Segment { stroke, .. }) => {
                self.lines.sel = Some(Sel {
                    layer: id,
                    stroke,
                    point: None,
                });
                let w0 = art.strokes[stroke].width;
                self.line_begin(
                    id,
                    "hist.line_width",
                    Op::StrokeWidth {
                        stroke,
                        start: pos,
                        w0,
                    },
                );
                Press::Drag
            }
        }
    }

    /// Pointer moved while a line edit is dragging.
    pub fn line_drag(&mut self, pos: Pos2, d: Option<Vec3>, pressure: f32) {
        let Some(a) = self.lines.active.clone() else {
            return;
        };
        let Some(mut art) = self.line_art_clone(a.layer) else {
            return;
        };
        let pa = panoraminal_core::lines::pixel_angle(self.doc.width, self.doc.height);
        let mut new_op = None;
        match a.op {
            Op::Curve {
                stroke,
                dragging: true,
            } => {
                let (Some(d), Some(s)) = (d, art.strokes.get_mut(stroke)) else {
                    return;
                };
                if let Some(p) = s.points.last_mut() {
                    p.set_dir(d);
                }
            }
            Op::Curve { .. } => return,
            Op::Free { stroke, mut raw } => {
                let Some(d) = d else { return };
                let last = raw.last().map(|r| r.0).unwrap_or(d);
                if angle_between(last, d) < pa * 0.75 {
                    return;
                }
                let w = if self.s.lines.pressure { pressure } else { 1.0 };
                raw.push((d, w));
                if let Some(s) = art.strokes.get_mut(stroke) {
                    s.points = raw
                        .iter()
                        .take(lines::MAX_POINTS)
                        .map(|(d, w)| LinePoint::new(*d, w.max(lines::MIN_POINT_WIDTH)))
                        .collect();
                }
                new_op = Some(Op::Free { stroke, raw });
            }
            Op::MovePoint { stroke, point } => {
                let Some(d) = d else { return };
                let Some(p) = art
                    .strokes
                    .get_mut(stroke)
                    .and_then(|s| s.points.get_mut(point))
                else {
                    return;
                };
                p.set_dir(d);
            }
            Op::MoveStroke {
                stroke,
                start,
                ref orig,
            } => {
                let Some(d) = d else { return };
                let mut s = orig.clone();
                s.rotate(&Rotation::between(start, d));
                if let Some(t) = art.strokes.get_mut(stroke) {
                    *t = s;
                }
            }
            Op::PointWidth {
                stroke,
                point,
                start,
                w0,
            } => {
                let k = ((pos.x - start.x) - (pos.y - start.y)) / 90.0;
                if let Some(p) = art
                    .strokes
                    .get_mut(stroke)
                    .and_then(|s| s.points.get_mut(point))
                {
                    p.w = (w0 * k.exp()).clamp(lines::MIN_POINT_WIDTH, lines::MAX_POINT_WIDTH);
                }
            }
            Op::StrokeWidth { stroke, start, w0 } => {
                let k = ((pos.x - start.x) - (pos.y - start.y)) / 120.0;
                if let Some(s) = art.strokes.get_mut(stroke) {
                    s.width = (w0 * k.exp()).clamp(lines::MIN_WIDTH, lines::MAX_WIDTH);
                }
            }
        }
        if let (Some(op), Some(act)) = (new_op, self.lines.active.as_mut()) {
            act.op = op;
        }
        self.line_apply(a.layer, art);
    }

    /// Pointer released after a line drag.
    pub fn line_release(&mut self, pos: Pos2) {
        let Some(a) = self.lines.active.clone() else {
            return;
        };
        match a.op {
            Op::Curve { stroke, .. } => {
                // Keep drawing the curve: the next click adds a point.
                if let Some(act) = self.lines.active.as_mut() {
                    act.op = Op::Curve {
                        stroke,
                        dragging: false,
                    };
                }
            }
            Op::Free { stroke, raw } => {
                let Some(mut art) = self.line_art_clone(a.layer) else {
                    return;
                };
                let screen = self.angle_per_point();
                let doc_px = lines::pixel_angle(self.doc.width, self.doc.height);
                let sm = self.s.lines.smoothing;
                let tol = doc_px.max(screen * 0.6) * (0.5 + sm as f64 * 3.0);
                let pts = lines::simplify(&raw, tol, sm);
                if let Some(s) = art.strokes.get_mut(stroke) {
                    s.points = pts;
                    s.smooth = self.s.lines.smooth;
                }
                self.line_apply(a.layer, art);
                self.lines.sel = Some(Sel {
                    layer: a.layer,
                    stroke,
                    point: None,
                });
                self.line_finish();
            }
            Op::MovePoint { stroke, point } => {
                if self.s.lines.snap_join {
                    if let Some(mut art) = self.line_art_clone(a.layer) {
                        let target = {
                            let proj = self.proj();
                            join_target(&art, stroke, point, &proj, pos)
                        };
                        if let Some((other, other_end)) = target {
                            let is_last = point + 1 == art.strokes[stroke].points.len();
                            if let Some(r) =
                                lines::join(&mut art, stroke, is_last, other, other_end)
                            {
                                self.line_apply(a.layer, art);
                                if let Some(act) = self.lines.active.as_mut() {
                                    act.label = "hist.line_join";
                                }
                                self.lines.sel = Some(Sel {
                                    layer: a.layer,
                                    stroke: r,
                                    point: None,
                                });
                            }
                        }
                    }
                }
                self.line_finish();
            }
            _ => self.line_finish(),
        }
    }

    /// Delete / Backspace on a line layer: removes the last point of the curve being
    /// drawn, else the selected point (or stroke). Returns `true` if handled.
    pub fn line_delete(&mut self) -> bool {
        if let Some(Active {
            layer,
            op: Op::Curve { stroke, .. },
            ..
        }) = self.lines.active.clone()
        {
            // Remove the last placed point of the curve being drawn.
            let Some(mut art) = self.line_art_clone(layer) else {
                return true;
            };
            let empty = match art.strokes.get_mut(stroke) {
                Some(s) => {
                    s.points.pop();
                    s.points.is_empty()
                }
                None => true,
            };
            if empty {
                self.line_cancel();
            } else {
                self.line_apply(layer, art);
            }
            return true;
        }
        if self.line_busy() {
            return true;
        }
        let Some(sel) = self.lines.sel.filter(|s| s.layer == self.doc.active) else {
            return false;
        };
        let Some(mut art) = self.line_art_clone(sel.layer) else {
            return false;
        };
        if sel.stroke >= art.strokes.len() {
            return false;
        }
        match sel.point {
            Some(p) => {
                let gone = lines::delete_point(&mut art, sel.stroke, p);
                self.lines.sel = if gone {
                    None
                } else {
                    let n = art.strokes[sel.stroke].points.len();
                    Some(Sel {
                        point: Some(p.min(n - 1)),
                        ..sel
                    })
                };
            }
            None => {
                art.strokes.remove(sel.stroke);
                self.lines.sel = None;
            }
        }
        ops::set_lines(
            &mut self.doc,
            &mut self.hist,
            sel.layer,
            art,
            "hist.line_delete",
        );
        self.stale_thumbs.insert(sel.layer);
        true
    }
}

// ------------------------------------------------------------------ screenshot demo

impl PanoApp {
    /// `--line-demo edit|width|curve`: a line layer with a few strokes in front of the
    /// camera, for screenshots and manual testing.
    pub fn line_demo(&mut self, mode: &str) {
        let (f, r, u) = self.cam.basis();
        let at = |x: f64, y: f64, w: f32| LinePoint::new((f + r * x + u * y).normalize(), w);
        let mk = |pts: Vec<LinePoint>, color: [u8; 3], width: f32| LineStroke {
            points: pts,
            color,
            opacity: 1.0,
            width,
            taper_start: 0.1,
            taper_end: 0.25,
            smooth: true,
            closed: false,
        };
        let mut art = LineArt::default();
        // A wave whose thickness swells and thins point by point.
        art.strokes.push(mk(
            vec![
                at(-0.95, 0.42, 0.4),
                at(-0.6, 0.62, 1.0),
                at(-0.25, 0.38, 2.2),
                at(0.1, 0.58, 0.7),
                at(0.45, 0.36, 1.6),
                at(0.85, 0.55, 0.5),
            ],
            [0xff, 0x4d, 0x9d],
            12.0,
        ));
        // A closed shape with one sharp corner.
        let mut blob: Vec<LinePoint> = (0..7)
            .map(|i| {
                let a = i as f64 / 7.0 * std::f64::consts::TAU;
                let rr = if i % 2 == 0 { 0.27 } else { 0.19 };
                at(-0.45 + rr * a.cos(), -0.2 + rr * a.sin(), 1.0)
            })
            .collect();
        blob[2].sharp = true;
        let mut b = mk(blob, [0x22, 0xd3, 0xee], 7.0);
        b.closed = true;
        b.taper_start = 0.0;
        b.taper_end = 0.0;
        art.strokes.push(b);
        // A calligraphic stroke: thick in the middle, tapered ends.
        art.strokes.push(mk(
            vec![
                at(0.05, -0.55, 0.3),
                at(0.25, -0.2, 1.4),
                at(0.5, -0.32, 2.6),
                at(0.72, -0.05, 1.2),
                at(0.95, -0.25, 0.35),
            ],
            [0xff, 0xd8, 0x1a],
            16.0,
        ));
        // Straight great-circle segments (sharp points).
        let mut zig = mk(
            vec![
                at(-0.9, -0.62, 1.0),
                at(-0.7, -0.45, 1.0),
                at(-0.5, -0.62, 1.0),
                at(-0.3, -0.45, 1.0),
            ],
            [0xf8, 0xfa, 0xfc],
            4.0,
        );
        zig.smooth = false;
        zig.taper_start = 0.0;
        zig.taper_end = 0.0;
        art.strokes.push(zig);
        let n = self.doc.layers.iter().filter(|l| l.is_lines()).count() + 1;
        let name = trf("layer.lines_name", &[("n", &n)]);
        let Some(id) = ops::add_line_layer(&mut self.doc, &mut self.hist, &name) else {
            return;
        };
        self.line_apply(id, art);
        self.stale_thumbs.insert(id);
        match mode {
            "width" => {
                self.tool = crate::app::Tool::Line(LineTool::Thickness);
                self.lines.sel = Some(Sel {
                    layer: id,
                    stroke: 0,
                    point: Some(2),
                });
            }
            "curve" => {
                self.tool = crate::app::Tool::Line(LineTool::Curve);
                let before = self.line_art_clone(id).unwrap_or_default();
                let mut a = before.clone();
                let mut s = self.s.lines.stroke([0xa7, 0x8b, 0xfa]);
                s.width = 9.0;
                s.points = vec![at(0.2, 0.12, 1.0), at(0.45, 0.22, 1.0), at(0.62, 0.08, 1.0)];
                a.strokes.push(s);
                let si = a.strokes.len() - 1;
                self.line_apply(id, a);
                self.lines.active = Some(Active {
                    layer: id,
                    before,
                    label: "hist.line_add",
                    op: Op::Curve {
                        stroke: si,
                        dragging: false,
                    },
                });
                self.lines.sel = Some(Sel {
                    layer: id,
                    stroke: si,
                    point: Some(2),
                });
            }
            _ => {
                self.tool = crate::app::Tool::Line(LineTool::Edit);
                self.lines.sel = Some(Sel {
                    layer: id,
                    stroke: 2,
                    point: Some(2),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoraminal_core::lines::LineArt;
    use panoraminal_core::math::dir_from_lonlat;

    /// Flat "screen": 1 point = 0.1° of longitude/latitude around (180°, 0°).
    fn proj(d: Vec3, _near: Pos2) -> Option<Pos2> {
        let (lon, lat) = panoraminal_core::math::lonlat_of(d);
        Some(Pos2::new(
            ((lon.to_degrees() - 180.0) * 10.0) as f32 + 500.0,
            (-lat.to_degrees() * 10.0) as f32 + 500.0,
        ))
    }
    fn at(x: f32, y: f32) -> Vec3 {
        dir_from_lonlat(
            (((x - 500.0) / 10.0) as f64 + 180.0).to_radians(),
            (-((y - 500.0) / 10.0) as f64).to_radians(),
        )
    }
    fn art() -> LineArt {
        let mut s = LineSettings::default().stroke([0, 0, 0]);
        s.width = 2.0;
        s.points = vec![
            LinePoint::new(at(400.0, 500.0), 1.0),
            LinePoint::new(at(500.0, 450.0), 1.0),
            LinePoint::new(at(600.0, 500.0), 1.0),
        ];
        let mut s2 = s.clone();
        s2.points = vec![
            LinePoint::new(at(620.0, 500.0), 1.0),
            LinePoint::new(at(700.0, 520.0), 1.0),
        ];
        LineArt {
            strokes: vec![s, s2],
        }
    }

    #[test]
    fn hit_points_then_lines() {
        let a = art();
        let step = 0.1f64.to_radians() * 3.0;
        let pxa = 0.1f64.to_radians();
        let h = hit_test(&a, Pos2::new(503.0, 452.0), None, step, pxa, 4096, &proj);
        assert_eq!(
            h,
            Some(Hit::Point {
                stroke: 0,
                point: 1
            })
        );
        let h = hit_test(&a, Pos2::new(660.0, 510.0), None, step, pxa, 4096, &proj);
        assert!(
            matches!(
                h,
                Some(Hit::Segment {
                    stroke: 1,
                    seg: 0,
                    ..
                })
            ),
            "{h:?}"
        );
        if let Some(Hit::Segment { t, .. }) = h {
            assert!((t - 0.5).abs() < 0.1);
        }
        assert_eq!(
            hit_test(&a, Pos2::new(500.0, 700.0), None, step, pxa, 4096, &proj),
            None
        );
    }

    #[test]
    fn join_target_finds_nearby_end() {
        let mut a = art();
        // End of stroke 0 is 20 points from the start of stroke 1: too far.
        assert_eq!(join_target(&a, 0, 2, &proj, Pos2::ZERO), None);
        a.strokes[0].points[2] = LinePoint::new(at(615.0, 501.0), 1.0);
        assert_eq!(join_target(&a, 0, 2, &proj, Pos2::ZERO), Some((1, false)));
        // Inner points never join.
        assert_eq!(join_target(&a, 0, 1, &proj, Pos2::ZERO), None);
    }

    #[test]
    fn screen_polylines_cover_the_stroke() {
        let a = art();
        let pl = screen_polylines(&a.strokes[0], 0.002, Pos2::ZERO, &proj);
        assert_eq!(pl.len(), 1);
        let p = &pl[0];
        assert!((p[0] - Pos2::new(400.0, 500.0)).length() < 0.5);
        assert!((p[p.len() - 1] - Pos2::new(600.0, 500.0)).length() < 0.5);
    }

    #[test]
    fn settings_make_strokes() {
        let mut s = LineSettings {
            width: 9999.0,
            ..Default::default()
        };
        s.clamp();
        assert_eq!(s.width, lines::MAX_WIDTH);
        let st = s.stroke([1, 2, 3]);
        assert_eq!(st.color, [1, 2, 3]);
        assert!(st.points.is_empty());
    }
}
