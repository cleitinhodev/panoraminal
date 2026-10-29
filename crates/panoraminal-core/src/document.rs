//! The layered document and the CPU reference compositor.

use crate::blend::{composite, to_f, to_u8, BlendMode};
use crate::rect::{DirtyRegion, IRect};
use serde::{Deserialize, Serialize};

/// Stable identifier of a layer within a document session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LayerId(pub u64);

/// One raster layer. Pixels are straight-alpha RGBA8, row-major, `width*height*4` bytes.
#[derive(Clone, Debug)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    /// Alpha lock ("preserve transparency"): painting only affects already-opaque pixels.
    pub alpha_locked: bool,
    pub pixels: Vec<u8>,
}

impl Layer {
    pub fn new(id: LayerId, name: impl Into<String>, w: u32, h: u32) -> Self {
        Self {
            id,
            name: name.into(),
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            alpha_locked: false,
            pixels: vec![0; w as usize * h as usize * 4],
        }
    }
    /// True if every pixel is fully transparent.
    pub fn is_empty(&self) -> bool {
        self.pixels.chunks_exact(4).all(|p| p[3] == 0)
    }
}

/// Initial background of a new document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Background {
    White,
    Black,
    Transparent,
    /// Sky gradient above the horizon, ground below — handy for sketching scenes.
    Sky,
    /// Panoraminal's deep blue night gradient.
    Night,
}

/// Properties of a layer that can change without touching its pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerProps {
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    pub alpha_locked: bool,
}

#[derive(Clone, Debug)]
pub struct Document {
    pub width: u32,
    pub height: u32,
    /// Bottom (index 0) to top.
    pub layers: Vec<Layer>,
    pub active: LayerId,
    next_id: u64,
    /// Pixel regions changed since the renderer last synced, per layer.
    pub dirty: Vec<(LayerId, DirtyRegion)>,
    /// Bumped whenever the layer list or a layer property changes.
    pub structure_rev: u64,
    /// Bumped on every content change (used for "modified" tracking and thumbnails).
    pub content_rev: u64,
}

impl Document {
    /// Empty document with no layers. Use [`Document::new_blank`] for a usable one.
    pub fn empty(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            layers: Vec::new(),
            active: LayerId(0),
            next_id: 1,
            dirty: Vec::new(),
            structure_rev: 1,
            content_rev: 1,
        }
    }

    /// A background layer plus an empty "Layer 1" (names are supplied by the caller for i18n).
    pub fn new_blank(
        width: u32,
        height: u32,
        bg: Background,
        bg_name: &str,
        layer_name: &str,
    ) -> Self {
        let mut d = Self::empty(width, height);
        let mut base = d.new_layer(bg_name);
        fill_background(&mut base.pixels, width, height, bg);
        d.layers.push(base);
        let l1 = d.new_layer(layer_name);
        d.active = l1.id;
        d.layers.push(l1);
        d
    }

    pub fn alloc_id(&mut self) -> LayerId {
        let id = LayerId(self.next_id);
        self.next_id += 1;
        id
    }
    /// Creates (but does not insert) a transparent layer with a fresh id.
    pub fn new_layer(&mut self, name: &str) -> Layer {
        let id = self.alloc_id();
        Layer::new(id, name, self.width, self.height)
    }
    pub fn pixel_len(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }
    pub fn index_of(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }
    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }
    pub fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }
    pub fn active_index(&self) -> usize {
        self.index_of(self.active)
            .unwrap_or(self.layers.len().saturating_sub(1))
    }
    pub fn active_layer(&self) -> Option<&Layer> {
        self.layer(self.active)
    }
    /// Makes sure `active` refers to an existing layer.
    pub fn fix_active(&mut self) {
        if self.index_of(self.active).is_none() {
            if let Some(l) = self.layers.last() {
                self.active = l.id;
            }
        }
    }

    pub fn props(&self, id: LayerId) -> Option<LayerProps> {
        self.layer(id).map(|l| LayerProps {
            name: l.name.clone(),
            visible: l.visible,
            opacity: l.opacity,
            blend: l.blend,
            alpha_locked: l.alpha_locked,
        })
    }
    pub fn set_props(&mut self, id: LayerId, p: &LayerProps) {
        if let Some(l) = self.layer_mut(id) {
            l.name = p.name.clone();
            l.visible = p.visible;
            l.opacity = p.opacity.clamp(0.0, 1.0);
            l.blend = p.blend;
            l.alpha_locked = p.alpha_locked;
        }
        self.touch_structure();
    }

    pub fn touch_structure(&mut self) {
        self.structure_rev += 1;
        self.content_rev += 1;
    }
    /// Records that `rect` of layer `id` changed.
    pub fn mark_dirty(&mut self, id: LayerId, rect: IRect) {
        self.content_rev += 1;
        if let Some((_, d)) = self.dirty.iter_mut().find(|(l, _)| *l == id) {
            d.add(rect);
        } else {
            let mut d = DirtyRegion::default();
            d.add(rect);
            self.dirty.push((id, d));
        }
    }
    pub fn mark_layer_full(&mut self, id: LayerId) {
        self.content_rev += 1;
        if let Some((_, d)) = self.dirty.iter_mut().find(|(l, _)| *l == id) {
            d.mark_full();
        } else {
            let mut d = DirtyRegion::default();
            d.mark_full();
            self.dirty.push((id, d));
        }
    }
    pub fn take_dirty(&mut self) -> Vec<(LayerId, DirtyRegion)> {
        std::mem::take(&mut self.dirty)
    }

    /// Composite colour of one pixel (straight RGBA8) over transparency.
    pub fn composite_pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        let mut acc = [0.0f32; 4];
        for l in self.layers.iter().filter(|l| l.visible && l.opacity > 0.0) {
            let p = [
                l.pixels[i],
                l.pixels[i + 1],
                l.pixels[i + 2],
                l.pixels[i + 3],
            ];
            acc = composite(acc, to_f(p), l.opacity, l.blend);
        }
        to_u8(acc)
    }

    /// Flattens all visible layers. With `bg = Some(rgb)` the result is opaque over that colour.
    pub fn flatten(&self, bg: Option<[u8; 3]>) -> Vec<u8> {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut out = vec![0u8; w * h * 4];
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(1, 16);
        let rows_per = h.div_ceil(threads);
        let layers: Vec<&Layer> = self
            .layers
            .iter()
            .filter(|l| l.visible && l.opacity > 0.0)
            .collect();
        std::thread::scope(|s| {
            for (chunk_i, chunk) in out.chunks_mut(rows_per * w * 4).enumerate() {
                let layers = &layers;
                s.spawn(move || {
                    let base = chunk_i * rows_per * w * 4;
                    for (j, px) in chunk.chunks_exact_mut(4).enumerate() {
                        let i = base + j * 4;
                        let mut acc = match bg {
                            Some(c) => [
                                c[0] as f32 / 255.0,
                                c[1] as f32 / 255.0,
                                c[2] as f32 / 255.0,
                                1.0,
                            ],
                            None => [0.0; 4],
                        };
                        for l in layers.iter() {
                            let a = l.pixels[i + 3];
                            if a == 0 {
                                continue;
                            }
                            acc = composite(
                                acc,
                                to_f([l.pixels[i], l.pixels[i + 1], l.pixels[i + 2], a]),
                                l.opacity,
                                l.blend,
                            );
                        }
                        px.copy_from_slice(&to_u8(acc));
                    }
                });
            }
        });
        out
    }

    /// Pixels that result from merging layer `idx` into the layer below it.
    /// The upper layer's blend mode and opacity are applied (the lower layer keeps its own).
    pub fn merged_down_pixels(&self, idx: usize) -> Option<Vec<u8>> {
        if idx == 0 || idx >= self.layers.len() {
            return None;
        }
        let up = &self.layers[idx];
        let low = &self.layers[idx - 1];
        let mut out = low.pixels.clone();
        if up.visible && up.opacity > 0.0 {
            for (o, s) in out.chunks_exact_mut(4).zip(up.pixels.chunks_exact(4)) {
                if s[3] == 0 {
                    continue;
                }
                let r = composite(
                    to_f([o[0], o[1], o[2], o[3]]),
                    to_f([s[0], s[1], s[2], s[3]]),
                    up.opacity,
                    up.blend,
                );
                o.copy_from_slice(&to_u8(r));
            }
        }
        Some(out)
    }

    /// Small RGBA thumbnail (box filtered) of a layer.
    pub fn thumbnail(&self, id: LayerId, tw: u32, th: u32) -> Vec<u8> {
        let Some(l) = self.layer(id) else {
            return vec![0; (tw * th * 4) as usize];
        };
        downsample(&l.pixels, self.width, self.height, tw, th)
    }
}

/// Box-filtered downsample of a straight RGBA8 buffer (alpha-weighted).
pub fn downsample(src: &[u8], w: u32, h: u32, tw: u32, th: u32) -> Vec<u8> {
    let mut out = vec![0u8; (tw * th * 4) as usize];
    let step_x = (w / tw / 4).max(1);
    let step_y = (h / th / 4).max(1);
    for ty in 0..th {
        let y0 = ty * h / th;
        let y1 = ((ty + 1) * h / th).max(y0 + 1);
        for tx in 0..tw {
            let x0 = tx * w / tw;
            let x1 = ((tx + 1) * w / tw).max(x0 + 1);
            let (mut r, mut g, mut b, mut a, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
            let mut y = y0;
            while y < y1 {
                let mut x = x0;
                while x < x1 {
                    let i = ((y * w + x) * 4) as usize;
                    let pa = src[i + 3] as u64;
                    r += src[i] as u64 * pa;
                    g += src[i + 1] as u64 * pa;
                    b += src[i + 2] as u64 * pa;
                    a += pa;
                    n += 1;
                    x += step_x;
                }
                y += step_y;
            }
            let o = ((ty * tw + tx) * 4) as usize;
            if a > 0 {
                out[o] = (r / a) as u8;
                out[o + 1] = (g / a) as u8;
                out[o + 2] = (b / a) as u8;
                out[o + 3] = (a / n.max(1)) as u8;
            }
        }
    }
    out
}

/// Fills a pixel buffer with one of the predefined backgrounds.
pub fn fill_background(px: &mut [u8], w: u32, h: u32, bg: Background) {
    let lerp = |a: [f32; 3], b: [f32; 3], t: f32| {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    };
    for y in 0..h {
        let v = y as f32 / h as f32;
        let c: [f32; 4] = match bg {
            Background::White => [255.0, 255.0, 255.0, 255.0],
            Background::Black => [0.0, 0.0, 0.0, 255.0],
            Background::Transparent => [0.0; 4],
            Background::Sky => {
                let c = if v < 0.5 {
                    let t = v / 0.5;
                    if t < 0.9 {
                        lerp([27.0, 74.0, 168.0], [161.0, 219.0, 251.0], t / 0.9)
                    } else {
                        lerp(
                            [161.0, 219.0, 251.0],
                            [232.0, 244.0, 250.0],
                            (t - 0.9) / 0.1,
                        )
                    }
                } else {
                    lerp([84.0, 104.0, 92.0], [38.0, 44.0, 46.0], (v - 0.5) / 0.5)
                };
                [c[0], c[1], c[2], 255.0]
            }
            Background::Night => {
                let c = if v < 0.5 {
                    lerp([2.0, 6.0, 26.0], [6.0, 60.0, 160.0], v / 0.5)
                } else {
                    lerp([4.0, 30.0, 90.0], [1.0, 4.0, 16.0], (v - 0.5) / 0.5)
                };
                [c[0], c[1], c[2], 255.0]
            }
        };
        let p = [c[0] as u8, c[1] as u8, c[2] as u8, c[3] as u8];
        let row = &mut px[(y * w * 4) as usize..((y + 1) * w * 4) as usize];
        for q in row.chunks_exact_mut(4) {
            q.copy_from_slice(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blank_has_two_layers() {
        let d = Document::new_blank(64, 32, Background::White, "bg", "l1");
        assert_eq!(d.layers.len(), 2);
        assert_eq!(d.active, d.layers[1].id);
        assert_eq!(d.composite_pixel(3, 3), [255, 255, 255, 255]);
    }
    #[test]
    fn flatten_matches_pixel_composite() {
        let mut d = Document::new_blank(64, 32, Background::Sky, "bg", "l1");
        let l = d.layers.last_mut().unwrap();
        l.blend = BlendMode::Multiply;
        l.opacity = 0.6;
        for (i, p) in l.pixels.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(&[(i * 7) as u8, (i * 3) as u8, 200, (i % 256) as u8]);
        }
        let f = d.flatten(None);
        for &(x, y) in &[(0, 0), (10, 5), (63, 31), (40, 20)] {
            let i = ((y * 64 + x) * 4) as usize;
            assert_eq!(&f[i..i + 4], &d.composite_pixel(x, y));
        }
    }
    #[test]
    fn merge_down_preserves_composite_on_opaque_base() {
        let mut d = Document::new_blank(32, 16, Background::Sky, "bg", "l1");
        for m in BlendMode::ALL {
            let l = d.layers.last_mut().unwrap();
            l.blend = m;
            l.opacity = 0.7;
            for (i, p) in l.pixels.chunks_exact_mut(4).enumerate() {
                p.copy_from_slice(&[250, (i * 5) as u8, 30, ((i * 11) % 256) as u8]);
            }
            let before = d.flatten(None);
            let merged = d.merged_down_pixels(1).unwrap();
            let mut d2 = d.clone();
            d2.layers[0].pixels = merged;
            d2.layers.pop();
            let after = d2.flatten(None);
            let maxdiff = before
                .iter()
                .zip(&after)
                .map(|(a, b)| (*a as i32 - *b as i32).abs())
                .max()
                .unwrap();
            assert!(maxdiff <= 1, "{m:?} diff {maxdiff}");
        }
    }
    #[test]
    fn thumbnail_size() {
        let d = Document::new_blank(256, 128, Background::White, "bg", "l1");
        let t = d.thumbnail(d.layers[0].id, 32, 16);
        assert_eq!(t.len(), 32 * 16 * 4);
        assert_eq!(&t[0..4], &[255, 255, 255, 255]);
    }
}
