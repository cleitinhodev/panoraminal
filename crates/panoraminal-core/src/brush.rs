//! The spherical brush engine.
//!
//! Strokes are sampled along great circles of the panorama sphere, so a stroke that
//! crosses the 0°/360° seam or passes over a pole stays continuous. Each dab is an
//! ellipse in equirectangular space whose horizontal radius is divided by `cos(lat)`
//! (capped), which makes it look round once wrapped onto the sphere ("polar
//! correction").
//!
//! Dabs accumulate into a floating-point *stroke mask*; while painting, the renderer
//! previews `layer ⊕ mask` on the GPU, and on release the mask is committed into the
//! layer pixels with tile-based undo data.

use crate::document::{Document, LayerId};
use crate::history::{Edit, Tile};
use crate::math::{angle_between, dir_to_px, slerp, Vec3};
use crate::rect::{DirtyRegion, IRect};
use crate::tip::{PreparedTip, Rng};
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;
use std::sync::Arc;

/// Maximum horizontal stretch `1/cos(lat)` applied near the poles.
pub const MAX_STRETCH: f64 = 12.0;
/// Side of an undo tile, in pixels.
pub const TILE: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// Hard, pressure-sensitive inking pen.
    Pen,
    /// Soft round brush.
    Brush,
    /// Very soft, low-flow brush that builds up.
    Airbrush,
    Eraser,
    /// Sampled tip from the brush library (Photoshop ABR, GIMP GBR/GIH, images).
    Custom,
}

impl ToolKind {
    pub const ALL: [ToolKind; 5] = [
        ToolKind::Pen,
        ToolKind::Brush,
        ToolKind::Airbrush,
        ToolKind::Eraser,
        ToolKind::Custom,
    ];
    pub fn id(self) -> &'static str {
        match self {
            ToolKind::Pen => "pen",
            ToolKind::Brush => "brush",
            ToolKind::Airbrush => "airbrush",
            ToolKind::Eraser => "eraser",
            ToolKind::Custom => "custom",
        }
    }
    pub fn is_eraser(self) -> bool {
        self == ToolKind::Eraser
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrushSettings {
    /// Diameter in document pixels (at the equator).
    pub size: f32,
    /// Maximum opacity of a stroke, 0–1.
    pub opacity: f32,
    /// Per-dab alpha, 0–1 (lower values build up).
    pub flow: f32,
    /// Edge hardness, 0 (soft) – 1 (hard).
    pub hardness: f32,
    /// Dab spacing as a fraction of the diameter.
    pub spacing: f32,
    /// Input smoothing / stabilizer, 0–1.
    pub smoothing: f32,
    pub pressure_size: bool,
    pub pressure_opacity: bool,
    /// Smallest size at zero pressure, as a fraction of `size`.
    pub min_size: f32,
    /// Keep dabs round on the sphere (stretch horizontally by 1/cos(lat)).
    pub polar_correction: bool,
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self {
            size: 40.0,
            opacity: 1.0,
            flow: 1.0,
            hardness: 0.7,
            spacing: 0.12,
            smoothing: 0.2,
            pressure_size: true,
            pressure_opacity: false,
            min_size: 0.08,
            polar_correction: true,
        }
    }
}

impl BrushSettings {
    pub fn preset(tool: ToolKind) -> Self {
        let d = Self::default();
        match tool {
            ToolKind::Pen => Self {
                size: 12.0,
                hardness: 0.95,
                spacing: 0.08,
                smoothing: 0.35,
                ..d
            },
            ToolKind::Brush => Self {
                size: 40.0,
                hardness: 0.6,
                ..d
            },
            ToolKind::Airbrush => Self {
                size: 140.0,
                hardness: 0.0,
                flow: 0.08,
                spacing: 0.06,
                pressure_size: false,
                pressure_opacity: true,
                ..d
            },
            ToolKind::Eraser => Self {
                size: 60.0,
                hardness: 0.8,
                ..d
            },
            ToolKind::Custom => Self {
                size: 80.0,
                hardness: 1.0,
                spacing: 0.25,
                smoothing: 0.15,
                ..d
            },
        }
    }
    pub fn clamp(&mut self) {
        self.size = self.size.clamp(1.0, 1000.0);
        self.opacity = self.opacity.clamp(0.01, 1.0);
        self.flow = self.flow.clamp(0.01, 1.0);
        self.hardness = self.hardness.clamp(0.0, 1.0);
        self.spacing = self.spacing.clamp(0.02, 2.0);
        self.smoothing = self.smoothing.clamp(0.0, 1.0);
        self.min_size = self.min_size.clamp(0.0, 1.0);
    }
    fn press_size(&self, p: f32) -> f32 {
        if self.pressure_size {
            self.min_size + (1.0 - self.min_size) * p
        } else {
            1.0
        }
    }
}

/// Live stroke state.
pub struct StrokeEngine {
    active: bool,
    layer: LayerId,
    tool: ToolKind,
    settings: BrushSettings,
    rgb: [u8; 3],
    mask: Vec<f32>,
    w: u32,
    h: u32,
    tiles_x: u32,
    touched: Vec<bool>,
    any_dab: bool,
    raw_last: Vec3,
    smooth_dir: Vec3,
    smooth_p: f32,
    last: Vec3,
    last_p: f32,
    gap: f64,
    tip: Option<Arc<PreparedTip>>,
    rng: Rng,
    seq: usize,
    last_dab: Option<(f64, f64)>,
    /// Seed of the jitter generator for the next stroke (strokes are reproducible).
    pub seed: u64,
    /// Regions of the mask changed since the renderer last uploaded it.
    pub mask_dirty: DirtyRegion,
    /// End point of the previous committed stroke (for Shift+click straight lines).
    pub last_end: Option<Vec3>,
}

impl Default for StrokeEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl StrokeEngine {
    pub fn new() -> Self {
        Self {
            active: false,
            layer: LayerId(0),
            tool: ToolKind::Brush,
            settings: BrushSettings::default(),
            rgb: [0, 0, 0],
            mask: Vec::new(),
            w: 0,
            h: 0,
            tiles_x: 0,
            touched: Vec::new(),
            any_dab: false,
            raw_last: Vec3::default(),
            smooth_dir: Vec3::default(),
            smooth_p: 1.0,
            last: Vec3::default(),
            last_p: 1.0,
            gap: 0.0,
            tip: None,
            rng: Rng::new(1),
            seq: 0,
            last_dab: None,
            seed: 1,
            mask_dirty: DirtyRegion::default(),
            last_end: None,
        }
    }
    pub fn is_active(&self) -> bool {
        self.active
    }
    pub fn layer(&self) -> LayerId {
        self.layer
    }
    pub fn tool(&self) -> ToolKind {
        self.tool
    }
    pub fn opacity(&self) -> f32 {
        self.settings.opacity
    }
    pub fn color(&self) -> [u8; 3] {
        self.rgb
    }
    /// The stroke coverage mask (`w*h` floats), valid while a stroke is active.
    pub fn mask(&self) -> &[f32] {
        &self.mask
    }
    pub fn mask_size(&self) -> (u32, u32) {
        (self.w, self.h)
    }
    /// Forgets document-specific state (call when the document is replaced).
    pub fn reset(&mut self) {
        let tip = self.tip.take();
        *self = Self::new();
        self.tip = tip;
    }
    /// Sets the sampled tip used by [`ToolKind::Custom`] strokes (`None` = round dab).
    pub fn set_tip(&mut self, tip: Option<Arc<PreparedTip>>) {
        self.tip = tip;
    }
    pub fn tip(&self) -> Option<&Arc<PreparedTip>> {
        self.tip.as_ref()
    }

    fn ensure_buffers(&mut self, w: u32, h: u32) {
        if self.w != w || self.h != h || self.mask.len() != (w * h) as usize {
            self.w = w;
            self.h = h;
            self.mask = vec![0.0; (w * h) as usize];
            self.tiles_x = w.div_ceil(TILE);
            self.touched = vec![false; (self.tiles_x * h.div_ceil(TILE)) as usize];
            self.mask_dirty.mark_full();
        }
    }

    /// Starts a stroke on the document's active layer. Returns `false` if the layer
    /// is missing or hidden.
    pub fn begin(
        &mut self,
        doc: &Document,
        tool: ToolKind,
        settings: BrushSettings,
        rgb: [u8; 3],
        dir: Vec3,
        pressure: f32,
    ) -> bool {
        let Some(layer) = doc.active_layer() else {
            return false;
        };
        if !layer.visible {
            return false;
        }
        self.ensure_buffers(doc.width, doc.height);
        let mut settings = settings;
        settings.clamp();
        self.active = true;
        self.layer = layer.id;
        self.tool = tool;
        self.settings = settings;
        self.rgb = rgb;
        self.any_dab = false;
        let p = pressure.clamp(0.0, 1.0);
        self.raw_last = dir;
        self.smooth_dir = dir;
        self.smooth_p = p;
        self.last = dir;
        self.last_p = p;
        self.rng = Rng::new(self.seed);
        self.seed = self.seed.wrapping_add(0x5851_F42D);
        self.seq = 0;
        self.last_dab = None;
        self.stamp(dir, p);
        self.gap = self.gap_for(p);
        true
    }

    /// Adds an input sample. `direct` bypasses smoothing (used for the final sample).
    pub fn add_point(&mut self, dir: Vec3, pressure: f32, direct: bool) {
        if !self.active {
            return;
        }
        let p = pressure.clamp(0.0, 1.0);
        self.raw_last = dir;
        let s = if direct {
            0.0
        } else {
            self.settings.smoothing as f64 * 0.92
        };
        if s > 0.0 {
            self.smooth_dir = self.smooth_dir.lerp(dir, 1.0 - s).normalize();
            self.smooth_p += (p - self.smooth_p) * (1.0 - s as f32);
        } else {
            self.smooth_dir = dir;
            self.smooth_p = p;
        }
        let (a, pa, b, pb) = (self.last, self.last_p, self.smooth_dir, self.smooth_p);
        self.segment(a, pa, b, pb);
        self.last = b;
        self.last_p = pb;
    }

    /// Straight great-circle line from `a` to `b` (Shift+click).
    pub fn line(
        &mut self,
        doc: &Document,
        tool: ToolKind,
        settings: BrushSettings,
        rgb: [u8; 3],
        a: Vec3,
        b: Vec3,
        pressure: f32,
    ) -> bool {
        if !self.begin(doc, tool, settings, rgb, a, pressure) {
            return false;
        }
        self.settings.smoothing = 0.0;
        self.add_point(b, pressure, true);
        true
    }

    fn gap_for(&self, p: f32) -> f64 {
        let dia = self.settings.size as f64 * self.settings.press_size(p) as f64;
        let px_ang = TAU / self.w as f64;
        (dia * px_ang * self.settings.spacing as f64).max(px_ang * 0.5)
    }

    fn segment(&mut self, a: Vec3, pa: f32, b: Vec3, pb: f32) {
        let ang = angle_between(a, b);
        if ang < 1e-12 {
            return;
        }
        let mut pos = self.gap;
        let mut guard = 0u32;
        while pos <= ang && guard < 200_000 {
            let t = pos / ang;
            let d = slerp(a, b, t);
            let p = pa + (pb - pa) * t as f32;
            self.stamp(d, p);
            pos += self.gap_for(p);
            guard += 1;
        }
        self.gap = pos - ang;
    }

    /// Rasterizes one elliptical dab into the mask (wrapping horizontally at the seam).
    fn stamp(&mut self, dir: Vec3, pressure: f32) {
        if self.tool == ToolKind::Custom {
            if let Some(tip) = self.tip.clone() {
                self.stamp_tip(&tip, dir, pressure);
                return;
            }
        }
        let (w, h) = (self.w, self.h);
        let s = &self.settings;
        let pp = dir_to_px(dir, w, h);
        let dia = (s.size * s.press_size(pressure)).max(0.5) as f64;
        let r = dia / 2.0;
        let ry = r * (2.0 * h as f64 / w as f64);
        let mut rx = r;
        if s.polar_correction {
            rx = r / pp.lat.cos().max(1.0 / MAX_STRETCH);
        }
        rx = rx.min(w as f64 / 2.0 - 1.0).max(0.5);
        let ry = ry.max(0.5);
        let mut alpha = s.flow;
        if s.pressure_opacity {
            alpha *= 0.05 + 0.95 * pressure;
        }
        if alpha <= 0.0 {
            return;
        }
        let hard = s.hardness;
        // AA width in normalized units (one pixel)
        let aa = 1.0 / rx.min(ry).max(0.5);
        let y0 = (pp.y - ry - 1.0).floor().max(0.0) as i64;
        let y1 = (pp.y + ry + 1.0).ceil().min(h as f64) as i64;
        let x0 = (pp.x - rx - 1.0).floor() as i64;
        let x1 = (pp.x + rx + 1.0).ceil() as i64;
        if y1 <= y0 {
            return;
        }
        // Tiny brushes: scale coverage by area so 1px lines aren't too heavy.
        let small = ((rx * ry) / 0.25).min(1.0) as f32;
        for y in y0..y1 {
            let dy = (y as f64 + 0.5 - pp.y) / ry;
            if dy.abs() > 1.0 + aa {
                continue;
            }
            let row = (y as u32 * w) as usize;
            for x in x0..x1 {
                let dx = (x as f64 + 0.5 - pp.x) / rx;
                let d = (dx * dx + dy * dy).sqrt();
                let cov = if hard >= 0.98 {
                    ((1.0 - d) / aa + 0.5).clamp(0.0, 1.0) as f32
                } else {
                    let hh = hard as f64;
                    if d >= 1.0 {
                        0.0
                    } else if d <= hh {
                        1.0
                    } else {
                        let t = (d - hh) / (1.0 - hh);
                        (1.0 - t * t * (3.0 - 2.0 * t)) as f32
                    }
                };
                if cov <= 0.0 {
                    continue;
                }
                let a = cov * alpha * small;
                let xi = x.rem_euclid(w as i64) as u32;
                let m = &mut self.mask[row + xi as usize];
                *m += a * (1.0 - *m);
            }
        }
        self.mark_box(x0, x1, y0, y1);
        self.any_dab = true;
    }

    /// Marks mask rows `y0..y1` × columns `x0..x1` (may extend past the seam) dirty.
    fn mark_box(&mut self, x0: i64, x1: i64, y0: i64, y1: i64) {
        let w = self.w;
        let yr = (y0.max(0) as u32, (y1.min(self.h as i64)) as u32);
        if yr.1 <= yr.0 {
            return;
        }
        let mut mark = |xa: i64, xb: i64| {
            let xa = xa.clamp(0, w as i64) as u32;
            let xb = xb.clamp(0, w as i64) as u32;
            if xb <= xa {
                return;
            }
            self.mask_dirty
                .add(IRect::new(xa, yr.0, xb - xa, yr.1 - yr.0));
            for ty in yr.0 / TILE..=(yr.1 - 1) / TILE {
                for tx in xa / TILE..=(xb - 1) / TILE {
                    self.touched[(ty * self.tiles_x + tx) as usize] = true;
                }
            }
        };
        if x1 - x0 >= w as i64 {
            mark(0, w as i64);
        } else {
            mark(x0, x1);
            if x0 < 0 {
                mark(x0 + w as i64, w as i64);
            }
            if x1 > w as i64 {
                mark(0, x1 - w as i64);
            }
        }
    }

    /// Stamps a sampled tip (with its shape dynamics) centred on `dir`.
    ///
    /// The tip is rasterized in the local tangent frame: one tip "radius" spans `rx`
    /// pixels horizontally (`r / cos(lat)` with polar correction) and `ry` vertically,
    /// so stamps keep their shape on the sphere and wrap across the seam.
    fn stamp_tip(&mut self, tip: &PreparedTip, dir: Vec3, pressure: f32) {
        let (w, h) = (self.w, self.h);
        let s = self.settings;
        let dy = &tip.dynamics;
        let pp = dir_to_px(dir, w, h);
        let base_dia = (s.size * s.press_size(pressure)).max(0.5) as f64;
        let stretch = if s.polar_correction {
            1.0 / pp.lat.cos().max(1.0 / MAX_STRETCH)
        } else {
            1.0
        };
        let ky = 2.0 * h as f64 / w as f64;
        // Stroke direction in the tangent frame (east, north), unit length.
        let mut tdir = (1.0f64, 0.0f64);
        if let Some((lx, ly)) = self.last_dab {
            let mut ddx = pp.x - lx;
            let wf = w as f64;
            if ddx > wf / 2.0 {
                ddx -= wf;
            } else if ddx < -wf / 2.0 {
                ddx += wf;
            }
            let (ex, ny) = (ddx / stretch, -(pp.y - ly) / ky);
            let l = (ex * ex + ny * ny).sqrt();
            if l > 1e-9 {
                tdir = (ex / l, ny / l);
            }
        }
        self.last_dab = Some((pp.x, pp.y));
        let mut count = dy.count.max(1) as f32;
        if dy.count_jitter > 0.0 {
            count = (count * (1.0 - dy.count_jitter * self.rng.next_f32())).max(1.0);
        }
        let n_var = tip.variants.len();
        for _ in 0..count.round() as u32 {
            let rng = &mut self.rng;
            // size / angle / roundness jitter
            let mut size_f = 1.0f64;
            if dy.size_jitter > 0.0 {
                size_f = (1.0 - dy.size_jitter as f64 * rng.next_f32() as f64)
                    .max(dy.min_diameter as f64);
            }
            let dia = (base_dia * size_f).max(0.5);
            let r = dia / 2.0;
            let mut ang = (dy.angle as f64).to_radians();
            if dy.follow_direction {
                ang += tdir.1.atan2(tdir.0);
            }
            if dy.angle_jitter > 0.0 {
                ang += dy.angle_jitter as f64 * std::f64::consts::PI * rng.signed() as f64;
            }
            let mut round = dy.roundness as f64;
            if dy.roundness_jitter > 0.0 {
                round *= (1.0 - dy.roundness_jitter as f64 * rng.next_f32() as f64)
                    .max(dy.min_roundness as f64);
            }
            let round = round.clamp(0.01, 1.0);
            let flip_x = dy.flip_x ^ (dy.flip_x_jitter && rng.next_f32() < 0.5);
            let flip_y = dy.flip_y ^ (dy.flip_y_jitter && rng.next_f32() < 0.5);
            let mut alpha = s.flow;
            if dy.flow_jitter > 0.0 {
                alpha *= 1.0 - dy.flow_jitter * rng.next_f32();
            }
            if dy.opacity_jitter > 0.0 {
                alpha *= 1.0 - dy.opacity_jitter * rng.next_f32();
            }
            if s.pressure_opacity {
                alpha *= 0.05 + 0.95 * pressure;
            }
            // scattering (perpendicular to the stroke; optionally along it too)
            let (mut cx, mut cy) = (pp.x, pp.y);
            if dy.scatter > 0.0 {
                let perp = (-tdir.1, tdir.0);
                let a = dy.scatter as f64 * dia * 0.5 * rng.signed() as f64;
                let (mut ex, mut ny) = (perp.0 * a, perp.1 * a);
                if dy.scatter_both_axes {
                    let b = dy.scatter as f64 * dia * 0.5 * rng.signed() as f64;
                    ex += tdir.0 * b;
                    ny += tdir.1 * b;
                }
                cx += ex * stretch;
                cy = (cy - ny * ky).clamp(0.0, h as f64 - 0.001);
            }
            let variant = match dy.selection {
                crate::tip::TipSelection::Sequential => self.seq,
                crate::tip::TipSelection::Random => (rng.next_f32() * n_var as f32) as usize,
            } % n_var;
            self.seq = self.seq.wrapping_add(1);
            if alpha <= 0.0 {
                continue;
            }
            let img = tip.level(variant, dia);
            // Map the tip's longest side onto the dab diameter.
            let m = img.max_side() as f64;
            let ext = (img.width as f64).hypot(img.height as f64) / m;
            let rx = (r * stretch).max(0.5);
            let ry = (r * ky).max(0.5);
            let ex_x = (rx * ext).min(w as f64 / 2.0 - 1.0);
            let ex_y = ry * ext;
            let y0 = (cy - ex_y - 1.0).floor().max(0.0) as i64;
            let y1 = (cy + ex_y + 1.0).ceil().min(h as f64) as i64;
            let x0 = (cx - ex_x - 1.0).floor() as i64;
            let x1 = (cx + ex_x + 1.0).ceil() as i64;
            if y1 <= y0 {
                continue;
            }
            let (sa, ca) = ang.sin_cos();
            let half_m = m * 0.5;
            let (tcx, tcy) = (img.width as f64 * 0.5, img.height as f64 * 0.5);
            let small = ((rx * ry) / 0.25).min(1.0) as f32;
            for y in y0..y1 {
                let ny = -(y as f64 + 0.5 - cy) / ry;
                let row = (y as u32 * w) as usize;
                for x in x0..x1 {
                    let ex = (x as f64 + 0.5 - cx) / rx;
                    // rotate into the tip frame, undo roundness
                    let mut a = ex * ca + ny * sa;
                    let mut b = (-ex * sa + ny * ca) / round;
                    if flip_x {
                        a = -a;
                    }
                    if flip_y {
                        b = -b;
                    }
                    let tx = tcx + a * half_m;
                    let ty = tcy - b * half_m;
                    if tx < -1.0 || ty < -1.0 || tx > img.width as f64 + 1.0 {
                        continue;
                    }
                    if ty > img.height as f64 + 1.0 {
                        continue;
                    }
                    let cov = img.sample(tx as f32, ty as f32);
                    if cov <= 0.0 {
                        continue;
                    }
                    let av = (cov * alpha * small).min(1.0);
                    let xi = x.rem_euclid(w as i64) as usize;
                    let mm = &mut self.mask[row + xi];
                    *mm += av * (1.0 - *mm);
                }
            }
            self.mark_box(x0, x1, y0, y1);
            self.any_dab = true;
        }
    }

    /// Aborts the stroke, discarding the mask.
    pub fn cancel(&mut self) {
        if !self.active {
            return;
        }
        self.clear_touched();
        self.active = false;
    }

    fn clear_touched(&mut self) {
        let (w, h) = (self.w, self.h);
        for (ti, t) in self.touched.iter_mut().enumerate() {
            if !*t {
                continue;
            }
            *t = false;
            let tx = ti as u32 % self.tiles_x * TILE;
            let ty = ti as u32 / self.tiles_x * TILE;
            let tw = TILE.min(w - tx);
            let th = TILE.min(h - ty);
            for y in ty..ty + th {
                let i = (y * w + tx) as usize;
                self.mask[i..i + tw as usize].fill(0.0);
            }
            self.mask_dirty.add(IRect::new(tx, ty, tw, th));
        }
    }

    /// Commits the stroke into its layer. Returns the undo edit (or `None` if nothing changed).
    pub fn end(&mut self, doc: &mut Document) -> Option<Edit> {
        if !self.active {
            return None;
        }
        self.active = false;
        self.last_end = Some(self.raw_last);
        if !self.any_dab {
            self.clear_touched();
            return None;
        }
        let (w, h) = (self.w, self.h);
        if doc.width != w || doc.height != h {
            self.clear_touched();
            return None;
        }
        let id = self.layer;
        let eraser = self.tool.is_eraser();
        let opacity = self.settings.opacity;
        let [cr, cg, cb] = self.rgb.map(|c| c as f32 / 255.0);
        let mut tiles = Vec::new();
        let mut dirty = Vec::new();
        {
            let Some(layer) = doc.layer_mut(id) else {
                self.clear_touched();
                return None;
            };
            let lock = layer.alpha_locked;
            for ti in 0..self.touched.len() {
                if !self.touched[ti] {
                    continue;
                }
                let tx = ti as u32 % self.tiles_x * TILE;
                let ty = ti as u32 / self.tiles_x * TILE;
                let tw = TILE.min(w - tx);
                let th = TILE.min(h - ty);
                let rect = IRect::new(tx, ty, tw, th);
                tiles.push(Tile::capture(&layer.pixels, w, rect));
                for y in ty..ty + th {
                    for x in tx..tx + tw {
                        let mi = (y * w + x) as usize;
                        let m = self.mask[mi];
                        if m <= 0.0 {
                            continue;
                        }
                        self.mask[mi] = 0.0;
                        let a = (m * opacity).min(1.0);
                        let p = &mut layer.pixels[mi * 4..mi * 4 + 4];
                        apply_paint(p, a, [cr, cg, cb], eraser, lock);
                    }
                }
                dirty.push(rect);
            }
            self.touched.fill(false);
        }
        for r in &dirty {
            doc.mark_dirty(id, *r);
            self.mask_dirty.add(*r);
        }
        Some(Edit::Pixels { layer: id, tiles })
    }
}

/// Applies a paint (or erase) contribution of alpha `a` to one straight-RGBA8 pixel.
/// Must match the preview logic in `composite.wgsl`.
#[inline]
pub fn apply_paint(p: &mut [u8], a: f32, rgb: [f32; 3], eraser: bool, alpha_lock: bool) {
    let da = p[3] as f32 / 255.0;
    if eraser {
        if alpha_lock {
            return;
        }
        let na = da * (1.0 - a);
        p[3] = (na * 255.0 + 0.5) as u8;
        return;
    }
    if alpha_lock {
        for i in 0..3 {
            let c = p[i] as f32 / 255.0;
            p[i] = ((c + (rgb[i] - c) * a) * 255.0 + 0.5) as u8;
        }
        return;
    }
    let oa = a + da * (1.0 - a);
    if oa <= 0.0 {
        return;
    }
    for i in 0..3 {
        let c = p[i] as f32 / 255.0;
        let o = (rgb[i] * a + c * da * (1.0 - a)) / oa;
        p[i] = (o * 255.0 + 0.5) as u8;
    }
    p[3] = (oa * 255.0 + 0.5) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Background;
    use crate::math::{dir_from_lonlat, px_to_dir};
    use std::f64::consts::PI;

    fn doc() -> Document {
        Document::new_blank(2048, 1024, Background::White, "bg", "l1")
    }
    fn bbox(d: &Document) -> (u32, u32, u32, u32, usize) {
        let l = d.active_layer().unwrap();
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (u32::MAX, u32::MAX, 0, 0, 0);
        for y in 0..d.height {
            for x in 0..d.width {
                if l.pixels[((y * d.width + x) * 4 + 3) as usize] > 8 {
                    n += 1;
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    y0 = y0.min(y);
                    y1 = y1.max(y);
                }
            }
        }
        (x0, y0, x1, y1, n)
    }
    fn hard(size: f32, polar: bool) -> BrushSettings {
        BrushSettings {
            size,
            hardness: 1.0,
            smoothing: 0.0,
            pressure_size: false,
            polar_correction: polar,
            ..Default::default()
        }
    }

    #[test]
    fn dab_round_at_equator_and_stretched_at_60() {
        for (lat, polar, ratio) in [(0.0, true, 1.0), (60.0f64, true, 2.0), (60.0, false, 1.0)] {
            let mut d = doc();
            let mut s = StrokeEngine::new();
            assert!(s.begin(
                &d,
                ToolKind::Brush,
                hard(60.0, polar),
                [255, 0, 0],
                dir_from_lonlat(PI, lat.to_radians()),
                1.0
            ));
            s.end(&mut d).unwrap();
            let (x0, y0, x1, y1, _) = bbox(&d);
            let r = (x1 - x0 + 1) as f64 / (y1 - y0 + 1) as f64;
            assert!(
                (r - ratio).abs() < 0.12,
                "lat {lat} polar {polar}: ratio {r}"
            );
        }
    }

    #[test]
    fn stroke_across_seam_has_no_spurious_line() {
        let mut d = doc();
        let mut s = StrokeEngine::new();
        let a = px_to_dir(2000.0, 512.0, 2048, 1024);
        let b = px_to_dir(48.0, 512.0, 2048, 1024);
        s.begin(&d, ToolKind::Brush, hard(20.0, true), [0, 0, 255], a, 1.0);
        s.add_point(b, 1.0, true);
        s.end(&mut d).unwrap();
        let l = d.active_layer().unwrap();
        let mid: usize = (900..1100)
            .map(|x| l.pixels[((512 * 2048 + x) * 4 + 3) as usize] as usize)
            .sum();
        assert_eq!(mid, 0, "stroke must go the short way across the seam");
        assert!(l.pixels[((512 * 2048) * 4 + 3) as usize] > 200);
        assert!(l.pixels[((512 * 2048 + 2047) * 4 + 3) as usize] > 200);
    }

    #[test]
    fn undo_tiles_restore_exactly() {
        let mut d = doc();
        let before = d.active_layer().unwrap().pixels.clone();
        let mut s = StrokeEngine::new();
        s.begin(
            &d,
            ToolKind::Brush,
            BrushSettings::default(),
            [10, 200, 30],
            dir_from_lonlat(PI, 0.2),
            0.7,
        );
        s.add_point(dir_from_lonlat(PI + 0.5, -0.1), 0.9, false);
        let mut e = s.end(&mut d).unwrap();
        assert_ne!(d.active_layer().unwrap().pixels, before);
        e.swap(&mut d);
        assert_eq!(d.active_layer().unwrap().pixels, before);
        assert!(
            s.mask().iter().all(|m| *m == 0.0),
            "mask cleared after commit"
        );
    }

    #[test]
    fn pressure_changes_width() {
        let mut d = doc();
        let mut s = StrokeEngine::new();
        let mut st = hard(80.0, true);
        st.pressure_size = true;
        s.begin(
            &d,
            ToolKind::Brush,
            st,
            [0, 0, 0],
            px_to_dir(800.0, 512.0, 2048, 1024),
            0.1,
        );
        for i in 1..=40 {
            s.add_point(
                px_to_dir(800.0 + i as f64 * 10.0, 512.0, 2048, 1024),
                0.1 + 0.9 * i as f32 / 40.0,
                true,
            );
        }
        s.end(&mut d);
        let l = d.active_layer().unwrap();
        let col = |x: u32| {
            (0..1024)
                .filter(|y| l.pixels[((y * 2048 + x) * 4 + 3) as usize] > 128)
                .count()
        };
        assert!(col(1150) > col(830) * 2, "{} vs {}", col(1150), col(830));
    }

    #[test]
    fn eraser_and_alpha_lock() {
        let mut p = [100, 100, 100, 255];
        apply_paint(&mut p, 1.0, [1.0, 0.0, 0.0], true, false);
        assert_eq!(p[3], 0);
        let mut p = [0, 0, 0, 0];
        apply_paint(&mut p, 1.0, [1.0, 0.0, 0.0], false, true);
        assert_eq!(p[3], 0, "alpha lock keeps transparent pixels transparent");
        let mut p = [0, 0, 0, 0];
        apply_paint(&mut p, 0.5, [1.0, 0.0, 0.0], false, false);
        assert_eq!(p, [255, 0, 0, 128]);
    }

    #[test]
    fn hidden_layer_refuses_stroke() {
        let mut d = doc();
        let id = d.active;
        d.layer_mut(id).unwrap().visible = false;
        let mut s = StrokeEngine::new();
        assert!(!s.begin(
            &d,
            ToolKind::Brush,
            BrushSettings::default(),
            [0, 0, 0],
            dir_from_lonlat(1.0, 0.0),
            1.0
        ));
    }

    #[test]
    fn pole_stroke_capped() {
        let mut d = doc();
        let mut s = StrokeEngine::new();
        s.begin(
            &d,
            ToolKind::Brush,
            hard(58.0, true),
            [0, 0, 0],
            dir_from_lonlat(PI, 88f64.to_radians()),
            1.0,
        );
        s.end(&mut d);
        let (x0, _, x1, _, _) = bbox(&d);
        assert!(x1 - x0 <= (12.3 * 58.0) as u32);
    }

    // ---- custom (sampled) tips -------------------------------------------------

    fn bar_tip(dynamics: crate::tip::TipDynamics) -> std::sync::Arc<PreparedTip> {
        // 64×8 horizontal bar
        PreparedTip::new(
            &[crate::tip::TipImage::new(64, 8, vec![255; 512])],
            &dynamics,
        )
    }
    fn custom_dab(tip: std::sync::Arc<PreparedTip>, lon: f64, lat: f64, polar: bool) -> Document {
        let mut d = doc();
        let mut s = StrokeEngine::new();
        s.set_tip(Some(tip));
        let mut b = hard(64.0, polar);
        b.spacing = 0.25;
        assert!(s.begin(
            &d,
            ToolKind::Custom,
            b,
            [255, 0, 0],
            dir_from_lonlat(lon, lat),
            1.0
        ));
        s.end(&mut d).unwrap();
        d
    }

    #[test]
    fn custom_tip_shape_and_angle() {
        let d = custom_dab(bar_tip(Default::default()), PI, 0.0, true);
        let (x0, y0, x1, y1, _) = bbox(&d);
        let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
        assert!((62..=67).contains(&bw), "bar width {bw}");
        assert!((7..=11).contains(&bh), "bar height {bh}");
        // rotated by 90° the bar stands upright
        let dy = crate::tip::TipDynamics {
            angle: 90.0,
            ..Default::default()
        };
        let d = custom_dab(bar_tip(dy), PI, 0.0, true);
        let (x0, y0, x1, y1, _) = bbox(&d);
        assert!((x1 - x0 + 1) < 12 && (y1 - y0 + 1) > 60);
        // roundness squashes the tip's height
        let dy = crate::tip::TipDynamics {
            roundness: 0.5,
            ..Default::default()
        };
        let sq = PreparedTip::new(&[crate::tip::TipImage::new(32, 32, vec![255; 1024])], &dy);
        let d = custom_dab(sq, PI, 0.0, true);
        let (x0, y0, x1, y1, _) = bbox(&d);
        let r = (x1 - x0 + 1) as f64 / (y1 - y0 + 1) as f64;
        assert!((r - 2.0).abs() < 0.2, "roundness ratio {r}");
    }

    #[test]
    fn custom_tip_polar_correction() {
        let eq = custom_dab(bar_tip(Default::default()), PI, 0.0, true);
        let hi = custom_dab(bar_tip(Default::default()), PI, 60f64.to_radians(), true);
        let w_eq = {
            let b = bbox(&eq);
            b.2 - b.0 + 1
        };
        let (x0, y0, x1, y1, _) = bbox(&hi);
        let w_hi = x1 - x0 + 1;
        let r = w_hi as f64 / w_eq as f64;
        assert!((r - 2.0).abs() < 0.1, "1/cos(60°) stretch, got {r}");
        assert!((y1 - y0 + 1) < 12, "height unchanged near the pole");
    }

    #[test]
    fn custom_tip_wraps_across_the_seam() {
        let d = custom_dab(bar_tip(Default::default()), 0.0, 0.0, true);
        let l = d.active_layer().unwrap();
        let row = 512 * 2048;
        assert!(l.pixels[(row * 4 + 3) as usize] > 200);
        assert!(l.pixels[((row + 2047) * 4 + 3) as usize] > 200);
        assert!(l.pixels[((row + 20) * 4 + 3) as usize] > 200);
        assert!(l.pixels[((row + 2030) * 4 + 3) as usize] > 200);
        assert_eq!(l.pixels[((row + 1024) * 4 + 3) as usize], 0);
    }

    #[test]
    fn custom_tip_jitter_is_deterministic_and_scatter_spreads() {
        let dy = crate::tip::TipDynamics {
            scatter: 3.0,
            size_jitter: 0.5,
            angle_jitter: 1.0,
            count: 3,
            ..Default::default()
        };
        let run = |seed: u64| {
            let mut d = doc();
            let mut s = StrokeEngine::new();
            s.seed = seed;
            s.set_tip(Some(PreparedTip::new(
                &[crate::tip::TipImage::ellipse(16, 1.0, 1.0)],
                &dy,
            )));
            let b = BrushSettings {
                spacing: 0.5,
                ..hard(16.0, true)
            };
            let a = px_to_dir(800.0, 512.0, 2048, 1024);
            s.begin(&d, ToolKind::Custom, b, [0, 0, 0], a, 1.0);
            s.add_point(px_to_dir(1200.0, 512.0, 2048, 1024), 1.0, true);
            s.end(&mut d).unwrap();
            d.active_layer().unwrap().pixels.clone()
        };
        let a = run(42);
        assert_eq!(a, run(42), "same seed → same stroke");
        assert_ne!(a, run(43));
        // scattering moves dabs away from the stroke line (y = 512) by up to 3×16/2 px
        let mut far = 0;
        for y in (512 - 30)..(512 + 30) {
            if (y - 512i32).abs() < 10 {
                continue;
            }
            for x in 800..1200 {
                if a[((y * 2048 + x) * 4 + 3) as usize] > 0 {
                    far += 1;
                }
            }
        }
        assert!(far > 50, "scatter must spread paint off the line ({far})");
    }

    #[test]
    fn custom_tip_spacing_controls_dab_count() {
        // A tiny dot tip; with spacing 2.0 (200 %) dabs are separated by gaps.
        let dot = PreparedTip::new(
            &[crate::tip::TipImage::ellipse(8, 1.0, 1.0)],
            &Default::default(),
        );
        let mut d = doc();
        let mut s = StrokeEngine::new();
        s.set_tip(Some(dot));
        let b = BrushSettings {
            spacing: 2.0,
            ..hard(8.0, true)
        };
        s.begin(
            &d,
            ToolKind::Custom,
            b,
            [0, 0, 0],
            px_to_dir(1000.0, 512.0, 2048, 1024),
            1.0,
        );
        s.add_point(px_to_dir(1160.0, 512.0, 2048, 1024), 1.0, true);
        s.end(&mut d).unwrap();
        let l = d.active_layer().unwrap();
        let mut runs = 0;
        let mut inside = false;
        for x in 990..1175 {
            let on = l.pixels[((512 * 2048 + x) * 4 + 3) as usize] > 100;
            if on && !inside {
                runs += 1;
            }
            inside = on;
        }
        // 160 px / (8 px × 2.0) = 10 gaps → 11 dabs
        assert!((10..=12).contains(&runs), "dabs: {runs}");
    }
}
