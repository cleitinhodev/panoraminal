//! Paint styles: procedural tip shapes, paper textures, pigment-style colour mixing
//! and the noise used by the "wet" (oil) brush.
//!
//! Everything here is generated procedurally — no bitmap assets — and works on the
//! equirectangular canvas without seams:
//!
//! * tip shapes are small coverage images stamped in the local tangent frame of the
//!   sphere (see [`crate::brush`]), exactly like custom brush tips;
//! * noise textures are evaluated on the **sphere** (3D value noise of the pixel's
//!   direction), so they wrap across 0°/360° and do not pinch at the poles;
//! * the canvas weave lives in image space with an integer number of periods across
//!   the width, so it also tiles across the seam.

use crate::tip::{TipDynamics, TipImage};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

// ---- tip shapes -------------------------------------------------------------------

/// Shape of the tip of the built-in tools (pen, brush, airbrush, eraser).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipShape {
    /// The classic round dab (uses the hardness setting).
    #[default]
    Round,
    /// A flat, chisel-like tip — thick and thin strokes depending on the direction.
    Flat,
    /// A clump of bristles that leaves parallel streaks.
    Bristle,
    /// A fan of long bristles.
    Fan,
    /// A grainy, broken disc, like chalk or pastel.
    Chalk,
    /// Soft porous blobs, like a sponge.
    Sponge,
    /// Droplets scattered around the centre.
    Splatter,
    /// A hard square (pixel / block work).
    Square,
}

impl TipShape {
    pub const ALL: [TipShape; 8] = [
        TipShape::Round,
        TipShape::Flat,
        TipShape::Bristle,
        TipShape::Fan,
        TipShape::Chalk,
        TipShape::Sponge,
        TipShape::Splatter,
        TipShape::Square,
    ];
    pub fn id(self) -> &'static str {
        match self {
            TipShape::Round => "round",
            TipShape::Flat => "flat",
            TipShape::Bristle => "bristle",
            TipShape::Fan => "fan",
            TipShape::Chalk => "chalk",
            TipShape::Sponge => "sponge",
            TipShape::Splatter => "splatter",
            TipShape::Square => "square",
        }
    }
    /// Shapes that look best when they turn with the stroke.
    pub fn follows_stroke_by_default(self) -> bool {
        matches!(self, TipShape::Flat | TipShape::Bristle | TipShape::Fan)
    }
    /// Procedural coverage image for this shape (`size`×`size`, except flat/fan).
    pub fn image(self, size: u32, hardness: f32) -> TipImage {
        let n = size.clamp(8, 512);
        let mut rng = Hash(0x51A7_0000 ^ self as u64);
        let mut data = vec![0f32; (n * n) as usize];
        let c = n as f32 * 0.5;
        let soft = |d: f32, hard: f32| -> f32 {
            if d >= 1.0 {
                0.0
            } else if d <= hard {
                1.0
            } else {
                let t = (d - hard) / (1.0 - hard).max(1e-4);
                1.0 - t * t * (3.0 - 2.0 * t)
            }
        };
        let disc = |data: &mut [f32], cx: f32, cy: f32, r: f32, a: f32, hard: f32| {
            let x0 = ((cx - r - 1.0).floor().max(0.0)) as u32;
            let x1 = ((cx + r + 1.0).ceil().min(n as f32)) as u32;
            let y0 = ((cy - r - 1.0).floor().max(0.0)) as u32;
            let y1 = ((cy + r + 1.0).ceil().min(n as f32)) as u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    let dx = x as f32 + 0.5 - cx;
                    let dy = y as f32 + 0.5 - cy;
                    let d = (dx * dx + dy * dy).sqrt() / r.max(0.5);
                    let v = soft(d, hard) * a;
                    let p = &mut data[(y * n + x) as usize];
                    *p = 1.0 - (1.0 - *p) * (1.0 - v);
                }
            }
        };
        let hard = hardness.clamp(0.0, 0.98);
        match self {
            TipShape::Round => return TipImage::ellipse(n, 1.0, hardness),
            TipShape::Square => {
                for y in 0..n {
                    for x in 0..n {
                        let dx = ((x as f32 + 0.5 - c) / c).abs();
                        let dy = ((y as f32 + 0.5 - c) / c).abs();
                        let d = dx.max(dy) * 0.98;
                        data[(y * n + x) as usize] = soft(d, hard.max(0.85));
                    }
                }
            }
            TipShape::Flat => {
                // A rounded rectangle, 4:1, slightly frayed ends.
                for y in 0..n {
                    for x in 0..n {
                        let dx = ((x as f32 + 0.5 - c) / c).abs();
                        let dy = ((y as f32 + 0.5 - c) / (c * 0.26)).abs();
                        let d = (dx.powf(6.0) + dy.powf(6.0)).powf(1.0 / 6.0);
                        let fray = 1.0 - 0.25 * rng.at(x as u64 / 2) * (dx > 0.8) as u32 as f32;
                        data[(y * n + x) as usize] = soft(d, hard.max(0.5)) * fray;
                    }
                }
            }
            TipShape::Bristle => {
                // ~28 bristles of varying thickness and strength inside a disc.
                for _ in 0..28 {
                    let a = rng.next() * TAU as f32;
                    let rr = rng.next().sqrt() * 0.78;
                    let cx = c + a.cos() * rr * c;
                    let cy = c + a.sin() * rr * c;
                    let r = c * (0.12 + 0.14 * rng.next());
                    disc(&mut data, cx, cy, r, 0.55 + 0.45 * rng.next(), 0.35);
                }
            }
            TipShape::Fan => {
                // Bristles along a wide arc (fan brush), drawn as a row of thin strands.
                for i in 0..22 {
                    let t = i as f32 / 21.0 - 0.5;
                    let ang = t * 1.6;
                    let cx = c + ang.sin() * c * 0.86;
                    let cy = c * 1.25 - ang.cos() * c * 0.7;
                    let r = c * (0.05 + 0.05 * rng.next());
                    for k in 0..4 {
                        let k = k as f32 * 0.07;
                        disc(
                            &mut data,
                            cx - ang.sin() * k * c,
                            cy + ang.cos() * k * c,
                            r,
                            0.6 + 0.4 * rng.next(),
                            0.3,
                        );
                    }
                }
            }
            TipShape::Chalk => {
                for y in 0..n {
                    for x in 0..n {
                        let dx = (x as f32 + 0.5 - c) / c;
                        let dy = (y as f32 + 0.5 - c) / c;
                        let d = (dx * dx + dy * dy).sqrt();
                        let g = value_noise2(x as f32 * 0.45, y as f32 * 0.45, 3);
                        let g2 = value_noise2(x as f32 * 0.12, y as f32 * 0.12, 9);
                        let grain = smoothstep(0.30, 0.62, g * 0.65 + g2 * 0.35);
                        let edge = soft(d + 0.18 * (g2 - 0.5), 0.55);
                        data[(y * n + x) as usize] = edge * grain;
                    }
                }
            }
            TipShape::Sponge => {
                for _ in 0..40 {
                    let a = rng.next() * TAU as f32;
                    let rr = rng.next().sqrt() * 0.8;
                    let r = c * (0.06 + 0.16 * rng.next());
                    disc(
                        &mut data,
                        c + a.cos() * rr * c,
                        c + a.sin() * rr * c,
                        r,
                        0.25 + 0.5 * rng.next(),
                        0.1,
                    );
                }
            }
            TipShape::Splatter => {
                disc(&mut data, c, c, c * 0.22, 0.9, 0.5);
                for _ in 0..34 {
                    let a = rng.next() * TAU as f32;
                    let rr = 0.15 + rng.next().powf(0.7) * 0.8;
                    let r = c * (0.025 + 0.09 * rng.next() * (1.0 - rr * 0.6));
                    disc(
                        &mut data,
                        c + a.cos() * rr * c,
                        c + a.sin() * rr * c,
                        r,
                        0.8 + 0.2 * rng.next(),
                        0.6,
                    );
                }
            }
        }
        let bytes = data
            .iter()
            .map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
            .collect();
        TipImage::new(n, n, bytes)
    }
    /// Shape dynamics used when stamping this shape.
    pub fn dynamics(self, angle_deg: f32, follow: bool) -> TipDynamics {
        let mut d = TipDynamics {
            angle: angle_deg,
            follow_direction: follow,
            ..TipDynamics::default()
        };
        match self {
            TipShape::Chalk | TipShape::Sponge => d.angle_jitter = 1.0,
            TipShape::Splatter => {
                d.angle_jitter = 1.0;
                d.size_jitter = 0.35;
                d.min_diameter = 0.5;
                d.flip_x_jitter = true;
                d.flip_y_jitter = true;
            }
            TipShape::Bristle => d.size_jitter = 0.08,
            _ => {}
        }
        d
    }
}

// ---- paper textures -------------------------------------------------------------

/// A paper / canvas grain that modulates the coverage of every dab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperTexture {
    #[default]
    None,
    /// Woven canvas (oil painting).
    Canvas,
    /// Soft, cloudy paper fibres.
    Paper,
    /// Fine, high-contrast grain (charcoal, pastel).
    Grain,
    /// Big porous holes (sponge, dry rag).
    Sponge,
}

impl PaperTexture {
    pub const ALL: [PaperTexture; 5] = [
        PaperTexture::None,
        PaperTexture::Canvas,
        PaperTexture::Paper,
        PaperTexture::Grain,
        PaperTexture::Sponge,
    ];
    pub fn id(self) -> &'static str {
        match self {
            PaperTexture::None => "none",
            PaperTexture::Canvas => "canvas",
            PaperTexture::Paper => "paper",
            PaperTexture::Grain => "grain",
            PaperTexture::Sponge => "sponge",
        }
    }
}

/// Evaluates a [`PaperTexture`] on a `w`×`h` equirectangular document.
///
/// Noise textures are sampled at the pixel's 3D direction (seamless and uniform on
/// the sphere); the canvas weave uses an integer number of periods across the width.
#[derive(Clone, Debug, Default)]
pub struct TextureField {
    kind: PaperTexture,
    strength: f32,
    /// Pixels per texture feature at the equator.
    feature: f64,
    w: u32,
    cols: Vec<(f32, f32)>,
    rows: Vec<(f32, f32)>,
    weave_n: f64,
}

impl TextureField {
    pub fn new(kind: PaperTexture, strength: f32, scale: f32, w: u32, h: u32) -> Self {
        let strength = strength.clamp(0.0, 1.0);
        if kind == PaperTexture::None || strength <= 0.0 || w == 0 || h == 0 {
            return Self::default();
        }
        let base = match kind {
            PaperTexture::Canvas => 7.0,
            PaperTexture::Paper => 9.0,
            PaperTexture::Grain => 2.2,
            PaperTexture::Sponge => 16.0,
            PaperTexture::None => 1.0,
        };
        let feature = (base * scale.clamp(0.25, 8.0) as f64).max(1.0);
        let cols = (0..w)
            .map(|x| {
                let lon = (x as f64 + 0.5) / w as f64 * TAU;
                (lon.sin() as f32, lon.cos() as f32)
            })
            .collect();
        let rows = (0..h)
            .map(|y| {
                let lat = PI * 0.5 - (y as f64 + 0.5) / h as f64 * PI;
                (lat.sin() as f32, lat.cos() as f32)
            })
            .collect();
        Self {
            kind,
            strength,
            feature,
            w,
            cols,
            rows,
            weave_n: (w as f64 / feature).round().max(1.0),
        }
    }
    pub fn is_active(&self) -> bool {
        self.kind != PaperTexture::None && self.strength > 0.0 && !self.cols.is_empty()
    }
    /// Raw texture value (0 = valley, 1 = peak) at pixel `(x, y)` (`x` already wrapped).
    pub fn value(&self, x: u32, y: u32) -> f32 {
        let (Some(&(so, co)), Some(&(sa, ca))) =
            (self.cols.get(x as usize), self.rows.get(y as usize))
        else {
            return 1.0;
        };
        // direction scaled so one unit ≈ one feature at the equator
        let k = (self.w as f64 / TAU / self.feature) as f32;
        let (px, py, pz) = (ca * so * k, sa * k, ca * co * k);
        match self.kind {
            PaperTexture::None => 1.0,
            PaperTexture::Canvas => {
                let n = self.weave_n;
                let u = (x as f64 + 0.5) / self.w as f64 * n;
                let v = (y as f64 + 0.5) / self.w as f64 * n;
                let (fu, fv) = (u.fract(), v.fract());
                let odd = ((u.floor() + v.floor()) as i64).rem_euclid(2) == 1;
                // the visible thread is bright in its middle, dark at the gaps
                let t = if odd { fv } else { fu };
                let thread = (t * PI).sin().powf(0.6) as f32;
                let fuzz = value_noise3(px * 3.0, py * 3.0, pz * 3.0);
                (0.15 + 0.75 * thread + 0.1 * fuzz).clamp(0.0, 1.0)
            }
            PaperTexture::Paper => {
                let a = value_noise3(px, py, pz);
                let b = value_noise3(px * 2.7 + 11.0, py * 2.7, pz * 2.7);
                let c = value_noise3(px * 7.1, py * 7.1 + 5.0, pz * 7.1);
                (0.5 * a + 0.32 * b + 0.18 * c).clamp(0.0, 1.0)
            }
            PaperTexture::Grain => {
                let a = value_noise3(px, py, pz);
                let b = value_noise3(px * 2.3 + 3.0, py * 2.3, pz * 2.3);
                smoothstep(0.32, 0.68, 0.65 * a + 0.35 * b)
            }
            PaperTexture::Sponge => {
                let a = value_noise3(px, py, pz);
                let b = value_noise3(px * 3.1, py * 3.1, pz * 3.1 + 7.0);
                smoothstep(0.38, 0.55, 0.75 * a + 0.25 * b)
            }
        }
    }
    /// Coverage multiplier at `(x, y)`: `1 - strength · (1 - value)`.
    #[inline]
    pub fn factor(&self, x: u32, y: u32) -> f32 {
        if !self.is_active() {
            return 1.0;
        }
        1.0 - self.strength * (1.0 - self.value(x, y))
    }
}

// ---- colour mixing ----------------------------------------------------------------

/// Kubelka–Munk absorption/scattering ratio of a reflectance (0–1).
#[inline]
fn km_ks(r: f32) -> f32 {
    let r = r.clamp(0.002, 1.0);
    (1.0 - r) * (1.0 - r) / (2.0 * r)
}
#[inline]
fn km_r(ks: f32) -> f32 {
    (1.0 + ks - (ks * ks + 2.0 * ks).sqrt()).clamp(0.0, 1.0)
}

/// Mixes colour `a` towards `b` by `t` (0 = `a`, 1 = `b`), components 0–1.
///
/// With `pigment` the mix behaves like paint (a single-constant Kubelka–Munk model,
/// softened with a little linear mixing): yellow + blue gives a green instead of a
/// grey, and dark colours dominate. Without it, it is a plain RGB lerp.
#[inline]
pub fn mix_color(a: [f32; 3], b: [f32; 3], t: f32, pigment: bool) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    let mut o = [0.0; 3];
    for i in 0..3 {
        let lin = a[i] + (b[i] - a[i]) * t;
        o[i] = if pigment && t > 0.0 && t < 1.0 {
            let km = km_r(km_ks(a[i]) * (1.0 - t) + km_ks(b[i]) * t);
            (km * 0.72 + lin * 0.28).clamp(0.0, 1.0)
        } else {
            lin
        };
    }
    o
}

// ---- noise ------------------------------------------------------------------------

/// Tiny deterministic hash-based generator for procedural shapes.
pub(crate) struct Hash(pub u64);
impl Hash {
    pub fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        hash01(self.0)
    }
    /// A stable value for an index (does not advance the generator).
    pub fn at(&self, i: u64) -> f32 {
        hash01(self.0 ^ i.wrapping_mul(0xD6E8_FEB8_6659_FD93))
    }
}

#[inline]
pub fn hash01(mut x: u64) -> f32 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^= x >> 33;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

#[inline]
fn lattice(i: i64, j: i64, k: i64, seed: u64) -> f32 {
    hash01(
        (i as u64).wrapping_mul(0x8DA6_B343)
            ^ (j as u64).wrapping_mul(0xD816_3841)
            ^ (k as u64).wrapping_mul(0xCB1A_B31F)
            ^ seed.wrapping_mul(0x1656_67B1),
    )
}

#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 3D value noise in `[0, 1]`.
pub fn value_noise3(x: f32, y: f32, z: f32) -> f32 {
    let (fx, fy, fz) = (x.floor(), y.floor(), z.floor());
    let (i, j, k) = (fx as i64, fy as i64, fz as i64);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (u, v, w) = (s(x - fx), s(y - fy), s(z - fz));
    let l = |a, b, c| lattice(i + a, j + b, k + c, 7);
    let x00 = l(0, 0, 0) + (l(1, 0, 0) - l(0, 0, 0)) * u;
    let x10 = l(0, 1, 0) + (l(1, 1, 0) - l(0, 1, 0)) * u;
    let x01 = l(0, 0, 1) + (l(1, 0, 1) - l(0, 0, 1)) * u;
    let x11 = l(0, 1, 1) + (l(1, 1, 1) - l(0, 1, 1)) * u;
    let y0 = x00 + (x10 - x00) * v;
    let y1 = x01 + (x11 - x01) * v;
    y0 + (y1 - y0) * w
}

/// 2D value noise in `[0, 1]` (for tip images).
pub fn value_noise2(x: f32, y: f32, seed: u64) -> f32 {
    let (fx, fy) = (x.floor(), y.floor());
    let (i, j) = (fx as i64, fy as i64);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (u, v) = (s(x - fx), s(y - fy));
    let l = |a, b| lattice(i + a, j + b, 0, seed);
    let a = l(0, 0) + (l(1, 0) - l(0, 0)) * u;
    let b = l(0, 1) + (l(1, 1) - l(0, 1)) * u;
    a + (b - a) * v
}

/// 1D value noise in `[0, 1]` (bristle streaks across a stroke).
pub fn value_noise1(x: f32, seed: u64) -> f32 {
    let f = x.floor();
    let i = f as i64;
    let t = x - f;
    let t = t * t * (3.0 - 2.0 * t);
    let a = lattice(i, 0, 0, seed);
    let b = lattice(i + 1, 0, 0, seed);
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3], e: f32) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() <= e)
    }

    #[test]
    fn mixing_endpoints_are_exact() {
        let y = [1.0, 0.9, 0.1];
        let b = [0.1, 0.2, 0.9];
        for pig in [false, true] {
            assert!(close(mix_color(y, b, 0.0, pig), y, 1e-6));
            assert!(close(mix_color(y, b, 1.0, pig), b, 1e-6));
            // mixing a colour with itself is a no-op
            assert!(close(mix_color(y, y, 0.4, pig), y, 0.02));
        }
    }

    #[test]
    fn pigment_yellow_plus_blue_is_green() {
        let y = [1.0, 0.9, 0.1];
        let b = [0.1, 0.2, 0.9];
        let lin = mix_color(y, b, 0.5, false);
        let pig = mix_color(y, b, 0.5, true);
        // the RGB mix is a neutral grey; the pigment mix leans green
        assert!((lin[1] - lin[0]).abs() < 0.2 && (lin[1] - lin[2]).abs() < 0.2);
        assert!(pig[1] > pig[0] + 0.05 && pig[1] > pig[2] + 0.05, "{pig:?}");
        // and it is darker than the linear mix (subtractive)
        assert!(pig.iter().sum::<f32>() < lin.iter().sum::<f32>());
    }

    #[test]
    fn mixing_is_monotonic_in_t() {
        let a = [0.9, 0.1, 0.1];
        let b = [0.95, 0.9, 0.05];
        let mut prev = mix_color(a, b, 0.0, true)[1];
        for i in 1..=10 {
            let g = mix_color(a, b, i as f32 / 10.0, true)[1];
            assert!(g + 1e-4 >= prev, "green must rise towards yellow");
            prev = g;
        }
    }

    #[test]
    fn tip_shapes_have_paint_and_are_deterministic() {
        for s in TipShape::ALL {
            let a = s.image(64, 0.7);
            let b = s.image(64, 0.7);
            assert_eq!(a, b, "{s:?} must be deterministic");
            let sum: u64 = a.data.iter().map(|v| *v as u64).sum();
            assert!(sum > 255 * 40, "{s:?} is empty");
        }
    }

    #[test]
    fn textures_wrap_across_the_seam() {
        for kind in PaperTexture::ALL.into_iter().skip(1) {
            let t = TextureField::new(kind, 1.0, 1.0, 2048, 1024);
            for y in [100u32, 512, 900] {
                let a = t.value(2047, y);
                let b = t.value(0, y);
                let c = t.value(1, y);
                // neighbours across the seam are as close as normal neighbours
                let seam = (a - b).abs();
                let normal = (b - c).abs();
                assert!(
                    seam <= normal + 0.35,
                    "{kind:?} y={y} seam {seam} normal {normal}"
                );
            }
            let mut lo = 1.0f32;
            let mut hi = 0.0f32;
            for x in (0..2048).step_by(7) {
                let v = t.value(x, 512);
                lo = lo.min(v);
                hi = hi.max(v);
            }
            assert!(hi - lo > 0.3, "{kind:?} has no contrast");
        }
        let none = TextureField::new(PaperTexture::None, 1.0, 1.0, 64, 32);
        assert_eq!(none.factor(3, 3), 1.0);
    }
}
