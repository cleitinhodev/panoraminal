//! The custom brush library of the app: persistence, built-in brushes, selection
//! and cached previews.

use eframe::egui;
use panoraminal_core::tip::{PreparedTip, TipDynamics, TipImage};
use panoraminal_core::{BrushSettings, Document, StrokeEngine, ToolKind};
use panoraminal_io::brushes::{BrushLibrary, CustomBrush};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum LibFilter {
    #[default]
    All,
    Favorites,
    Category(String),
}

pub struct BrushLib {
    pub lib: BrushLibrary,
    pub selected: Option<String>,
    pub filter: LibFilter,
    pub search: String,
    /// Persist changes to the data directory (off in screenshot/test runs).
    pub persist: bool,
    pub thumbs: HashMap<String, egui::TextureHandle>,
    preview: Option<(u64, egui::TextureHandle)>,
    tip_cache: Option<(String, TipDynamics, Arc<PreparedTip>)>,
    /// Brushes used on the current document (embedded on save when enabled).
    pub used: HashSet<String>,
}

pub fn library_dir() -> Option<PathBuf> {
    crate::settings::data_dir().map(|d| d.join("brushes"))
}

impl BrushLib {
    pub fn load(persist: bool) -> Self {
        let mut lib = BrushLibrary::default();
        let mut seeded = false;
        if persist {
            if let Some(dir) = library_dir() {
                if dir.join("library.json").exists() {
                    match BrushLibrary::load(&dir) {
                        Ok(l) => {
                            lib = l;
                            seeded = true;
                        }
                        Err(e) => log::warn!("could not load the brush library: {e}"),
                    }
                }
            }
        }
        if !seeded {
            lib.add(builtin_brushes());
        }
        let s = Self {
            selected: lib.brushes.first().map(|b| b.id.clone()),
            lib,
            filter: LibFilter::All,
            search: String::new(),
            persist,
            thumbs: HashMap::new(),
            preview: None,
            tip_cache: None,
            used: HashSet::new(),
        };
        if persist && !seeded {
            s.save();
        }
        s
    }

    pub fn save(&self) {
        if !self.persist {
            return;
        }
        if let Some(dir) = library_dir() {
            if let Err(e) = self.lib.save(&dir) {
                log::warn!("could not save the brush library: {e}");
            }
        }
    }

    pub fn selected(&self) -> Option<&CustomBrush> {
        self.selected.as_deref().and_then(|id| self.lib.find(id))
    }

    pub fn selected_mut(&mut self) -> Option<&mut CustomBrush> {
        let id = self.selected.clone()?;
        self.lib.find_mut(&id)
    }

    /// Prepared tip of the selected brush (cached until the brush or its dynamics change).
    pub fn prepared(&mut self) -> Option<Arc<PreparedTip>> {
        let b = self.selected()?;
        if let Some((id, d, t)) = &self.tip_cache {
            if *id == b.id && *d == b.dynamics {
                return Some(t.clone());
            }
        }
        let t = b.prepared();
        self.tip_cache = Some((b.id.clone(), b.dynamics.clone(), t.clone()));
        Some(t)
    }

    /// Brush settings to start from when `b` gets selected.
    pub fn settings_for(b: &CustomBrush, current: &BrushSettings) -> BrushSettings {
        let mut s = *current;
        s.size = b.diameter.clamp(1.0, 1000.0).round();
        s.spacing = b.dynamics.spacing.clamp(0.02, 1.0);
        s.pressure_size = b.pressure_size;
        s.pressure_opacity = b.pressure_opacity;
        s.min_size = b.min_size;
        s.clamp();
        s
    }

    pub fn visible(&self) -> Vec<&CustomBrush> {
        let q = self.search.trim().to_lowercase();
        let mut v: Vec<&CustomBrush> = self
            .lib
            .brushes
            .iter()
            .filter(|b| match &self.filter {
                LibFilter::All => true,
                LibFilter::Favorites => b.favorite,
                LibFilter::Category(c) => &b.category == c,
            })
            .filter(|b| {
                q.is_empty()
                    || b.name.to_lowercase().contains(&q)
                    || b.category.to_lowercase().contains(&q)
            })
            .collect();
        // Favourites first, otherwise library order.
        v.sort_by_key(|b| !b.favorite);
        v
    }

    pub fn thumb(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        rgb: [u8; 3],
    ) -> Option<egui::TextureId> {
        if let Some(t) = self.thumbs.get(id) {
            return Some(t.id());
        }
        let b = self.lib.find(id)?;
        let n = 64u32;
        let px = b.thumbnail(n, rgb);
        let img = egui::ColorImage::from_rgba_unmultiplied([n as usize, n as usize], &px);
        let t = ctx.load_texture(format!("brush-{id}"), img, egui::TextureOptions::LINEAR);
        let tid = t.id();
        self.thumbs.insert(id.to_string(), t);
        Some(tid)
    }

    pub fn forget_thumbs(&mut self) {
        self.thumbs.clear();
        self.preview = None;
    }

    /// A preview stroke of the selected brush painted with the real stroke engine.
    pub fn preview(
        &mut self,
        ctx: &egui::Context,
        size: [u32; 2],
        settings: &BrushSettings,
        rgb: [u8; 3],
    ) -> Option<egui::TextureId> {
        let b = self.selected()?;
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            b.id.hash(&mut h);
            serde_json::to_string(&b.dynamics)
                .unwrap_or_default()
                .hash(&mut h);
            size.hash(&mut h);
            rgb.hash(&mut h);
            settings.opacity.to_bits().hash(&mut h);
            settings.flow.to_bits().hash(&mut h);
            settings.spacing.to_bits().hash(&mut h);
            h.finish()
        };
        if let Some((k, t)) = &self.preview {
            if *k == key {
                return Some(t.id());
            }
        }
        let tip = self.prepared()?;
        let px = preview_stroke(tip, settings, size[0], size[1], rgb);
        let img =
            egui::ColorImage::from_rgba_unmultiplied([size[0] as usize, size[1] as usize], &px);
        let t = ctx.load_texture("brush-preview", img, egui::TextureOptions::LINEAR);
        let id = t.id();
        self.preview = Some((key, t));
        Some(id)
    }

    /// Adds imported brushes; returns how many were new. Selects the first new one.
    pub fn import(&mut self, v: Vec<CustomBrush>) -> usize {
        let first = v.first().map(|b| b.id.clone());
        let before = self.lib.brushes.len();
        let n = self.lib.add(v);
        if n > 0 {
            self.selected = self.lib.brushes.get(before).map(|b| b.id.clone()).or(first);
            self.save();
        }
        n
    }

    pub fn delete(&mut self, id: &str) {
        if self.lib.remove(id).is_some() {
            self.thumbs.remove(id);
            if self.selected.as_deref() == Some(id) {
                self.selected = self.lib.brushes.first().map(|b| b.id.clone());
            }
            self.save();
        }
    }

    /// Brushes to embed in a project.
    pub fn used_brushes(&self) -> Vec<CustomBrush> {
        self.lib
            .brushes
            .iter()
            .filter(|b| self.used.contains(&b.id))
            .cloned()
            .collect()
    }
}

/// Paints an S-shaped stroke with the real engine on a small equirectangular canvas
/// near the equator (where distortion is negligible) and returns RGBA pixels.
pub fn preview_stroke(
    tip: Arc<PreparedTip>,
    settings: &BrushSettings,
    w: u32,
    h: u32,
    rgb: [u8; 3],
) -> Vec<u8> {
    // Document width chosen so `w` pixels span ~0.9 rad of longitude.
    let dw = ((w as f64 / 0.9) * std::f64::consts::TAU).round() as u32 & !1;
    let dh = dw / 2;
    let mut doc = Document::empty(dw, dh);
    let l = doc.new_layer("preview");
    doc.active = l.id;
    let id = l.id;
    doc.layers.push(l);
    let mut eng = StrokeEngine::new();
    eng.set_tip(Some(tip));
    eng.seed = 7;
    let mut s = *settings;
    s.size = (h as f32 * 0.42).clamp(4.0, 60.0);
    s.smoothing = 0.0;
    s.pressure_size = true;
    s.pressure_opacity = false;
    s.min_size = 0.35;
    let x0 = dw as f64 / 2.0 - w as f64 / 2.0;
    let y0 = dh as f64 / 2.0 - h as f64 / 2.0;
    let pt = |t: f64| {
        let x = x0 + w as f64 * (0.1 + 0.8 * t);
        let y = y0 + h as f64 * (0.5 - 0.22 * (t * std::f64::consts::TAU).sin());
        panoraminal_core::math::px_to_dir(x, y, dw, dh)
    };
    let pr = |t: f64| (0.35 + 0.65 * (t * std::f64::consts::PI).sin()) as f32;
    let n = 120;
    if eng.begin(&doc, ToolKind::Custom, s, rgb, pt(0.0), pr(0.0)) {
        for i in 1..=n {
            let t = i as f64 / n as f64;
            eng.add_point(pt(t), pr(t), i == n);
        }
        eng.end(&mut doc);
    }
    let mut out = vec![0u8; (w * h * 4) as usize];
    if let Some(l) = doc.layer(id) {
        for y in 0..h {
            let sy = y + y0 as u32;
            let src = ((sy * dw + x0 as u32) * 4) as usize;
            let dst = (y * w * 4) as usize;
            out[dst..dst + (w * 4) as usize]
                .copy_from_slice(&l.pixels[src..src + (w * 4) as usize]);
        }
    }
    out
}

// ---- built-in brushes ----------------------------------------------------------------

fn hash2(x: u32, y: u32, s: u32) -> f32 {
    let mut h = x
        .wrapping_mul(0x8da6_b343)
        .wrapping_add(y.wrapping_mul(0xd816_3841))
        .wrapping_add(s.wrapping_mul(0xcb1a_b31f));
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.0
}

/// Smooth value noise in 0–1.
fn noise(x: f32, y: f32, s: u32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (tx, ty) = (x - xi, y - yi);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let (xi, yi) = (xi as i32 as u32, yi as i32 as u32);
    let a = hash2(xi, yi, s);
    let b = hash2(xi.wrapping_add(1), yi, s);
    let c = hash2(xi, yi.wrapping_add(1), s);
    let d = hash2(xi.wrapping_add(1), yi.wrapping_add(1), s);
    let top = a + (b - a) * sx;
    let bot = c + (d - c) * sx;
    top + (bot - top) * sy
}

fn fbm(x: f32, y: f32, s: u32) -> f32 {
    let mut v = 0.0;
    let mut amp = 0.5;
    let mut f = 1.0;
    for o in 0..4 {
        v += noise(x * f, y * f, s + o) * amp;
        f *= 2.0;
        amp *= 0.5;
    }
    v / 0.9375
}

fn tip_from(n: u32, f: impl Fn(f32, f32) -> f32) -> TipImage {
    let mut data = vec![0u8; (n * n) as usize];
    for y in 0..n {
        for x in 0..n {
            // -1..1, +y up
            let u = (x as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let v = 1.0 - (y as f32 + 0.5) / n as f32 * 2.0;
            data[(y * n + x) as usize] = (f(u, v).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
    TipImage::new(n, n, data).trimmed()
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn brush(
    name: &str,
    id: &str,
    tips: Vec<TipImage>,
    diameter: f32,
    f: impl FnOnce(&mut TipDynamics),
) -> CustomBrush {
    let mut b = CustomBrush::new(name, tips, diameter);
    b.id = id.to_string();
    b.category = "Panoraminal".into();
    b.source = "built-in".into();
    f(&mut b.dynamics);
    b.dynamics.clamp();
    b
}

/// Procedurally generated brushes shipped with the app (seeded into a new library).
pub fn builtin_brushes() -> Vec<CustomBrush> {
    let chalk = tip_from(96, |u, v| {
        let r = (u * u + v * v).sqrt();
        let edge = 1.0 - smooth(0.7, 1.0, r + (fbm(u * 3.0, v * 3.0, 3) - 0.5) * 0.5);
        let grain = smooth(0.35, 0.6, fbm(u * 9.0 + 4.0, v * 9.0, 11));
        edge * grain
    });
    let splatter = tip_from(128, |u, v| {
        let mut c: f32 = 0.0;
        for i in 0..14 {
            let a = hash2(i, 1, 5) * std::f32::consts::TAU;
            let d = hash2(i, 2, 5).sqrt() * 0.75;
            let r = 0.05 + hash2(i, 3, 5) * 0.16;
            let (cx, cy) = (a.cos() * d, a.sin() * d);
            let q = ((u - cx).powi(2) + (v - cy).powi(2)).sqrt() / r;
            c = c.max(1.0 - smooth(0.75, 1.0, q));
        }
        c
    });
    let star = tip_from(96, |u, v| {
        let a = v.atan2(u) + std::f32::consts::FRAC_PI_2;
        let r = (u * u + v * v).sqrt();
        let k = (a * 2.5).cos().abs();
        let lim = 0.45 + 0.5 * k.powf(6.0);
        1.0 - smooth(lim - 0.04, lim + 0.02, r)
    });
    let grass = tip_from(128, |u, v| {
        let mut c: f32 = 0.0;
        for i in 0..7 {
            let x0 = -0.6 + 1.2 * hash2(i, 9, 2);
            let tall = 0.5 + 0.5 * hash2(i, 8, 2);
            let lean = (hash2(i, 7, 2) - 0.5) * 0.5;
            let t = (v + 1.0) / 2.0; // 0 at the bottom
            if t > tall {
                continue;
            }
            let cx = x0 + lean * (t / tall).powi(2);
            let wdt = 0.06 * (1.0 - t / tall);
            c = c.max(1.0 - smooth(wdt * 0.6, wdt, (u - cx).abs()));
        }
        c
    });
    let cloud = tip_from(128, |u, v| {
        let r = (u * u + v * v).sqrt();
        let n = fbm(u * 2.2 + 1.0, v * 2.2, 21);
        let fall = 1.0 - smooth(0.25, 1.0, r);
        smooth(0.25, 0.9, n * fall * 1.6)
    });
    let hatch = tip_from(96, |u, v| {
        let r = (u * u + v * v).sqrt();
        let lines = ((u * 9.0).sin() * 0.5 + 0.5).powf(3.0);
        let _ = v;
        lines * (1.0 - smooth(0.7, 1.0, r))
    });
    let leaf = tip_from(96, |u, v| {
        // A leaf pointing up: lens shape with a vein.
        let w = 0.55 * (1.0 - v * v).max(0.0).sqrt() * (1.0 - 0.25 * v);
        let body = 1.0 - smooth(w - 0.05, w, u.abs());
        let vein = smooth(0.0, 0.03, u.abs());
        body * (0.55 + 0.45 * vein)
    });
    vec![
        brush(
            "Soft round",
            "builtin-soft-round",
            vec![TipImage::ellipse(96, 1.0, 0.0)],
            60.0,
            |d| {
                d.spacing = 0.12;
            },
        ),
        brush("Chalk", "builtin-chalk", vec![chalk], 36.0, |d| {
            d.spacing = 0.12;
            d.angle_jitter = 1.0;
            d.size_jitter = 0.12;
        }),
        brush(
            "Calligraphy",
            "builtin-calligraphy",
            vec![TipImage::ellipse(96, 0.22, 0.85)],
            30.0,
            |d| {
                d.spacing = 0.04;
                d.angle = 45.0;
            },
        ),
        brush("Splatter", "builtin-splatter", vec![splatter], 70.0, |d| {
            d.spacing = 0.6;
            d.scatter = 1.2;
            d.scatter_both_axes = true;
            d.count = 2;
            d.size_jitter = 0.5;
            d.angle_jitter = 1.0;
            d.opacity_jitter = 0.3;
        }),
        brush("Stars", "builtin-stars", vec![star], 34.0, |d| {
            d.spacing = 1.4;
            d.scatter = 1.6;
            d.scatter_both_axes = true;
            d.size_jitter = 0.7;
            d.min_diameter = 0.2;
            d.angle_jitter = 1.0;
            d.opacity_jitter = 0.4;
        }),
        brush("Grass", "builtin-grass", vec![grass], 64.0, |d| {
            d.spacing = 0.3;
            d.scatter = 0.4;
            d.size_jitter = 0.4;
            d.angle_jitter = 0.06;
            d.flip_x_jitter = true;
        }),
        brush("Clouds", "builtin-clouds", vec![cloud], 140.0, |d| {
            d.spacing = 0.25;
            d.scatter = 0.6;
            d.scatter_both_axes = true;
            d.size_jitter = 0.5;
            d.angle_jitter = 1.0;
            d.opacity_jitter = 0.5;
            d.flow_jitter = 0.3;
        }),
        brush("Hatching", "builtin-hatching", vec![hatch], 40.0, |d| {
            d.spacing = 0.5;
            d.follow_direction = true;
            d.opacity_jitter = 0.2;
        }),
        brush("Leaves", "builtin-leaves", vec![leaf], 40.0, |d| {
            d.spacing = 0.7;
            d.scatter = 1.0;
            d.count = 2;
            d.size_jitter = 0.45;
            d.angle_jitter = 1.0;
            d.roundness_jitter = 0.4;
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_valid_and_unique() {
        let v = builtin_brushes();
        let ids: HashSet<_> = v.iter().map(|b| b.id.clone()).collect();
        assert_eq!(ids.len(), v.len());
        for b in &v {
            assert!(!b.tips.is_empty());
            let t = &b.tips[0];
            assert!(t.width > 4 && t.height > 4, "{}", b.name);
            assert!(
                t.data.iter().any(|&p| p > 200),
                "{} has no solid pixels",
                b.name
            );
        }
    }

    #[test]
    fn preview_stroke_paints_something() {
        let b = &builtin_brushes()[1];
        let s = BrushSettings::preset(ToolKind::Custom);
        let px = preview_stroke(b.prepared(), &s, 200, 60, [255, 0, 0]);
        let painted = px.chunks_exact(4).filter(|p| p[3] > 20).count();
        assert!(painted > 400, "{painted}");
        assert!(px
            .chunks_exact(4)
            .filter(|p| p[3] > 0)
            .all(|p| p[0] == 255 && p[1] == 0));
    }
}
