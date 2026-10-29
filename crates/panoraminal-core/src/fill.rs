//! Seam-aware flood fill (paint bucket).
//!
//! The fill wraps horizontally across the 0°/360° seam, so a closed shape drawn
//! across the seam fills correctly.

use crate::brush::{apply_paint, TILE};
use crate::document::Document;
use crate::history::{Edit, Tile};
use crate::rect::IRect;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FillOptions {
    /// Colour tolerance, 0–1.
    pub tolerance: f32,
    /// Decide the region from all visible layers (true) or the active layer only.
    pub sample_merged: bool,
    pub opacity: f32,
    /// Grow the filled region by this many pixels to avoid halos at line edges.
    pub expand: u32,
}

impl Default for FillOptions {
    fn default() -> Self {
        Self {
            tolerance: 0.12,
            sample_merged: true,
            opacity: 1.0,
            expand: 1,
        }
    }
}

/// Computes the fill region starting at (`x`,`y`) in `src` (RGBA8).
pub fn region(src: &[u8], w: u32, h: u32, x: u32, y: u32, tolerance: f32) -> Vec<bool> {
    let n = (w * h) as usize;
    let mut out = vec![false; n];
    if x >= w || y >= h {
        return out;
    }
    let i0 = ((y * w + x) * 4) as usize;
    let r: [u8; 4] = [src[i0], src[i0 + 1], src[i0 + 2], src[i0 + 3]];
    let tol = (tolerance.clamp(0.0, 1.0) * 255.0).round() as i32;
    let like = |i: usize| -> bool {
        let p = &src[i * 4..i * 4 + 4];
        // Fully transparent pixels match each other regardless of their colour.
        if p[3] == 0 && r[3] == 0 {
            return true;
        }
        (0..4).all(|c| (p[c] as i32 - r[c] as i32).abs() <= tol)
    };
    let mut stack = vec![(x, y)];
    while let Some((sx, sy)) = stack.pop() {
        let row = (sy * w) as usize;
        if out[row + sx as usize] || !like(row + sx as usize) {
            continue;
        }
        // Scan left and right (wrapping) to find the span.
        let mut l = sx;
        let mut count = 1;
        loop {
            let nl = if l == 0 { w - 1 } else { l - 1 };
            if count >= w || out[row + nl as usize] || !like(row + nl as usize) {
                break;
            }
            l = nl;
            count += 1;
        }
        let mut len = 0;
        let mut cx = l;
        while len < w {
            let i = row + cx as usize;
            if out[i] || !like(i) {
                break;
            }
            out[i] = true;
            len += 1;
            for ny in [sy.wrapping_sub(1), sy + 1] {
                if ny < h {
                    let j = (ny * w + cx) as usize;
                    if !out[j] && like(j) {
                        stack.push((cx, ny));
                    }
                }
            }
            cx = (cx + 1) % w;
        }
    }
    out
}

fn grow(mask: &mut [bool], w: u32, h: u32, by: u32) {
    for _ in 0..by {
        let src = mask.to_vec();
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                if src[i] {
                    continue;
                }
                let l = (y * w + (x + w - 1) % w) as usize;
                let r = (y * w + (x + 1) % w) as usize;
                let u = if y > 0 { src[i - w as usize] } else { false };
                let d = if y + 1 < h {
                    src[i + w as usize]
                } else {
                    false
                };
                if src[l] || src[r] || u || d {
                    mask[i] = true;
                }
            }
        }
    }
}

/// Fills on the active layer. Returns the undo edit, or `None` if nothing changed.
pub fn flood_fill(
    doc: &mut Document,
    x: u32,
    y: u32,
    rgb: [u8; 3],
    opts: FillOptions,
) -> Option<Edit> {
    let (w, h) = (doc.width, doc.height);
    let idx = doc.active_index();
    let layer = doc.layers.get(idx)?;
    if !layer.visible {
        return None;
    }
    let mut mask = if opts.sample_merged {
        region(&doc.flatten(None), w, h, x, y, opts.tolerance)
    } else {
        region(&layer.pixels, w, h, x, y, opts.tolerance)
    };
    grow(&mut mask, w, h, opts.expand.min(8));
    let id = layer.id;
    let rgbf = rgb.map(|c| c as f32 / 255.0);
    let a = opts.opacity.clamp(0.0, 1.0);
    let tiles_x = w.div_ceil(TILE);
    let tiles_y = h.div_ceil(TILE);
    let layer = &mut doc.layers[idx];
    let lock = layer.alpha_locked;
    let mut tiles = Vec::new();
    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let rect = IRect::new(
                tx * TILE,
                ty * TILE,
                TILE.min(w - tx * TILE),
                TILE.min(h - ty * TILE),
            );
            let any = (rect.y..rect.bottom())
                .any(|yy| (rect.x..rect.right()).any(|xx| mask[(yy * w + xx) as usize]));
            if !any {
                continue;
            }
            tiles.push(Tile::capture(&layer.pixels, w, rect));
            for yy in rect.y..rect.bottom() {
                for xx in rect.x..rect.right() {
                    let i = (yy * w + xx) as usize;
                    if mask[i] {
                        apply_paint(&mut layer.pixels[i * 4..i * 4 + 4], a, rgbf, false, lock);
                    }
                }
            }
        }
    }
    if tiles.is_empty() {
        return None;
    }
    let rects: Vec<_> = tiles.iter().map(|t| t.rect).collect();
    for r in rects {
        doc.mark_dirty(id, r);
    }
    Some(Edit::Pixels { layer: id, tiles })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Background;

    #[test]
    fn fill_wraps_across_seam_and_stops_at_lines() {
        let (w, h) = (64u32, 32u32);
        let mut src = vec![0u8; (w * h * 4) as usize];
        // A vertical wall at x = 20 and x = 40: the region containing x = 0 wraps to x = 63.
        for y in 0..h {
            for x in [20u32, 40] {
                let i = ((y * w + x) * 4) as usize;
                src[i..i + 4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
        let m = region(&src, w, h, 2, 5, 0.1);
        assert!(m[(5 * w + 63) as usize], "wrapped across seam");
        assert!(m[(31 * w + 10) as usize]);
        assert!(!m[(5 * w + 30) as usize], "blocked by the walls");
        assert!(!m[(5 * w + 20) as usize]);
    }

    #[test]
    fn fill_undo() {
        let mut d = Document::new_blank(64, 32, Background::White, "bg", "l1");
        let id = d.active;
        let mut e = flood_fill(&mut d, 5, 5, [255, 0, 0], FillOptions::default()).unwrap();
        assert_eq!(&d.layer(id).unwrap().pixels[0..4], &[255, 0, 0, 255]);
        e.swap(&mut d);
        assert_eq!(d.layer(id).unwrap().pixels[3], 0);
        e.swap(&mut d);
        assert_eq!(&d.layer(id).unwrap().pixels[0..4], &[255, 0, 0, 255]);
    }
}
