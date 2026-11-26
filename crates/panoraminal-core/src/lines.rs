//! Vector line layers ("linework").
//!
//! A line layer keeps a list of editable strokes. Every control point lives on the
//! panorama sphere (longitude/latitude, in degrees), so a stroke is independent of the
//! document resolution, wraps across the 0°/360° seam and passes over the poles like
//! any other direction.
//!
//! * Curves go through their control points as a **centripetal Catmull-Rom spline**
//!   evaluated in 3-D and projected back onto the unit sphere. A segment whose two
//!   ends are *sharp* points (or any segment of a non-smooth stroke) is a
//!   **great-circle arc**, which is exactly what looks like a straight line from the
//!   centre of the panorama.
//! * The width is an angle: `width` is the diameter in document pixels at the equator,
//!   multiplied by the per-point width (`w`, the "pressure") and the start/end tapers.
//!   Because the rasterizer measures true angular distances on the sphere, a line keeps
//!   the same thickness everywhere, near the poles and across the seam included.
//! * The layer pixels are a raster cache: `pixels == render(lines)` at all times. Old
//!   Panoraminal versions simply see that cache as a normal layer.

use crate::document::{Document, LayerId};
use crate::math::{angle_between, dir_from_lonlat, lonlat_of, slerp, Vec3};
use crate::rect::IRect;
use serde::{Deserialize, Serialize};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// Limits (also applied when loading untrusted files).
pub const MAX_STROKES: usize = 20_000;
pub const MAX_POINTS: usize = 8_192;
pub const MIN_WIDTH: f32 = 0.5;
pub const MAX_WIDTH: f32 = 400.0;
pub const MIN_POINT_WIDTH: f32 = 0.05;
pub const MAX_POINT_WIDTH: f32 = 4.0;

fn one() -> f32 {
    1.0
}
fn is_one(v: &f32) -> bool {
    (*v - 1.0).abs() < 1e-6
}
fn is_false(b: &bool) -> bool {
    !*b
}

/// One control point of a stroke.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinePoint {
    /// Longitude in degrees, `[0, 360)`.
    pub lon: f64,
    /// Latitude in degrees, `[-90, 90]`.
    pub lat: f64,
    /// Width multiplier at this point ("pressure"), 0.05–4.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub w: f32,
    /// Corner point: the curve does not stay smooth through it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub sharp: bool,
}

impl LinePoint {
    pub fn new(d: Vec3, w: f32) -> Self {
        let mut p = Self {
            lon: 0.0,
            lat: 0.0,
            w,
            sharp: false,
        };
        p.set_dir(d);
        p
    }
    pub fn dir(&self) -> Vec3 {
        dir_from_lonlat(self.lon.to_radians(), self.lat.to_radians())
    }
    pub fn set_dir(&mut self, d: Vec3) {
        let (lon, lat) = lonlat_of(d.normalize());
        self.lon = lon.to_degrees();
        self.lat = lat.to_degrees();
    }
}

/// One editable stroke.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LineStroke {
    pub points: Vec<LinePoint>,
    pub color: [u8; 3],
    /// 0–1.
    pub opacity: f32,
    /// Diameter in document pixels at the equator.
    pub width: f32,
    /// Fraction of the length (0–1) over which the start / end narrows to a point.
    pub taper_start: f32,
    pub taper_end: f32,
    /// Curve through the points (`true`) or straight great-circle segments.
    pub smooth: bool,
    /// The last point connects back to the first.
    pub closed: bool,
}

impl Default for LineStroke {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            color: [0, 0, 0],
            opacity: 1.0,
            width: 6.0,
            taper_start: 0.0,
            taper_end: 0.0,
            smooth: true,
            closed: false,
        }
    }
}

/// The vector content of a line layer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LineArt {
    #[serde(default)]
    pub strokes: Vec<LineStroke>,
}

/// A point of the tessellated centre line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub dir: Vec3,
    /// Width multiplier interpolated between control points.
    pub w: f32,
    /// Arc length from the start (radians).
    pub s: f64,
}

/// A rasterization sample: direction, latitude/longitude and angular radius.
#[derive(Clone, Copy, Debug)]
struct RSample {
    dir: Vec3,
    lon: f64,
    lat: f64,
    r: f64,
}

/// Angular size of one document pixel (the smaller of both axes).
pub fn pixel_angle(w: u32, h: u32) -> f64 {
    (TAU / w as f64).min(PI / h as f64)
}

fn ease(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * (2.0 - x)
}

/// Centripetal Catmull-Rom between `p1` and `p2` in 3-D, projected onto the sphere.
pub fn catmull_rom(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, u: f64) -> Vec3 {
    let d12 = (p2 - p1).len();
    if d12 < 1e-12 {
        return p1;
    }
    // Missing / coincident neighbours: reflect, which gives a straight tangent.
    let p0 = if (p1 - p0).len() < 1e-9 {
        p1 * 2.0 - p2
    } else {
        p0
    };
    let p3 = if (p3 - p2).len() < 1e-9 {
        p2 * 2.0 - p1
    } else {
        p3
    };
    let knot = |a: Vec3, b: Vec3| (b - a).len().sqrt().max(1e-6);
    let t0 = 0.0;
    let t1 = t0 + knot(p0, p1);
    let t2 = t1 + knot(p1, p2);
    let t3 = t2 + knot(p2, p3);
    let t = t1 + u * (t2 - t1);
    let mix = |a: Vec3, b: Vec3, ta: f64, tb: f64| {
        a * ((tb - t) / (tb - ta)) + b * ((t - ta) / (tb - ta))
    };
    let a1 = mix(p0, p1, t0, t1);
    let a2 = mix(p1, p2, t1, t2);
    let a3 = mix(p2, p3, t2, t3);
    let b1 = mix(a1, a2, t0, t2);
    let b2 = mix(a2, a3, t1, t3);
    let c = mix(b1, b2, t1, t2);
    if c.len() < 1e-9 {
        slerp(p1, p2, u)
    } else {
        c.normalize()
    }
}

impl LineStroke {
    fn closed_eff(&self) -> bool {
        self.closed && self.points.len() >= 3
    }

    /// Number of segments between control points.
    pub fn segment_count(&self) -> usize {
        let n = self.points.len();
        if n < 2 {
            0
        } else if self.closed_eff() {
            n
        } else {
            n - 1
        }
    }

    /// Point on segment `i` (from point `i` to point `i+1`) at parameter `t ∈ [0,1]`.
    pub fn eval(&self, i: usize, t: f64) -> Vec3 {
        let n = self.points.len();
        let a = i % n;
        let b = (i + 1) % n;
        let (pa, pb) = (&self.points[a], &self.points[b]);
        let p1 = pa.dir();
        let p2 = pb.dir();
        if !self.smooth || (pa.sharp && pb.sharp) {
            return slerp(p1, p2, t);
        }
        let closed = self.closed_eff();
        let p0 = if pa.sharp || (a == 0 && !closed) {
            p1
        } else {
            self.points[(a + n - 1) % n].dir()
        };
        let p3 = if pb.sharp || (b == n - 1 && !closed) {
            p2
        } else {
            self.points[(b + 1) % n].dir()
        };
        catmull_rom(p0, p1, p2, p3, t)
    }

    /// Width multiplier on segment `i` at `t` (linear between the two points).
    pub fn width_at(&self, i: usize, t: f64) -> f32 {
        let n = self.points.len();
        let wa = self.points[i % n].w;
        let wb = self.points[(i + 1) % n].w;
        wa + (wb - wa) * t as f32
    }

    /// The centre line, sampled so consecutive samples are at most about `step` radians apart.
    pub fn centerline(&self, step: f64) -> Vec<Sample> {
        let n = self.points.len();
        let step = step.max(1e-6);
        let mut out: Vec<Sample> = Vec::new();
        if n == 0 {
            return out;
        }
        if n == 1 {
            out.push(Sample {
                dir: self.points[0].dir(),
                w: self.points[0].w,
                s: 0.0,
            });
            return out;
        }
        for i in 0..self.segment_count() {
            let p1 = self.points[i % n].dir();
            let p2 = self.points[(i + 1) % n].dir();
            let ang = angle_between(p1, p2);
            let mut k = ((ang * 1.2) / step).ceil().clamp(1.0, 4096.0) as usize;
            let pts = loop {
                let pts: Vec<Vec3> = (0..=k).map(|j| self.eval(i, j as f64 / k as f64)).collect();
                let worst = pts
                    .windows(2)
                    .map(|p| angle_between(p[0], p[1]))
                    .fold(0.0, f64::max);
                if worst > step * 1.6 && k < 4096 {
                    k = (k * 2).min(4096);
                } else {
                    break pts;
                }
            };
            for (j, d) in pts.into_iter().enumerate() {
                if j == 0 && i > 0 {
                    continue;
                }
                let t = j as f64 / k as f64;
                out.push(Sample {
                    dir: d,
                    w: self.width_at(i, t),
                    s: 0.0,
                });
            }
        }
        let mut acc = 0.0;
        for i in 1..out.len() {
            acc += angle_between(out[i - 1].dir, out[i].dir);
            out[i].s = acc;
        }
        out
    }

    /// Angular radius (radians) of the full-width stroke.
    pub fn base_radius(&self, w: u32) -> f64 {
        self.width as f64 * 0.5 * TAU / w as f64
    }

    /// Tessellation for rasterizing: samples with angular radii (width × point width × taper).
    fn raster_samples(&self, w: u32, h: u32) -> Vec<RSample> {
        let pa = pixel_angle(w, h);
        let r0 = self.base_radius(w);
        let minw = self
            .points
            .iter()
            .map(|p| p.w)
            .fold(f32::INFINITY, f32::min)
            .clamp(0.25, 4.0) as f64;
        let step = (0.35 * r0 * minw).clamp(1.2 * pa, 24.0 * pa);
        let line = self.centerline(step);
        let total = line.last().map_or(0.0, |s| s.s);
        let taper = !self.closed_eff() && total > 0.0;
        let (ts, te) = (
            self.taper_start.clamp(0.0, 1.0) as f64,
            self.taper_end.clamp(0.0, 1.0) as f64,
        );
        line.iter()
            .map(|s| {
                let mut f = 1.0;
                if taper {
                    if ts > 0.0 {
                        f = f64::min(f, ease(s.s / (ts * total)));
                    }
                    if te > 0.0 {
                        f = f64::min(f, ease((total - s.s) / (te * total)));
                    }
                    f = f.max(0.03);
                }
                let (lon, lat) = lonlat_of(s.dir);
                RSample {
                    dir: s.dir,
                    lon,
                    lat,
                    r: r0 * s.w as f64 * f,
                }
            })
            .collect()
    }

    /// Pixel rectangles (at most two, split at the seam) that contain the stroke.
    pub fn bounds(&self, w: u32, h: u32) -> Vec<IRect> {
        sample_bounds(&self.raster_samples(w, h), w, h)
    }

    /// Clamps every value into its valid range and drops invalid points.
    pub fn sanitize(&mut self) {
        self.points
            .retain(|p| p.lon.is_finite() && p.lat.is_finite() && p.w.is_finite());
        self.points.truncate(MAX_POINTS);
        for p in &mut self.points {
            p.lon = p.lon.rem_euclid(360.0);
            p.lat = p.lat.clamp(-90.0, 90.0);
            p.w = p.w.clamp(MIN_POINT_WIDTH, MAX_POINT_WIDTH);
        }
        let fix =
            |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.opacity = fix(self.opacity, 0.0, 1.0, 1.0);
        self.width = fix(self.width, MIN_WIDTH, MAX_WIDTH, 6.0);
        self.taper_start = fix(self.taper_start, 0.0, 1.0, 0.0);
        self.taper_end = fix(self.taper_end, 0.0, 1.0, 0.0);
    }

    /// Rotates every point (moving the whole stroke along the sphere).
    pub fn rotate(&mut self, r: &Rotation) {
        for p in &mut self.points {
            let d = r.apply(p.dir());
            p.set_dir(d);
        }
    }

    /// The stroke with its point order reversed.
    pub fn reversed(&self) -> LineStroke {
        let mut s = self.clone();
        s.points.reverse();
        std::mem::swap(&mut s.taper_start, &mut s.taper_end);
        s
    }
}

impl LineArt {
    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty()
    }
    pub fn point_count(&self) -> usize {
        self.strokes.iter().map(|s| s.points.len()).sum()
    }
    /// Clamps values, drops empty strokes and enforces the limits.
    pub fn sanitize(&mut self) {
        self.strokes.truncate(MAX_STROKES);
        for s in &mut self.strokes {
            s.sanitize();
        }
        self.strokes.retain(|s| !s.points.is_empty());
    }
    /// Rotates the panorama heading by `deg` degrees (like *Offset heading*).
    pub fn offset_lon(&mut self, deg: f64) {
        for s in &mut self.strokes {
            for p in &mut s.points {
                p.lon = (p.lon + deg).rem_euclid(360.0);
            }
        }
    }
    /// Mirrors left ↔ right (like *Flip horizontally*: `u → 1 − u`).
    pub fn flip_lon(&mut self) {
        for s in &mut self.strokes {
            for p in &mut s.points {
                p.lon = (360.0 - p.lon).rem_euclid(360.0);
            }
        }
    }
    /// Approximate memory use, for the undo budget.
    pub fn bytes(&self) -> usize {
        64 + self
            .strokes
            .iter()
            .map(|s| 64 + s.points.len() * std::mem::size_of::<LinePoint>())
            .sum::<usize>()
    }
}

/// A 3×3 rotation matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rotation(pub [[f64; 3]; 3]);

impl Rotation {
    pub const IDENTITY: Rotation = Rotation([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    /// The shortest rotation taking unit vector `a` onto `b`.
    pub fn between(a: Vec3, b: Vec3) -> Rotation {
        let a = a.normalize();
        let b = b.normalize();
        let v = a.cross(b);
        let s = v.len();
        let c = a.dot(b);
        if s < 1e-12 {
            if c > 0.0 {
                return Self::IDENTITY;
            }
            // 180°: rotate about any axis perpendicular to a.
            let axis = if a.x.abs() < 0.9 {
                a.cross(Vec3::new(1.0, 0.0, 0.0))
            } else {
                a.cross(Vec3::new(0.0, 1.0, 0.0))
            }
            .normalize();
            return Self::axis_angle(axis, PI);
        }
        Self::axis_angle(v * (1.0 / s), s.atan2(c))
    }
    pub fn axis_angle(k: Vec3, ang: f64) -> Rotation {
        let (s, c) = ang.sin_cos();
        let t = 1.0 - c;
        Rotation([
            [
                t * k.x * k.x + c,
                t * k.x * k.y - s * k.z,
                t * k.x * k.z + s * k.y,
            ],
            [
                t * k.x * k.y + s * k.z,
                t * k.y * k.y + c,
                t * k.y * k.z - s * k.x,
            ],
            [
                t * k.x * k.z - s * k.y,
                t * k.y * k.z + s * k.x,
                t * k.z * k.z + c,
            ],
        ])
    }
    pub fn apply(&self, v: Vec3) -> Vec3 {
        let m = &self.0;
        Vec3::new(
            m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
            m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
            m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
        )
    }
}

// ---------------------------------------------------------------------------- bounds

/// Pixel-space box of a capsule between two samples: `(x0, x1, y0, y1, full_width)`.
fn seg_box(a: &RSample, b: &RSample, w: u32, h: u32, pa: f64) -> (f64, f64, f64, f64, bool) {
    let rm = a.r.max(b.r) + pa * 1.5;
    let lat_hi = a.lat.max(b.lat) + rm;
    let lat_lo = a.lat.min(b.lat) - rm;
    let y0 = (0.5 - lat_hi / PI) * h as f64;
    let y1 = (0.5 - lat_lo / PI) * h as f64;
    let maxabs = a.lat.abs().max(b.lat.abs()) + rm;
    if maxabs >= FRAC_PI_2 - 1e-6 {
        return (0.0, w as f64, y0, y1, true);
    }
    let k = rm.sin() / maxabs.cos();
    if k >= 1.0 {
        return (0.0, w as f64, y0, y1, true);
    }
    let m = k.asin() / TAU * w as f64 + 1.0;
    let mut dl = b.lon - a.lon;
    if dl > PI {
        dl -= TAU;
    } else if dl < -PI {
        dl += TAU;
    }
    let xa = a.lon / TAU * w as f64;
    let xb = xa + dl / TAU * w as f64;
    let (x0, x1) = (xa.min(xb) - m, xa.max(xb) + m);
    if x1 - x0 >= w as f64 {
        return (0.0, w as f64, y0, y1, true);
    }
    (x0, x1, y0, y1, false)
}

fn sample_bounds(s: &[RSample], w: u32, h: u32) -> Vec<IRect> {
    if s.is_empty() {
        return Vec::new();
    }
    let pa = pixel_angle(w, h);
    let wf = w as f64;
    let (mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut x0, mut x1) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut full = false;
    // Unwrapped x offset of each sample relative to the first one.
    let mut prev_x = s[0].lon / TAU * wf;
    let mut base = prev_x;
    let n = s.len();
    for i in 0..n.max(2) - 1 {
        let a = &s[i.min(n - 1)];
        let b = &s[(i + 1).min(n - 1)];
        let (bx0, bx1, by0, by1, f) = seg_box(a, b, w, h, pa);
        y0 = y0.min(by0);
        y1 = y1.max(by1);
        if f {
            full = true;
            continue;
        }
        // Keep the boxes continuous across the seam.
        let ax = a.lon / TAU * wf;
        let mut d = ax - prev_x;
        if d > wf / 2.0 {
            d -= wf;
        } else if d < -wf / 2.0 {
            d += wf;
        }
        base += d;
        prev_x = ax;
        let shift = base - ax;
        x0 = x0.min(bx0 + shift);
        x1 = x1.max(bx1 + shift);
    }
    let ry0 = y0.floor().clamp(0.0, h as f64) as u32;
    let ry1 = y1.ceil().clamp(0.0, h as f64) as u32;
    if ry1 <= ry0 {
        return Vec::new();
    }
    let rh = ry1 - ry0;
    if full || x1 - x0 >= wf - 1.0 {
        return vec![IRect::new(0, ry0, w, rh)];
    }
    let fx0 = x0.floor() as i64;
    let fx1 = x1.ceil() as i64;
    let start = fx0.rem_euclid(w as i64);
    let end = start + (fx1 - fx0);
    if end <= w as i64 {
        vec![IRect::new(start as u32, ry0, (end - start) as u32, rh)]
    } else {
        vec![
            IRect::new(start as u32, ry0, w - start as u32, rh),
            IRect::new(0, ry0, (end - w as i64) as u32, rh),
        ]
    }
}

/// Merges overlapping rectangles.
pub fn merge_rects(mut v: Vec<IRect>) -> Vec<IRect> {
    v.retain(|r| !r.is_empty());
    loop {
        let mut merged = false;
        'outer: for i in 0..v.len() {
            for j in i + 1..v.len() {
                if !v[i].intersect(&v[j]).is_empty() {
                    let u = v[i].union(&v[j]);
                    v[i] = u;
                    v.remove(j);
                    merged = true;
                    break 'outer;
                }
            }
        }
        if !merged {
            return v;
        }
    }
}

/// Regions whose pixels differ between rendering `old` and `new`.
pub fn changed_rects(old: &LineArt, new: &LineArt, w: u32, h: u32) -> Vec<IRect> {
    let mut used = vec![false; new.strokes.len()];
    let mut seq = Vec::with_capacity(old.strokes.len());
    let mut rects = Vec::new();
    let mut next = 0usize;
    for s in &old.strokes {
        let found = (next..new.strokes.len())
            .chain(0..next.min(new.strokes.len()))
            .find(|&j| !used[j] && new.strokes[j] == *s);
        match found {
            Some(j) => {
                used[j] = true;
                seq.push(j);
                next = j + 1;
            }
            None => rects.extend(s.bounds(w, h)),
        }
    }
    for (j, s) in new.strokes.iter().enumerate() {
        if !used[j] {
            rects.extend(s.bounds(w, h));
        }
    }
    if seq.windows(2).any(|p| p[0] > p[1]) {
        // The stacking order changed: redraw every stroke.
        for s in old.strokes.iter().chain(&new.strokes) {
            rects.extend(s.bounds(w, h));
        }
    }
    merge_rects(rects)
}

// ---------------------------------------------------------------------------- rasterizer

struct Prepared {
    samples: Vec<RSample>,
    bounds: Vec<IRect>,
    color: [u8; 3],
    opacity: f32,
}

fn prepare(art: &LineArt, w: u32, h: u32, rects: &[IRect]) -> Vec<Prepared> {
    art.strokes
        .iter()
        .filter(|s| s.opacity > 0.0 && !s.points.is_empty())
        .filter_map(|s| {
            let samples = s.raster_samples(w, h);
            let bounds = sample_bounds(&samples, w, h);
            let hit = bounds
                .iter()
                .any(|b| rects.iter().any(|r| !b.intersect(r).is_empty()));
            hit.then_some(Prepared {
                samples,
                bounds,
                color: s.color,
                opacity: s.opacity,
            })
        })
        .collect()
}

/// Re-renders the whole layer from its strokes.
pub fn render_full(art: &LineArt, px: &mut [u8], w: u32, h: u32) {
    render_rects(art, px, w, h, &[IRect::full(w, h)]);
}

/// Clears `rects` and draws every stroke that touches them (clipped to them).
pub fn render_rects(art: &LineArt, px: &mut [u8], w: u32, h: u32, rects: &[IRect]) {
    let full = IRect::full(w, h);
    let rects: Vec<IRect> = rects
        .iter()
        .map(|r| r.intersect(&full))
        .filter(|r| !r.is_empty())
        .collect();
    if rects.is_empty() || px.len() < (w as usize * h as usize * 4) {
        return;
    }
    let prepared = prepare(art, w, h, &rects);
    let cos_lon: Vec<f64> = (0..w)
        .map(|x| ((x as f64 + 0.5) / w as f64 * TAU).cos())
        .collect();
    let sin_lon: Vec<f64> = (0..w)
        .map(|x| ((x as f64 + 0.5) / w as f64 * TAU).sin())
        .collect();
    let ctx = RasterCtx {
        w,
        h,
        pa: pixel_angle(w, h),
        cos_lon: &cos_lon,
        sin_lon: &sin_lon,
        strokes: &prepared,
    };
    for r in rects {
        render_rect(&ctx, px, r);
    }
}

struct RasterCtx<'a> {
    w: u32,
    h: u32,
    pa: f64,
    cos_lon: &'a [f64],
    sin_lon: &'a [f64],
    strokes: &'a [Prepared],
}

const BAND: u32 = 64;

fn render_rect(ctx: &RasterCtx, px: &mut [u8], r: IRect) {
    let row = ctx.w as usize * 4;
    let slice = &mut px[r.y as usize * row..r.bottom() as usize * row];
    let bands: Vec<(u32, &mut [u8])> = slice
        .chunks_mut(BAND as usize * row)
        .enumerate()
        .map(|(i, c)| (r.y + i as u32 * BAND, c))
        .collect();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 16)
        .min(bands.len());
    if threads <= 1 {
        let mut cov = Vec::new();
        for (y0, b) in bands {
            render_band(ctx, b, y0, r, &mut cov);
        }
        return;
    }
    let mut work: Vec<Vec<(u32, &mut [u8])>> = (0..threads).map(|_| Vec::new()).collect();
    for (i, b) in bands.into_iter().enumerate() {
        work[i % threads].push(b);
    }
    std::thread::scope(|s| {
        for list in work {
            s.spawn(move || {
                let mut cov = Vec::new();
                for (y0, b) in list {
                    render_band(ctx, b, y0, r, &mut cov);
                }
            });
        }
    });
}

/// Renders rows `y0..y0+band.len()/row` of rect `r` into `band` (full-width rows).
fn render_band(ctx: &RasterCtx, band: &mut [u8], y0: u32, r: IRect, cov: &mut Vec<f32>) {
    let (w, h) = (ctx.w, ctx.h);
    let row = w as usize * 4;
    let bh = (band.len() / row) as u32;
    let y1 = y0 + bh;
    let clip = IRect::new(r.x, y0, r.w, bh);
    // Clear the region.
    for y in 0..bh as usize {
        band[y * row + r.x as usize * 4..y * row + r.right() as usize * 4].fill(0);
    }
    let cw = r.w as usize;
    cov.clear();
    cov.resize(cw * bh as usize, 0.0);
    let lat_tab: Vec<(f64, f64)> = (y0..y1)
        .map(|y| {
            let lat = (0.5 - (y as f64 + 0.5) / h as f64) * PI;
            (lat.cos(), lat.sin())
        })
        .collect();
    let pa = ctx.pa;
    for st in ctx.strokes {
        if !st.bounds.iter().any(|b| !b.intersect(&clip).is_empty()) {
            continue;
        }
        let (mut tx0, mut tx1, mut ty0, mut ty1) = (usize::MAX, 0usize, usize::MAX, 0usize);
        let n = st.samples.len();
        for i in 0..n.max(2) - 1 {
            let a = &st.samples[i.min(n - 1)];
            let b = &st.samples[(i + 1).min(n - 1)];
            let (bx0, bx1, by0, by1, full) = seg_box(a, b, w, h, pa);
            let yy0 = (by0.floor().max(0.0) as u32).max(y0);
            let yy1 = (by1.ceil().min(h as f64) as u32).min(y1);
            if yy0 >= yy1 {
                continue;
            }
            let (xs, xe) = if full {
                (r.x as i64, r.right() as i64)
            } else {
                (bx0.floor() as i64, bx1.ceil() as i64)
            };
            let ab = b.dir - a.dir;
            let ab2 = ab.dot(ab);
            for y in yy0..yy1 {
                let ly = (y - y0) as usize;
                let (cl, sl) = lat_tab[ly];
                for x in xs..xe {
                    let xi = x.rem_euclid(w as i64) as u32;
                    if xi < r.x || xi >= r.right() {
                        continue;
                    }
                    let p = Vec3::new(
                        cl * ctx.cos_lon[xi as usize],
                        sl,
                        cl * ctx.sin_lon[xi as usize],
                    );
                    let ap = p - a.dir;
                    let t = if ab2 > 1e-30 {
                        (ap.dot(ab) / ab2).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let q = a.dir + ab * t;
                    let d = (p - q).len();
                    let mut rr = a.r + (b.r - a.r) * t;
                    let mut k = 1.0;
                    if rr < 0.5 * pa {
                        k = rr / (0.5 * pa);
                        rr = 0.5 * pa;
                    }
                    let c = (((rr - d) / pa + 0.5).clamp(0.0, 1.0) * k) as f32;
                    if c <= 0.0 {
                        continue;
                    }
                    let lx = (xi - r.x) as usize;
                    let ci = ly * cw + lx;
                    if c > cov[ci] {
                        cov[ci] = c;
                    }
                    tx0 = tx0.min(lx);
                    tx1 = tx1.max(lx + 1);
                    ty0 = ty0.min(ly);
                    ty1 = ty1.max(ly + 1);
                }
            }
        }
        if tx0 >= tx1 {
            continue;
        }
        let col = [st.color[0] as f32, st.color[1] as f32, st.color[2] as f32];
        for ly in ty0..ty1 {
            for lx in tx0..tx1 {
                let ci = ly * cw + lx;
                let c = cov[ci];
                if c <= 0.0 {
                    continue;
                }
                cov[ci] = 0.0;
                let a = c * st.opacity;
                let i = ly * row + (r.x as usize + lx) * 4;
                let p = &mut band[i..i + 4];
                let da = p[3] as f32 / 255.0;
                let oa = a + da * (1.0 - a);
                if oa <= 0.0 {
                    continue;
                }
                for k in 0..3 {
                    let v = (col[k] * a + p[k] as f32 * da * (1.0 - a)) / oa;
                    p[k] = v.round().clamp(0.0, 255.0) as u8;
                }
                p[3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

// ---------------------------------------------------------------------------- editing

/// Replaces the strokes of line layer `id` and re-renders what changed (no history).
/// Returns `false` if `id` is not a line layer.
pub fn apply(doc: &mut Document, id: LayerId, art: LineArt) -> bool {
    let (w, h) = (doc.width, doc.height);
    let rects = {
        let Some(l) = doc.layer_mut(id) else {
            return false;
        };
        let Some(cur) = l.lines.as_mut() else {
            return false;
        };
        let rects = changed_rects(cur, &art, w, h);
        *cur = art;
        let cur = l.lines.as_ref().expect("line layer");
        render_rects(cur, &mut l.pixels, w, h, &rects);
        rects
    };
    for r in rects {
        doc.mark_dirty(id, r);
    }
    true
}

/// Freehand → editable points: smooths the raw samples, then keeps only the points
/// needed to stay within `tol` radians of the input (Ramer–Douglas–Peucker on the
/// sphere; width changes larger than ~0.12 are kept too).
pub fn simplify(raw: &[(Vec3, f32)], tol: f64, smoothing: f32) -> Vec<LinePoint> {
    if raw.is_empty() {
        return Vec::new();
    }
    // Moving average (endpoints fixed).
    let k = (smoothing.clamp(0.0, 1.0) * 6.0).round() as usize;
    let n = raw.len();
    let pts: Vec<(Vec3, f32)> = if k == 0 || n < 3 {
        raw.to_vec()
    } else {
        (0..n)
            .map(|i| {
                if i == 0 || i == n - 1 {
                    return raw[i];
                }
                let r = k.min(i).min(n - 1 - i);
                let (mut acc, mut wsum) = (Vec3::default(), 0.0f32);
                for p in &raw[i - r..=i + r] {
                    acc = acc + p.0;
                    wsum += p.1;
                }
                (acc.normalize(), wsum / (2 * r + 1) as f32)
            })
            .collect()
    };
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (pa, pb) = (pts[a].0, pts[b].0);
        let normal = pa.cross(pb);
        let nl = normal.len();
        let mut worst = (0.0, a);
        for (i, p) in pts.iter().enumerate().take(b).skip(a + 1) {
            let geo = if nl < 1e-9 {
                angle_between(p.0, pa)
            } else {
                // Distance to the great-circle arc between a and b.
                let off = (p.0.dot(normal) / nl).clamp(-1.0, 1.0).asin().abs();
                let inside = pa.cross(p.0).dot(normal) >= 0.0 && p.0.cross(pb).dot(normal) >= 0.0;
                if inside {
                    off
                } else {
                    angle_between(p.0, pa).min(angle_between(p.0, pb))
                }
            };
            let t = (i - a) as f32 / (b - a) as f32;
            let wl = pts[a].1 + (pts[b].1 - pts[a].1) * t;
            let err = (geo / tol.max(1e-9)).max(((p.1 - wl).abs() / 0.12) as f64);
            if err > worst.0 {
                worst = (err, i);
            }
        }
        if worst.0 > 1.0 {
            keep[worst.1] = true;
            stack.push((a, worst.1));
            stack.push((worst.1, b));
        }
    }
    let mut out: Vec<LinePoint> = pts
        .iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(p, _)| LinePoint::new(p.0, p.1.clamp(MIN_POINT_WIDTH, MAX_POINT_WIDTH)))
        .collect();
    out.truncate(MAX_POINTS);
    // A click without movement is a dot.
    if out.len() == 2 && angle_between(out[0].dir(), out[1].dir()) < 1e-7 {
        out.pop();
    }
    out
}

/// Inserts a control point on segment `seg` of stroke `si` at parameter `t`;
/// returns its index.
pub fn insert_point(art: &mut LineArt, si: usize, seg: usize, t: f64) -> Option<usize> {
    let s = art.strokes.get_mut(si)?;
    if seg >= s.segment_count() || s.points.len() >= MAX_POINTS {
        return None;
    }
    let p = LinePoint::new(s.eval(seg, t), s.width_at(seg, t));
    let at = seg + 1;
    s.points.insert(at, p);
    Some(at)
}

/// Removes point `pi` of stroke `si`; the stroke goes away when fewer than two points
/// would remain. Returns `true` if the whole stroke was removed.
pub fn delete_point(art: &mut LineArt, si: usize, pi: usize) -> bool {
    let Some(s) = art.strokes.get_mut(si) else {
        return false;
    };
    if pi >= s.points.len() {
        return false;
    }
    if s.points.len() <= 2 {
        art.strokes.remove(si);
        return true;
    }
    s.points.remove(pi);
    if s.points.len() < 3 {
        s.closed = false;
    }
    false
}

/// Cuts stroke `si` at point `pi` ("disconnect"). An open stroke becomes two strokes
/// sharing that point; a closed one opens there. Returns `false` if nothing changed.
pub fn split_at(art: &mut LineArt, si: usize, pi: usize) -> bool {
    let Some(s) = art.strokes.get(si) else {
        return false;
    };
    let n = s.points.len();
    if pi >= n {
        return false;
    }
    if s.closed && n >= 3 {
        let mut o = s.clone();
        o.points.rotate_left(pi);
        let first = o.points[0];
        o.points.push(first);
        o.closed = false;
        art.strokes[si] = o;
        return true;
    }
    if pi == 0 || pi == n - 1 {
        return false;
    }
    let mut a = s.clone();
    let mut b = s.clone();
    a.points.truncate(pi + 1);
    b.points.drain(..pi);
    a.taper_end = 0.0;
    b.taper_start = 0.0;
    art.strokes[si] = a;
    art.strokes.insert(si + 1, b);
    true
}

/// Joins the end `a_end` (`false` = first point, `true` = last point) of stroke `a`
/// to the end `b_end` of stroke `b` ("connect"). The joint keeps stroke `a`'s point
/// position; the result takes `a`'s style. Joining a stroke to itself closes it.
/// Returns the index of the resulting stroke.
pub fn join(art: &mut LineArt, a: usize, a_end: bool, b: usize, b_end: bool) -> Option<usize> {
    if a >= art.strokes.len() || b >= art.strokes.len() {
        return None;
    }
    if a == b {
        let s = &mut art.strokes[a];
        if s.points.len() < 4 || a_end == b_end {
            return None;
        }
        // The two ends meet: drop the duplicate end point and close.
        let joint = if a_end {
            *s.points.last()?
        } else {
            s.points[0]
        };
        s.points.pop();
        s.points[0].lon = joint.lon;
        s.points[0].lat = joint.lat;
        s.closed = true;
        return Some(a);
    }
    if art.strokes[a].closed || art.strokes[b].closed {
        return None;
    }
    let sa = if a_end {
        art.strokes[a].clone()
    } else {
        art.strokes[a].reversed()
    };
    let sb = if b_end {
        art.strokes[b].reversed()
    } else {
        art.strokes[b].clone()
    };
    if sa.points.len() + sb.points.len() > MAX_POINTS {
        return None;
    }
    let mut out = sa;
    out.points.extend(sb.points.iter().skip(1).copied());
    out.taper_end = sb.taper_end;
    art.strokes[a] = out;
    art.strokes.remove(b);
    Some(if b < a { a - 1 } else { a })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{dir_to_px, px_to_dir};

    fn pt(lon: f64, lat: f64) -> LinePoint {
        LinePoint {
            lon,
            lat,
            w: 1.0,
            sharp: false,
        }
    }
    fn stroke(points: Vec<LinePoint>, width: f32) -> LineStroke {
        LineStroke {
            points,
            color: [255, 0, 0],
            width,
            ..Default::default()
        }
    }
    fn alpha(px: &[u8], w: u32, x: u32, y: u32) -> u8 {
        px[((y * w + x) * 4 + 3) as usize]
    }

    #[test]
    fn curve_passes_through_control_points() {
        let s = stroke(
            vec![
                pt(10.0, 0.0),
                pt(40.0, 20.0),
                pt(80.0, -10.0),
                pt(120.0, 5.0),
            ],
            4.0,
        );
        for i in 0..s.segment_count() {
            let a = s.eval(i, 0.0);
            let b = s.eval(i, 1.0);
            assert!((a - s.points[i].dir()).len() < 1e-9);
            assert!((b - s.points[i + 1].dir()).len() < 1e-9);
            // On the unit sphere.
            assert!((s.eval(i, 0.37).len() - 1.0).abs() < 1e-9);
        }
        // Smooth: the tangent is continuous at an inner point.
        let e = 1e-4;
        let before = s.eval(0, 1.0) - s.eval(0, 1.0 - e);
        let after = s.eval(1, e) - s.eval(1, 0.0);
        let cos = before.normalize().dot(after.normalize());
        assert!(cos > 0.99, "tangent jump {cos}");
    }

    #[test]
    fn straight_segments_are_great_circles() {
        let mut s = stroke(vec![pt(0.0, 0.0), pt(60.0, 40.0), pt(100.0, 0.0)], 4.0);
        s.smooth = false;
        let n = s.points[0].dir().cross(s.points[1].dir()).normalize();
        for k in 0..=10 {
            let d = s.eval(0, k as f64 / 10.0);
            assert!(d.dot(n).abs() < 1e-9);
        }
        // Two sharp points in a smooth stroke also make a straight (great-circle) segment.
        let mut s2 = s.clone();
        s2.smooth = true;
        s2.points[0].sharp = true;
        s2.points[1].sharp = true;
        for k in 0..=10 {
            assert!(s2.eval(0, k as f64 / 10.0).dot(n).abs() < 1e-9);
        }
        // A two-point curve is a great-circle arc as well.
        let s3 = stroke(vec![pt(0.0, 0.0), pt(60.0, 40.0)], 4.0);
        for k in 0..=10 {
            assert!(s3.eval(0, k as f64 / 10.0).dot(n).abs() < 1e-9);
        }
    }

    #[test]
    fn centerline_is_dense_and_measures_length() {
        let s = stroke(vec![pt(0.0, 0.0), pt(90.0, 0.0)], 4.0);
        let step = 0.01;
        let c = s.centerline(step);
        for p in c.windows(2) {
            assert!(angle_between(p[0].dir, p[1].dir) <= step * 1.6 + 1e-12);
        }
        assert!((c.last().unwrap().s - FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn renders_uniform_thickness_on_the_equator_and_crosses_the_seam() {
        let (w, h) = (512u32, 256u32);
        let art = LineArt {
            strokes: vec![stroke(vec![pt(340.0, 0.0), pt(20.0, 0.0)], 8.0)],
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        render_full(&art, &mut px, w, h);
        let yc = h / 2;
        // Covered on both sides of the seam, not in the middle of the image.
        assert_eq!(alpha(&px, w, 2, yc), 255);
        assert_eq!(alpha(&px, w, w - 3, yc), 255);
        assert_eq!(alpha(&px, w, w / 2, yc), 0);
        // Thickness ≈ 8 px.
        let col = 5u32;
        let covered: u32 = (0..h).map(|y| alpha(&px, w, col, y) as u32).sum();
        let thick = covered as f64 / 255.0;
        assert!((thick - 8.0).abs() < 0.6, "thickness {thick}");
        // Colour is the stroke colour.
        let i = ((yc * w + 2) * 4) as usize;
        assert_eq!(&px[i..i + 3], &[255, 0, 0]);
    }

    #[test]
    fn lines_keep_their_angular_width_near_the_pole() {
        // A meridian segment near the north pole: in the equirectangular image it gets
        // wider horizontally by 1/cos(lat), exactly like the sphere requires.
        let (w, h) = (1024u32, 512u32);
        let lat = 70.0f64;
        let art = LineArt {
            strokes: vec![stroke(
                vec![pt(180.0, lat - 5.0), pt(180.0, lat + 5.0)],
                6.0,
            )],
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        render_full(&art, &mut px, w, h);
        let y = ((0.5 - lat / 180.0) * h as f64) as u32;
        let covered: u32 = (0..w).map(|x| alpha(&px, w, x, y) as u32).sum();
        let expect = 6.0 / lat.to_radians().cos();
        let got = covered as f64 / 255.0;
        assert!(
            (got - expect).abs() < expect * 0.12,
            "got {got}, expected {expect}"
        );
        // A stroke passing over the pole renders without gaps.
        let art = LineArt {
            strokes: vec![stroke(vec![pt(0.0, 80.0), pt(180.0, 80.0)], 6.0)],
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        render_full(&art, &mut px, w, h);
        let top_row: u32 = (0..w).map(|x| alpha(&px, w, x, 0) as u32).sum();
        assert!(top_row > 0, "the pole should be covered");
        let d = px_to_dir(5.0, 30.0, w, h);
        let pp = dir_to_px(d, w, h);
        assert!(alpha(&px, w, pp.x as u32, pp.y as u32) > 0);
    }

    #[test]
    fn bounds_contain_the_rendered_pixels() {
        let (w, h) = (512u32, 256u32);
        let mut s = stroke(
            vec![
                pt(350.0, 10.0),
                pt(10.0, 30.0),
                pt(40.0, -20.0),
                pt(80.0, 75.0),
            ],
            12.0,
        );
        s.taper_start = 0.3;
        let b = s.bounds(w, h);
        let art = LineArt { strokes: vec![s] };
        let mut px = vec![0u8; (w * h * 4) as usize];
        render_full(&art, &mut px, w, h);
        for y in 0..h {
            for x in 0..w {
                if alpha(&px, w, x, y) > 0 {
                    assert!(
                        b.iter()
                            .any(|r| x >= r.x && x < r.right() && y >= r.y && y < r.bottom()),
                        "pixel {x},{y} outside {b:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn partial_rerender_matches_full_render() {
        let (w, h) = (512u32, 256u32);
        let mut art = LineArt {
            strokes: vec![
                stroke(vec![pt(20.0, 0.0), pt(60.0, 20.0), pt(100.0, 0.0)], 10.0),
                stroke(vec![pt(40.0, -20.0), pt(80.0, 30.0)], 6.0),
                stroke(vec![pt(350.0, 5.0), pt(15.0, 5.0)], 4.0),
            ],
        };
        art.strokes[1].color = [0, 0, 255];
        art.strokes[1].opacity = 0.6;
        let mut px = vec![0u8; (w * h * 4) as usize];
        render_full(&art, &mut px, w, h);
        let mut new = art.clone();
        new.strokes[0].points[1].lat = -10.0;
        new.strokes[2].points[0].lon = 340.0;
        new.strokes.push(stroke(vec![pt(200.0, 0.0)], 20.0));
        let rects = changed_rects(&art, &new, w, h);
        render_rects(&new, &mut px, w, h, &rects);
        let mut full = vec![0u8; (w * h * 4) as usize];
        render_full(&new, &mut full, w, h);
        assert!(px == full, "incremental render differs");
        // Reordering is detected.
        let mut re = new.clone();
        re.strokes.swap(0, 1);
        let rects = changed_rects(&new, &re, w, h);
        render_rects(&re, &mut px, w, h, &rects);
        render_full(&re, &mut full, w, h);
        assert!(px == full);
        assert!(changed_rects(&re, &re.clone(), w, h).is_empty());
    }

    #[test]
    fn taper_and_point_width_change_thickness() {
        let (w, h) = (512u32, 256u32);
        let mut s = stroke(vec![pt(100.0, 0.0), pt(150.0, 0.0), pt(200.0, 0.0)], 10.0);
        s.points[2].w = 3.0;
        let art = LineArt {
            strokes: vec![s.clone()],
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        render_full(&art, &mut px, w, h);
        let thick = |px: &[u8], lon: f64| {
            let x = (lon / 360.0 * w as f64) as u32;
            (0..h).map(|y| alpha(px, w, x, y) as u32).sum::<u32>() as f64 / 255.0
        };
        assert!((thick(&px, 120.0) - 10.0).abs() < 0.8);
        assert!(thick(&px, 195.0) > 25.0);
        s.points[2].w = 1.0;
        s.taper_start = 0.5;
        let art = LineArt { strokes: vec![s] };
        render_full(&art, &mut px, w, h);
        assert!(thick(&px, 102.0) < 4.0);
        assert!((thick(&px, 190.0) - 10.0).abs() < 0.8);
    }

    #[test]
    fn simplify_keeps_shape_with_few_points() {
        let raw: Vec<(Vec3, f32)> = (0..=200)
            .map(|i| {
                let t = i as f64 / 200.0;
                (
                    dir_from_lonlat(
                        (40.0 + 80.0 * t).to_radians(),
                        (20.0 * (t * PI).sin()).to_radians(),
                    ),
                    1.0,
                )
            })
            .collect();
        let tol = 0.5 * pixel_angle(2048, 1024);
        let pts = simplify(&raw, tol, 0.0);
        assert!(pts.len() >= 3 && pts.len() < 40, "{} points", pts.len());
        assert!((pts[0].dir() - raw[0].0).len() < 1e-9);
        assert!((pts.last().unwrap().dir() - raw[200].0).len() < 1e-9);
        // A straight great-circle drag collapses to its two ends.
        let line: Vec<(Vec3, f32)> = (0..=50)
            .map(|i| (slerp(raw[0].0, raw[200].0, i as f64 / 50.0), 1.0))
            .collect();
        assert_eq!(simplify(&line, tol, 0.0).len(), 2);
        // A click is a dot.
        assert_eq!(
            simplify(&[(raw[0].0, 1.0), (raw[0].0, 1.0)], tol, 0.5).len(),
            1
        );
    }

    #[test]
    fn insert_delete_split_join() {
        let mut art = LineArt {
            strokes: vec![stroke(
                vec![pt(0.0, 0.0), pt(30.0, 0.0), pt(60.0, 0.0)],
                4.0,
            )],
        };
        let i = insert_point(&mut art, 0, 0, 0.5).unwrap();
        assert_eq!(i, 1);
        assert_eq!(art.strokes[0].points.len(), 4);
        assert!((art.strokes[0].points[1].lon - 15.0).abs() < 1.0);
        assert!(!delete_point(&mut art, 0, 1));
        assert_eq!(art.strokes[0].points.len(), 3);
        assert!(split_at(&mut art, 0, 1));
        assert_eq!(art.strokes.len(), 2);
        assert_eq!(art.strokes[0].points.len(), 2);
        assert_eq!(art.strokes[1].points.len(), 2);
        // Connect the end of the first to the start of the second again.
        let j = join(&mut art, 0, true, 1, false).unwrap();
        assert_eq!(j, 0);
        assert_eq!(art.strokes.len(), 1);
        assert_eq!(art.strokes[0].points.len(), 3);
        // Join the two ends of one stroke → closed (needs 3 points left).
        assert_eq!(join(&mut art, 0, true, 0, false), None);
        insert_point(&mut art, 0, 1, 0.5).unwrap();
        assert_eq!(join(&mut art, 0, true, 0, false), Some(0));
        assert!(art.strokes[0].closed);
        assert_eq!(art.strokes[0].points.len(), 3);
        assert_eq!(art.strokes[0].segment_count(), 3);
        // Splitting a closed stroke opens it at that point.
        assert!(split_at(&mut art, 0, 1));
        assert!(!art.strokes[0].closed);
        assert_eq!(art.strokes[0].points.len(), 4);
        // Deleting points of a 2-point stroke removes the stroke.
        art.strokes[0].points.truncate(2);
        assert!(delete_point(&mut art, 0, 0));
        assert!(art.strokes.is_empty());
    }

    #[test]
    fn rotation_moves_strokes_along_the_sphere() {
        let a = dir_from_lonlat(0.3, 0.2);
        let b = dir_from_lonlat(2.0, -0.5);
        let r = Rotation::between(a, b);
        assert!(angle_between(r.apply(a), b) < 1e-9);
        let mut s = stroke(vec![pt(10.0, 10.0), pt(40.0, 20.0)], 4.0);
        let len0 = angle_between(s.points[0].dir(), s.points[1].dir());
        s.rotate(&r);
        let len1 = angle_between(s.points[0].dir(), s.points[1].dir());
        assert!((len0 - len1).abs() < 1e-9);
    }

    #[test]
    fn sanitize_clamps_untrusted_values() {
        let mut art = LineArt {
            strokes: vec![
                LineStroke {
                    points: vec![
                        LinePoint {
                            lon: -30.0,
                            lat: 120.0,
                            w: 99.0,
                            sharp: false,
                        },
                        LinePoint {
                            lon: f64::NAN,
                            lat: 0.0,
                            w: 1.0,
                            sharp: false,
                        },
                    ],
                    width: 1e9,
                    opacity: f32::NAN,
                    ..Default::default()
                },
                LineStroke::default(),
            ],
        };
        art.sanitize();
        assert_eq!(art.strokes.len(), 1);
        let s = &art.strokes[0];
        assert_eq!(s.points.len(), 1);
        assert_eq!(s.points[0].lon, 330.0);
        assert_eq!(s.points[0].lat, 90.0);
        assert_eq!(s.points[0].w, MAX_POINT_WIDTH);
        assert_eq!(s.width, MAX_WIDTH);
        assert_eq!(s.opacity, 1.0);
    }
}
