//! A floating image being placed on the sphere (before it is merged).

use panoraminal_core::place::{PlaceImage, Placement, Projection};
use panoraminal_core::{LayerId, Vec3};
use panoraminal_render::PlacePreview;
use std::sync::Arc;

/// Longest side kept for a placed image.
pub const MAX_PLACE_DIM: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Handle {
    Move,
    /// Corner `i` (TL, TR, BR, BL): uniform scale (free with Shift).
    Corner(usize),
    /// Edge midpoint `i` (top, right, bottom, left): one-axis scale.
    Edge(usize),
    Rotate,
}

#[derive(Clone, Copy, Debug)]
pub struct DragState {
    pub handle: Handle,
    pub start: Placement,
    pub from: Vec3,
    /// Local (unrotated, normalized) coordinates under the pointer at the start.
    pub local0: (f64, f64),
}

pub struct Floating {
    pub img: Arc<PlaceImage>,
    pub name: String,
    pub p: Placement,
    pub drag: Option<DragState>,
    undo: Vec<Placement>,
    redo: Vec<Placement>,
    preview: Arc<Vec<u8>>,
    pw: u32,
    ph: u32,
    level: i32,
    rev: u64,
}

static REV: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Premultiplied RGBA8 from a premultiplied RGBA16 image.
fn premul8(img: &PlaceImage) -> Vec<u8> {
    img.data
        .iter()
        .map(|&v| ((v as u32 + 128) / 257) as u8)
        .collect()
}

impl Floating {
    pub fn new(img: PlaceImage, name: String, p: Placement) -> Self {
        let img = if img.width.max(img.height) > MAX_PLACE_DIM {
            let s = MAX_PLACE_DIM as f64 / img.width.max(img.height) as f64;
            img.resized(
                ((img.width as f64 * s).round() as u32).max(1),
                ((img.height as f64 * s).round() as u32).max(1),
            )
        } else {
            img
        };
        let mut f = Self {
            img: Arc::new(img),
            name,
            p,
            drag: None,
            undo: Vec::new(),
            redo: Vec::new(),
            preview: Arc::new(Vec::new()),
            pw: 0,
            ph: 0,
            level: i32::MIN,
            rev: 0,
        };
        f.p.clamp();
        f
    }

    /// Records the current placement for local undo (call before a change).
    pub fn checkpoint(&mut self) {
        if self.undo.last() != Some(&self.p) {
            self.undo.push(self.p);
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
    }
    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(p) => {
                self.redo.push(self.p);
                self.p = p;
                true
            }
            None => false,
        }
    }
    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(p) => {
                self.undo.push(self.p);
                self.p = p;
                true
            }
            None => false,
        }
    }

    pub fn set_projection(&mut self, proj: Projection, view: Vec3) {
        if proj != self.p.projection {
            self.checkpoint();
            // Keep the apparent size: convert between angular and plane half extents.
            let (hw, hh) = (self.p.half_w, self.p.half_h);
            self.p.set_projection(proj, view);
            self.p.half_w = hw;
            self.p.half_h = hh;
            self.p.clamp();
        }
    }

    /// The GPU preview of the floating image, downscaled to its on-sphere size so the
    /// single-level texture does not alias.
    pub fn preview(&mut self, layer: LayerId, doc_w: u32) -> PlacePreview {
        let (rx, ry) = self.p.minification(self.img.width, self.img.height, doc_w);
        let m = rx.max(ry).max(1.0);
        let level = m.log2().floor() as i32;
        if level != self.level {
            self.level = level;
            let s = 0.5f64.powi(level);
            let img = if level > 0 {
                self.img.resized(
                    ((self.img.width as f64 * s).ceil() as u32).max(2),
                    ((self.img.height as f64 * s).ceil() as u32).max(2),
                )
            } else {
                (*self.img).clone()
            };
            self.pw = img.width;
            self.ph = img.height;
            self.preview = Arc::new(premul8(&img));
            self.rev = REV.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        PlacePreview {
            layer,
            placement: self.p,
            rgba: self.preview.clone(),
            width: self.pw,
            height: self.ph,
            rev: self.rev,
        }
    }

    /// Updates the placement for a drag of `h` from the start state to pointer `to`.
    /// `free` = Shift (free scale / 15° rotation snap).
    pub fn drag_to(&mut self, to: Vec3, free: bool) {
        let Some(d) = self.drag else { return };
        let s = d.start;
        let mut p = s;
        match d.handle {
            Handle::Move => p.drag_move(&s, d.from, to),
            Handle::Corner(_) | Handle::Edge(_) => {
                if let Some((lx, ly)) = local_unclamped(&s, to) {
                    let (cx, cy) = d.local0;
                    match d.handle {
                        Handle::Corner(_) if !free => {
                            let k = ((lx * cx + ly * cy) / (cx * cx + cy * cy)).max(0.01);
                            scale(&mut p, &s, k, k);
                        }
                        Handle::Corner(_) => {
                            let kx = (lx / cx).abs().max(0.01);
                            let ky = (ly / cy).abs().max(0.01);
                            scale(&mut p, &s, kx, ky);
                        }
                        Handle::Edge(i) => {
                            if i % 2 == 1 {
                                scale(&mut p, &s, (lx / cx).abs().max(0.01), 1.0);
                            } else {
                                scale(&mut p, &s, 1.0, (ly / cy).abs().max(0.01));
                            }
                        }
                        _ => {}
                    }
                }
            }
            Handle::Rotate => {
                if let (Some(a), Some(b)) = (angle_of(&s, d.from), angle_of(&s, to)) {
                    let mut r = s.rotation + (b - a);
                    if free {
                        let st = 15f64.to_radians();
                        r = (r / st).round() * st;
                    }
                    p.rotation = r;
                }
            }
        }
        p.clamp();
        self.p = p;
    }
}

/// Scales the half extents in the space where the image is linear.
fn scale(p: &mut Placement, s: &Placement, kx: f64, ky: f64) {
    match s.projection {
        Projection::Wrap => {
            p.half_w = s.half_w * kx;
            p.half_h = s.half_h * ky;
        }
        Projection::Gnomonic => {
            p.half_w = (s.half_w.tan() * kx).atan();
            p.half_h = (s.half_h.tan() * ky).atan();
        }
    }
}

/// Local normalized coordinates (unbounded) of `d` relative to `p`.
pub fn local_unclamped(p: &Placement, d: Vec3) -> Option<(f64, f64)> {
    p.local(d)
}

/// Pointer angle around the image centre in the image's (rotation-free) plane.
fn angle_of(p: &Placement, d: Vec3) -> Option<f64> {
    let mut q = *p;
    q.rotation = 0.0;
    let (x, y) = q.local(d)?;
    let (x, y) = (x * q.frame().sx, y * q.frame().sy);
    (x * x + y * y > 1e-12).then(|| y.atan2(x))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img() -> PlaceImage {
        PlaceImage::from_rgba8(4, 2, &[200u8; 32])
    }

    #[test]
    fn drag_scale_rotate_and_local_undo() {
        for proj in [Projection::Wrap, Projection::Gnomonic] {
            let c = Vec3::new(1.0, 0.0, 0.0);
            let p = Placement::new(proj, c, c, 1.0, 2.0, 0.5);
            let mut f = Floating::new(img(), "x".into(), p);
            // Corner drag outwards: uniform scale ~1.5x.
            let corner = f.p.dir_at(1.0, 1.0);
            let target = f.p.dir_at(1.5, 1.5);
            f.checkpoint();
            f.drag = Some(DragState {
                handle: Handle::Corner(1),
                start: f.p,
                from: corner,
                local0: (1.0, 1.0),
            });
            f.drag_to(target, false);
            let (lx, ly) = f.p.local(target).unwrap();
            assert!(
                (lx - 1.0).abs() < 1e-3 && (ly - 1.0).abs() < 1e-3,
                "{proj:?} {lx} {ly}"
            );
            // Rotation by dragging the handle a quarter turn.
            f.checkpoint();
            let s = f.p;
            f.drag = Some(DragState {
                handle: Handle::Rotate,
                start: s,
                from: s.dir_at(0.0, 0.8),
                local0: (0.0, 0.8),
            });
            f.drag_to(s.dir_at(-0.8 * s.frame().sy / s.frame().sx, 0.0), false);
            let dr = (f.p.rotation - s.rotation).rem_euclid(std::f64::consts::TAU);
            assert!(
                (dr - std::f64::consts::FRAC_PI_2).abs() < 1e-3,
                "{proj:?} {dr}"
            );
            assert!(f.undo());
            assert_eq!(f.p, s);
            assert!(f.undo());
            assert_eq!(f.p, p);
            assert!(f.redo());
            assert_eq!(f.p, s);
        }
    }

    #[test]
    fn preview_is_premultiplied_and_rebuilt_on_level_change() {
        let c = Vec3::new(0.0, 0.0, 1.0);
        let big = PlaceImage::from_rgba8(512, 256, &vec![255u8; 512 * 256 * 4]);
        let mut f = Floating::new(
            big,
            "x".into(),
            Placement::new(Projection::Wrap, c, c, 0.2, 2.0, 0.5),
        );
        let a = f.preview(LayerId(1), 1024);
        assert!(a.width < 512);
        assert_eq!(a.rgba.len(), (a.width * a.height * 4) as usize);
        let b = f.preview(LayerId(1), 1024);
        assert_eq!(a.rev, b.rev);
        f.p.half_w *= 8.0;
        f.p.half_h *= 8.0;
        let c2 = f.preview(LayerId(1), 1024);
        assert_ne!(c2.rev, a.rev);
        assert!(c2.width > a.width);
    }
}
