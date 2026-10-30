//! Custom (sampled) brush tips and their dynamics.
//!
//! A custom brush is one or more grayscale *tip images* (coverage masks, 255 = full
//! paint) plus [`TipDynamics`]: spacing, angle, roundness, jitter, scattering, count,
//! flipping… — the parameter set of Photoshop / GIMP brushes that maps cleanly onto
//! a 360° canvas.
//!
//! Stamps are rasterized by [`crate::brush::StrokeEngine`] into the same coverage mask
//! as the round brushes, in the *local tangent frame* of the sphere: the horizontal
//! axis is stretched by `1/cos(lat)` (polar correction) and wraps at the seam, so a
//! stamp looks the same anywhere on the panorama. The mask is then composited on the
//! GPU exactly like a normal stroke.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A grayscale coverage image (row-major, `width * height` bytes, 255 = full paint).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TipImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl TipImage {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        debug_assert_eq!(data.len(), (width * height) as usize);
        Self {
            width,
            height,
            data,
        }
    }
    /// A soft or hard elliptical tip (used for Photoshop "computed" brushes).
    pub fn ellipse(diameter: u32, roundness: f32, hardness: f32) -> Self {
        let d = diameter.clamp(1, 2500);
        let h = ((d as f32 * roundness.clamp(0.01, 1.0)).round() as u32).max(1);
        let mut data = vec![0u8; (d * h) as usize];
        let (cx, cy) = (d as f32 * 0.5, h as f32 * 0.5);
        let hard = hardness.clamp(0.0, 1.0);
        for y in 0..h {
            for x in 0..d {
                let dx = (x as f32 + 0.5 - cx) / cx;
                let dy = (y as f32 + 0.5 - cy) / cy;
                let r = (dx * dx + dy * dy).sqrt();
                let c = if r >= 1.0 {
                    0.0
                } else if r <= hard || hard >= 0.999 {
                    1.0
                } else {
                    let t = (r - hard) / (1.0 - hard);
                    1.0 - t * t * (3.0 - 2.0 * t)
                };
                data[(y * d + x) as usize] = (c * 255.0 + 0.5) as u8;
            }
        }
        Self::new(d, h, data)
    }
    pub fn max_side(&self) -> u32 {
        self.width.max(self.height)
    }
    /// Box-filtered half-size copy.
    pub fn half(&self) -> TipImage {
        let w = (self.width / 2).max(1);
        let h = (self.height / 2).max(1);
        let mut data = vec![0u8; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let mut s = 0u32;
                let mut n = 0u32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(self.width - 1);
                    let sy = (y * 2 + dy).min(self.height - 1);
                    s += self.data[(sy * self.width + sx) as usize] as u32;
                    n += 1;
                }
                data[(y * w + x) as usize] = ((s + n / 2) / n) as u8;
            }
        }
        TipImage::new(w, h, data)
    }
    /// Crops away fully transparent borders (keeps at least 1×1).
    pub fn trimmed(&self) -> TipImage {
        let (w, h) = (self.width, self.height);
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
        for y in 0..h {
            for x in 0..w {
                if self.data[(y * w + x) as usize] > 0 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        if x0 > x1 {
            return TipImage::new(1, 1, vec![0]);
        }
        let (nw, nh) = (x1 - x0 + 1, y1 - y0 + 1);
        let mut data = Vec::with_capacity((nw * nh) as usize);
        for y in y0..=y1 {
            let i = (y * w + x0) as usize;
            data.extend_from_slice(&self.data[i..i + nw as usize]);
        }
        TipImage::new(nw, nh, data)
    }
    /// Downscales (box filter) so the longest side is at most `max`.
    pub fn limited(&self, max: u32) -> TipImage {
        let mut t = self.clone();
        while t.max_side() > max {
            t = t.half();
        }
        t
    }
    pub fn inverted(&self) -> TipImage {
        TipImage::new(
            self.width,
            self.height,
            self.data.iter().map(|v| 255 - v).collect(),
        )
    }
    /// Bilinear coverage at tip pixel coordinates (0 outside).
    #[inline]
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        let fx = x - 0.5;
        let fy = y - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = fx - x0;
        let ty = fy - y0;
        let (x0, y0) = (x0 as i64, y0 as i64);
        let (w, h) = (self.width as i64, self.height as i64);
        let get = |x: i64, y: i64| -> f32 {
            if x < 0 || y < 0 || x >= w || y >= h {
                0.0
            } else {
                self.data[(y * w + x) as usize] as f32
            }
        };
        let a = get(x0, y0) + (get(x0 + 1, y0) - get(x0, y0)) * tx;
        let b = get(x0, y0 + 1) + (get(x0 + 1, y0 + 1) - get(x0, y0 + 1)) * tx;
        (a + (b - a) * ty) / 255.0
    }
    /// Small RGBA preview (white paint on transparent), for thumbnails.
    pub fn to_rgba_preview(&self, size: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut out = vec![0u8; (size * size * 4) as usize];
        let m = self.max_side() as f32;
        let k = m / size as f32;
        let src = {
            let mut t = self.clone();
            while t.max_side() as f32 > size as f32 * 2.0 {
                t = t.half();
            }
            t
        };
        let ks = src.max_side() as f32 / m;
        let ox = (m - self.width as f32) * 0.5;
        let oy = (m - self.height as f32) * 0.5;
        for y in 0..size {
            for x in 0..size {
                let tx = ((x as f32 + 0.5) * k - ox) * ks;
                let ty = ((y as f32 + 0.5) * k - oy) * ks;
                let c = src.sample(tx, ty);
                let i = ((y * size + x) * 4) as usize;
                out[i..i + 3].copy_from_slice(&rgb);
                out[i + 3] = (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
        out
    }
}

/// How the tip image is chosen for each dab when a brush has several.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipSelection {
    #[default]
    Sequential,
    Random,
}

/// Shape dynamics of a custom brush. All jitter values are 0–1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TipDynamics {
    /// Dab spacing as a fraction of the diameter.
    pub spacing: f32,
    /// Tip angle in degrees (counter-clockwise).
    pub angle: f32,
    /// Roundness 0.01–1 (1 = undistorted).
    pub roundness: f32,
    pub size_jitter: f32,
    /// Smallest size reached by size jitter, fraction of the diameter.
    pub min_diameter: f32,
    /// Angle jitter, 1 = ±180°.
    pub angle_jitter: f32,
    pub roundness_jitter: f32,
    pub min_roundness: f32,
    /// Scatter distance as a multiple of the diameter (0–10).
    pub scatter: f32,
    pub scatter_both_axes: bool,
    /// Dabs per spacing step (1–16).
    pub count: u32,
    pub count_jitter: f32,
    pub opacity_jitter: f32,
    pub flow_jitter: f32,
    /// Rotate the tip along the stroke direction.
    pub follow_direction: bool,
    pub flip_x: bool,
    pub flip_y: bool,
    pub flip_x_jitter: bool,
    pub flip_y_jitter: bool,
    pub selection: TipSelection,
}

impl Default for TipDynamics {
    fn default() -> Self {
        Self {
            spacing: 0.25,
            angle: 0.0,
            roundness: 1.0,
            size_jitter: 0.0,
            min_diameter: 0.0,
            angle_jitter: 0.0,
            roundness_jitter: 0.0,
            min_roundness: 0.25,
            scatter: 0.0,
            scatter_both_axes: false,
            count: 1,
            count_jitter: 0.0,
            opacity_jitter: 0.0,
            flow_jitter: 0.0,
            follow_direction: false,
            flip_x: false,
            flip_y: false,
            flip_x_jitter: false,
            flip_y_jitter: false,
            selection: TipSelection::Sequential,
        }
    }
}

impl TipDynamics {
    pub fn clamp(&mut self) {
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        self.spacing = fin(self.spacing, 0.25).clamp(0.01, 10.0);
        self.angle = fin(self.angle, 0.0).rem_euclid(360.0);
        self.roundness = fin(self.roundness, 1.0).clamp(0.01, 1.0);
        for v in [
            &mut self.size_jitter,
            &mut self.min_diameter,
            &mut self.angle_jitter,
            &mut self.roundness_jitter,
            &mut self.min_roundness,
            &mut self.count_jitter,
            &mut self.opacity_jitter,
            &mut self.flow_jitter,
        ] {
            *v = fin(*v, 0.0).clamp(0.0, 1.0);
        }
        self.min_roundness = self.min_roundness.max(0.01);
        self.scatter = fin(self.scatter, 0.0).clamp(0.0, 10.0);
        self.count = self.count.clamp(1, 16);
    }
}

/// A tip image with its mip chain, ready for stamping.
#[derive(Debug)]
pub struct PreparedTip {
    /// One mip chain per tip variant (level 0 = full size).
    pub variants: Vec<Vec<TipImage>>,
    pub dynamics: TipDynamics,
}

impl PreparedTip {
    pub fn new(images: &[TipImage], dynamics: &TipDynamics) -> Arc<Self> {
        let mut dynamics = dynamics.clone();
        dynamics.clamp();
        let mut variants = Vec::new();
        for img in images.iter().filter(|i| i.width > 0 && i.height > 0) {
            let mut chain = vec![img.limited(2048)];
            while chain.last().map(|t| t.max_side() > 2).unwrap_or(false) {
                let h = chain.last().unwrap().half();
                chain.push(h);
            }
            variants.push(chain);
        }
        if variants.is_empty() {
            variants.push(vec![TipImage::ellipse(64, 1.0, 0.8)]);
        }
        Arc::new(Self { variants, dynamics })
    }
    /// Picks the mip level whose resolution best matches a dab `diameter_px` wide.
    pub fn level(&self, variant: usize, diameter_px: f64) -> &TipImage {
        let chain = &self.variants[variant % self.variants.len()];
        let mut best = &chain[0];
        for t in chain {
            if t.max_side() as f64 >= diameter_px * 1.4 {
                best = t;
            } else {
                break;
            }
        }
        best
    }
}

/// Small deterministic PRNG (xorshift64*), so jittered strokes are reproducible.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    /// Uniform in `[0, 1)`.
    pub fn next_f32(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32) / (1u64 << 24) as f32
    }
    /// Uniform in `[-1, 1)`.
    pub fn signed(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipse_and_trim() {
        let e = TipImage::ellipse(20, 0.5, 1.0);
        assert_eq!((e.width, e.height), (20, 10));
        assert_eq!(e.data[(5 * 20 + 10) as usize], 255);
        assert_eq!(e.data[0], 0);
        let mut d = vec![0u8; 100];
        d[55] = 200;
        let t = TipImage::new(10, 10, d).trimmed();
        assert_eq!((t.width, t.height, t.data[0]), (1, 1, 200));
    }

    #[test]
    fn mip_selection_matches_dab_size() {
        let p = PreparedTip::new(&[TipImage::ellipse(512, 1.0, 1.0)], &TipDynamics::default());
        assert_eq!(p.level(0, 400.0).max_side(), 512);
        assert_eq!(p.level(0, 40.0).max_side(), 64);
        assert!(p.level(0, 1.0).max_side() <= 4);
    }

    #[test]
    fn rng_is_deterministic_and_uniformish() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        let mut sum = 0.0;
        for _ in 0..10_000 {
            let x = a.next_f32();
            assert_eq!(x, b.next_f32());
            assert!((0.0..1.0).contains(&x));
            sum += x;
        }
        assert!((sum / 10_000.0 - 0.5).abs() < 0.02);
    }

    #[test]
    fn bilinear_sample_edges() {
        let t = TipImage::new(2, 1, vec![255, 0]);
        assert!((t.sample(0.5, 0.5) - 1.0).abs() < 1e-6);
        assert!((t.sample(1.0, 0.5) - 0.5).abs() < 1e-6);
        assert_eq!(t.sample(-3.0, 0.5), 0.0);
    }
}
