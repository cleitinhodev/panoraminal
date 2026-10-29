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
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

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
}

impl ToolKind {
    pub const ALL: [ToolKind; 4] = [
        ToolKind::Pen,
        ToolKind::Brush,
        ToolKind::Airbrush,
        ToolKind::Eraser,
    ];
    pub fn id(self) -> &'static str {
        match self {
            ToolKind::Pen => "pen",
            ToolKind::Brush => "brush",
            ToolKind::Airbrush => "airbrush",
            ToolKind::Eraser => "eraser",
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
        *self = Self::new();
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
        // bookkeeping: dirty rects + touched tiles (split at the seam)
        let yr = (y0 as u32, y1 as u32);
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
        self.any_dab = true;
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
}
