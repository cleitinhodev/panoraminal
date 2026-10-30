//! Placing raster images on the panorama sphere.
//!
//! A [`Placement`] maps a rectangular image onto the sphere in one of two ways:
//!
//! * [`Projection::Wrap`] ("perspective / sphere-wrap"): the image is a patch in
//!   *spherical coordinates* around its centre. Local image coordinates are an
//!   azimuth/elevation pair measured in a tangent frame at the centre, so the image
//!   follows the curvature of the sphere (its straight edges become great/small
//!   circles).
//! * [`Projection::Gnomonic`] ("persistent / projector"): the image lies on the
//!   *image plane* of a projector — the camera at insertion time. Every direction
//!   `d` meets the plane at `(d·r, d·u) / (d·f)`; that point is looked up in the
//!   image. Seen from the projector direction the image is perfectly flat and
//!   undistorted, and it stays anchored in the scene when the camera orbits.
//!
//! Both are written into the equirectangular layer by *inverse mapping*: for each
//! document pixel the direction is computed and mapped back into image space
//! (the GPU preview in `composite.wgsl` runs the same formulas).

use crate::brush::{apply_paint, TILE};
use crate::document::{Document, LayerId};
use crate::history::{Edit, Tile};
use crate::math::{angle_between, dir_to_px, px_to_dir, Vec3};
use crate::rect::IRect;
use serde::{Deserialize, Serialize};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// Largest half-extent of a projector image (degrees off-axis).
pub const MAX_GNOMONIC_HALF: f64 = 80.0 * PI / 180.0;
/// Smallest half-extent (radians) of any placed image.
pub const MIN_HALF: f64 = 0.2 * PI / 180.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    /// Spherical-coordinate patch (curves with the sphere).
    #[default]
    Wrap,
    /// Rectilinear projection from a fixed view (a projector).
    Gnomonic,
}

/// A floating image placed on the sphere.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub projection: Projection,
    /// Direction of the image centre.
    pub center: Vec3,
    /// Counter-clockwise rotation (radians) as seen from inside the sphere.
    pub rotation: f64,
    /// Half width / height. Wrap: angles (radians). Gnomonic: `atan` of the half
    /// extent on the projector's image plane.
    pub half_w: f64,
    pub half_h: f64,
    pub flip_x: bool,
    pub flip_y: bool,
    pub opacity: f32,
    /// Projector axis (gnomonic mode).
    pub view: Vec3,
}

/// Orthonormal `(forward, right, up)` frame looking along `f` (same convention as
/// [`crate::Camera::basis`]), with a fallback at the poles.
pub fn view_basis(f: Vec3) -> (Vec3, Vec3, Vec3) {
    let f = f.normalize();
    let mut r = f.cross(Vec3::UP);
    if r.len() < 1e-6 {
        // Looking straight up/down: pick "east" as the right vector.
        r = Vec3::new(0.0, 0.0, if f.y > 0.0 { -1.0 } else { 1.0 }).cross(f);
        r = f.cross(r);
        if r.len() < 1e-9 {
            r = Vec3::new(1.0, 0.0, 0.0);
        }
    }
    let r = r.normalize();
    let u = r.cross(f).normalize();
    (f, r, u)
}

/// Rotates `v` around unit `axis` by `ang` radians (Rodrigues).
pub fn rotate_about(v: Vec3, axis: Vec3, ang: f64) -> Vec3 {
    let (s, c) = ang.sin_cos();
    v * c + axis.cross(v) * s + axis * (axis.dot(v) * (1.0 - c))
}

/// The frame used by both the CPU bake and the GPU preview.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub projection: Projection,
    pub f: Vec3,
    pub r: Vec3,
    pub u: Vec3,
    /// Rotation applied in the projector plane (gnomonic only; wrap bakes it into r/u).
    pub cos: f64,
    pub sin: f64,
    /// Plane coordinates of the image centre (gnomonic only).
    pub c: (f64, f64),
    /// Half extents in local units (radians for wrap, plane units for gnomonic).
    pub sx: f64,
    pub sy: f64,
}

impl Placement {
    /// A new placement centred on `center`, sized to cover `frac` of a view whose
    /// vertical FOV is `fov` (radians), keeping the image aspect ratio `aspect = w/h`.
    pub fn new(
        projection: Projection,
        center: Vec3,
        view: Vec3,
        fov: f64,
        aspect: f64,
        frac: f64,
    ) -> Self {
        let aspect = if aspect.is_finite() && aspect > 0.0 {
            aspect
        } else {
            1.0
        };
        let mut p = Self {
            projection,
            center: center.normalize(),
            rotation: 0.0,
            half_w: 0.1,
            half_h: 0.1,
            flip_x: false,
            flip_y: false,
            opacity: 1.0,
            view: view.normalize(),
        };
        match projection {
            Projection::Wrap => {
                let hh = (fov * 0.5 * frac).clamp(MIN_HALF, FRAC_PI_2);
                p.half_h = hh;
                p.half_w = hh * aspect;
            }
            Projection::Gnomonic => {
                let th = (fov * 0.5).tan() * frac;
                p.half_h = th.atan();
                p.half_w = (th * aspect).atan();
            }
        }
        p.clamp();
        p
    }

    pub fn clamp(&mut self) {
        let fin = |v: f64, d: f64| if v.is_finite() { v } else { d };
        self.center = if self.center.len() > 1e-9 {
            self.center.normalize()
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        };
        self.view = if self.view.len() > 1e-9 {
            self.view.normalize()
        } else {
            self.center
        };
        self.rotation = fin(self.rotation, 0.0).rem_euclid(TAU);
        let (mw, mh) = match self.projection {
            Projection::Wrap => (PI, FRAC_PI_2),
            Projection::Gnomonic => (MAX_GNOMONIC_HALF, MAX_GNOMONIC_HALF),
        };
        self.half_w = fin(self.half_w, 0.1).clamp(MIN_HALF, mw);
        self.half_h = fin(self.half_h, 0.1).clamp(MIN_HALF, mh);
        self.opacity = if self.opacity.is_finite() {
            self.opacity.clamp(0.0, 1.0)
        } else {
            1.0
        };
        if self.projection == Projection::Gnomonic && self.center.dot(self.view) < 0.05 {
            self.view = self.center;
        }
    }

    pub fn frame(&self) -> Frame {
        match self.projection {
            Projection::Wrap => {
                let (f, r, u) = view_basis(self.center);
                let (s, c) = self.rotation.sin_cos();
                Frame {
                    projection: Projection::Wrap,
                    f,
                    r: r * c + u * s,
                    u: u * c - r * s,
                    cos: 1.0,
                    sin: 0.0,
                    c: (0.0, 0.0),
                    sx: self.half_w,
                    sy: self.half_h,
                }
            }
            Projection::Gnomonic => {
                let (f, r, u) = view_basis(self.view);
                let z = self.center.dot(f).max(1e-6);
                let (s, c) = self.rotation.sin_cos();
                Frame {
                    projection: Projection::Gnomonic,
                    f,
                    r,
                    u,
                    cos: c,
                    sin: s,
                    c: (self.center.dot(r) / z, self.center.dot(u) / z),
                    sx: self.half_w.tan(),
                    sy: self.half_h.tan(),
                }
            }
        }
    }

    /// Normalized image coordinates of direction `d` (`[-1, 1]²` covers the image,
    /// +y up), or `None` if `d` is behind the projector.
    pub fn local(&self, d: Vec3) -> Option<(f64, f64)> {
        self.frame().local(d)
    }
    /// Direction of normalized image coordinates (inverse of [`Self::local`]).
    pub fn dir_at(&self, nx: f64, ny: f64) -> Vec3 {
        self.frame().dir_at(nx, ny)
    }
    /// Texture coordinates (0–1, v down) of a normalized local point, honouring flips.
    pub fn uv(&self, nx: f64, ny: f64) -> (f64, f64) {
        let nx = if self.flip_x { -nx } else { nx };
        let ny = if self.flip_y { -ny } else { ny };
        ((nx + 1.0) * 0.5, (1.0 - ny) * 0.5)
    }
    /// Angular corners of the image (TL, TR, BR, BL).
    pub fn corners(&self) -> [Vec3; 4] {
        [
            self.dir_at(-1.0, 1.0),
            self.dir_at(1.0, 1.0),
            self.dir_at(1.0, -1.0),
            self.dir_at(-1.0, -1.0),
        ]
    }

    /// Switches the projection while keeping the centre and the apparent size.
    /// `view` is the current camera forward (the new projector axis).
    pub fn set_projection(&mut self, projection: Projection, view: Vec3) {
        if projection == self.projection {
            return;
        }
        self.projection = projection;
        if projection == Projection::Gnomonic {
            self.view = view.normalize();
            if self.center.dot(self.view) < 0.3 {
                self.view = self.center;
            }
        }
        self.clamp();
    }

    /// "Re-aim to current view": makes `view` the projector axis while keeping the
    /// image centre where it is.
    pub fn reaim(&mut self, view: Vec3) {
        self.view = view.normalize();
        if self.center.dot(self.view) < 0.3 {
            self.center = self.view;
        }
        self.clamp();
    }

    /// Moves the image so the point under `from` ends up under `to`.
    pub fn drag_move(&mut self, start: &Placement, from: Vec3, to: Vec3) {
        match self.projection {
            Projection::Wrap => {
                let axis = from.cross(to);
                let l = axis.len();
                if l < 1e-12 {
                    self.center = start.center;
                    return;
                }
                let ang = angle_between(from, to);
                self.center = rotate_about(start.center, axis * (1.0 / l), ang).normalize();
            }
            Projection::Gnomonic => {
                let fr = start.frame();
                let plane = |d: Vec3| -> Option<(f64, f64)> {
                    let z = d.dot(fr.f);
                    (z > 0.02).then(|| (d.dot(fr.r) / z, d.dot(fr.u) / z))
                };
                if let (Some(a), Some(b)) = (plane(from), plane(to)) {
                    let (cx, cy) = (fr.c.0 + b.0 - a.0, fr.c.1 + b.1 - a.1);
                    let lim = MAX_GNOMONIC_HALF.tan() * 2.0;
                    let (cx, cy) = (cx.clamp(-lim, lim), cy.clamp(-lim, lim));
                    self.center = (fr.f + fr.r * cx + fr.u * cy).normalize();
                }
            }
        }
        self.clamp();
    }

    /// Image pixels per document pixel at the most minified point of the image
    /// (x and y separately), for prefiltering.
    pub fn minification(&self, img_w: u32, img_h: u32, doc_w: u32) -> (f64, f64) {
        let doc_ang = TAU / doc_w as f64;
        let (mut rx, mut ry) = (0.0f64, 0.0f64);
        let dx = 2.0 / img_w.max(1) as f64;
        let dy = 2.0 / img_h.max(1) as f64;
        for (nx, ny) in [
            (0.0, 0.0),
            (-0.95, -0.95),
            (0.95, -0.95),
            (-0.95, 0.95),
            (0.95, 0.95),
        ] {
            let a = self.dir_at(nx, ny);
            let ax = angle_between(a, self.dir_at(nx + dx, ny)).max(1e-12);
            let ay = angle_between(a, self.dir_at(nx, ny + dy)).max(1e-12);
            rx = rx.max(doc_ang / ax);
            ry = ry.max(doc_ang / ay);
        }
        (rx, ry)
    }

    /// Document pixel rectangles (split at the seam) that the image may touch.
    pub fn pixel_bounds(&self, w: u32, h: u32) -> Vec<IRect> {
        const G: usize = 32;
        let fr = self.frame();
        let wf = w as f64;
        let mut xs = Vec::with_capacity((G + 1) * (G + 1));
        let (mut y0, mut y1) = (f64::MAX, f64::MIN);
        let mut grid = vec![(0.0f64, 0.0f64); (G + 1) * (G + 1)];
        for j in 0..=G {
            for i in 0..=G {
                let nx = -1.0 + 2.0 * i as f64 / G as f64;
                let ny = -1.0 + 2.0 * j as f64 / G as f64;
                let p = dir_to_px(fr.dir_at(nx, ny), w, h);
                grid[j * (G + 1) + i] = (p.x, p.y);
                xs.push(p.x);
                y0 = y0.min(p.y);
                y1 = y1.max(p.y);
            }
        }
        // Largest step between neighbouring samples → padding.
        let (mut sx, mut sy) = (0.0f64, 0.0f64);
        for j in 0..=G {
            for i in 0..=G {
                let a = grid[j * (G + 1) + i];
                for (di, dj) in [(1, 0), (0, 1)] {
                    if i + di > G || j + dj > G {
                        continue;
                    }
                    let b = grid[(j + dj) * (G + 1) + i + di];
                    let mut ddx = (b.0 - a.0).abs();
                    if ddx > wf / 2.0 {
                        ddx = wf - ddx;
                    }
                    sx = sx.max(ddx);
                    sy = sy.max((b.1 - a.1).abs());
                }
            }
        }
        let pad_x = sx * 0.5 + 2.0;
        let pad_y = sy * 0.5 + 2.0;
        let inside = |d: Vec3| {
            fr.local(d)
                .map(|(x, y)| x.abs() <= 1.0 && y.abs() <= 1.0)
                .unwrap_or(false)
        };
        let north = inside(Vec3::new(0.0, 1.0, 0.0));
        let south = inside(Vec3::new(0.0, -1.0, 0.0));
        let mut y0 = (y0 - pad_y).floor().max(0.0);
        let mut y1 = (y1 + pad_y).ceil().min(h as f64);
        if north {
            y0 = 0.0;
        }
        if south {
            y1 = h as f64;
        }
        let (y0, y1) = (y0 as u32, y1 as u32);
        if y1 <= y0 {
            return Vec::new();
        }
        let full = |v: &mut Vec<IRect>| v.push(IRect::new(0, y0, w, y1 - y0));
        let mut out = Vec::new();
        if north || south {
            full(&mut out);
            return out;
        }
        // Circular x range: complement of the largest gap between sorted samples.
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut gap = xs[0] + wf - xs[xs.len() - 1];
        let mut start = xs[0];
        for k in 1..xs.len() {
            let g = xs[k] - xs[k - 1];
            if g > gap {
                gap = g;
                start = xs[k];
            }
        }
        let span = wf - gap + 2.0 * pad_x;
        if span >= wf {
            full(&mut out);
            return out;
        }
        let a = (start - pad_x).floor() as i64;
        let b = (start - pad_x + span).ceil() as i64;
        let a = a.rem_euclid(w as i64);
        let b = a + (b - (start - pad_x).floor() as i64);
        let wi = w as i64;
        if b <= wi {
            out.push(IRect::new(a as u32, y0, (b - a) as u32, y1 - y0));
        } else {
            out.push(IRect::new(a as u32, y0, (wi - a) as u32, y1 - y0));
            out.push(IRect::new(0, y0, (b - wi).min(wi) as u32, y1 - y0));
        }
        out
    }

    /// Bakes `img` into `layer` at full quality (bicubic, edge antialiasing) and
    /// returns the undo edit. `img` should already be prefiltered with
    /// [`PlaceImage::prefiltered_for`].
    pub fn bake(&self, img: &PlaceImage, doc: &mut Document, layer: LayerId) -> Option<Edit> {
        let (w, h) = (doc.width, doc.height);
        let rects = self.pixel_bounds(w, h);
        if rects.is_empty() || self.opacity <= 0.0 {
            return None;
        }
        let fr = self.frame();
        let opacity = self.opacity;
        let (fx, fy) = (self.flip_x, self.flip_y);
        let lay = doc.layer_mut(layer)?;
        let lock = lay.alpha_locked;
        // Undo tiles first.
        let tiles_x = w.div_ceil(TILE);
        let mut tset = vec![false; (tiles_x * h.div_ceil(TILE)) as usize];
        for r in &rects {
            for ty in r.y / TILE..=(r.bottom() - 1) / TILE {
                for tx in r.x / TILE..=(r.right() - 1) / TILE {
                    tset[(ty * tiles_x + tx) as usize] = true;
                }
            }
        }
        let mut tiles = Vec::new();
        for (i, t) in tset.iter().enumerate() {
            if *t {
                let tx = i as u32 % tiles_x * TILE;
                let ty = i as u32 / tiles_x * TILE;
                let rect = IRect::new(tx, ty, TILE.min(w - tx), TILE.min(h - ty));
                tiles.push(Tile::capture(&lay.pixels, w, rect));
            }
        }
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(1, 16);
        for r in &rects {
            let rw = r.w as usize;
            // Process in bands so memory stays bounded.
            let band = 128u32;
            let mut by = r.y;
            while by < r.bottom() {
                let bh = band.min(r.bottom() - by);
                let mut out = vec![[0f32; 4]; rw * bh as usize];
                let rows_per = (bh as usize).div_ceil(threads).max(1);
                std::thread::scope(|s| {
                    for (ci, chunk) in out.chunks_mut(rows_per * rw).enumerate() {
                        let fr = &fr;
                        s.spawn(move || {
                            for (k, px) in chunk.iter_mut().enumerate() {
                                let y = by + (ci * rows_per + k / rw) as u32;
                                let x = r.x + (k % rw) as u32;
                                *px = shade(fr, img, x, y, w, h, fx, fy);
                            }
                        });
                    }
                });
                for row in 0..bh {
                    let y = by + row;
                    for col in 0..r.w {
                        let c = out[row as usize * rw + col as usize];
                        if c[3] <= 1e-5 {
                            continue;
                        }
                        let a = (c[3] * opacity).min(1.0);
                        let rgb = [c[0] / c[3], c[1] / c[3], c[2] / c[3]].map(|v| v.min(1.0));
                        let i = ((y * w + r.x + col) * 4) as usize;
                        apply_paint(&mut lay.pixels[i..i + 4], a, rgb, false, lock);
                    }
                }
                by += bh;
            }
        }
        for r in &rects {
            doc.mark_dirty(layer, *r);
        }
        Some(Edit::Pixels { layer, tiles })
    }
}

/// Bakes a placement and records it in the history as one undo step.
///
/// With `new_layer = Some(name)` a new layer is created above the active one and the
/// image is baked into it (layer creation and pixels undo together). Returns the
/// layer that received the image.
pub fn commit(
    doc: &mut Document,
    hist: &mut crate::History,
    p: &Placement,
    img: &PlaceImage,
    new_layer: Option<&str>,
) -> Option<LayerId> {
    let before = doc.active;
    match new_layer {
        None => {
            let id = doc.active_layer()?.id;
            let e = p.bake(img, doc, id)?;
            hist.push(e, "hist.place_image", before, id);
            Some(id)
        }
        Some(name) => {
            if doc.layers.len() >= crate::MAX_LAYERS {
                return None;
            }
            let l = doc.new_layer(name);
            let id = l.id;
            let index = (doc.active_index() + 1).min(doc.layers.len());
            doc.layers.insert(index, l);
            doc.active = id;
            doc.touch_structure();
            doc.mark_layer_full(id);
            let mut v = vec![Edit::Remove { index, stash: None }];
            if let Some(e) = p.bake(img, doc, id) {
                v.push(e);
            }
            hist.push(Edit::Group(v), "hist.place_image_layer", before, id);
            Some(id)
        }
    }
}

impl Frame {
    #[inline]
    pub fn local(&self, d: Vec3) -> Option<(f64, f64)> {
        match self.projection {
            Projection::Wrap => {
                let x = d.dot(self.r).atan2(d.dot(self.f));
                let y = d.dot(self.u).clamp(-1.0, 1.0).asin();
                Some((x / self.sx, y / self.sy))
            }
            Projection::Gnomonic => {
                let z = d.dot(self.f);
                if z <= 1e-6 {
                    return None;
                }
                let qx = d.dot(self.r) / z - self.c.0;
                let qy = d.dot(self.u) / z - self.c.1;
                let lx = qx * self.cos + qy * self.sin;
                let ly = -qx * self.sin + qy * self.cos;
                Some((lx / self.sx, ly / self.sy))
            }
        }
    }
    #[inline]
    pub fn dir_at(&self, nx: f64, ny: f64) -> Vec3 {
        match self.projection {
            Projection::Wrap => {
                let (x, y) = (nx * self.sx, ny * self.sy);
                let (sx, cx) = x.sin_cos();
                let (sy, cy) = y.sin_cos();
                (self.f * (cy * cx) + self.r * (cy * sx) + self.u * sy).normalize()
            }
            Projection::Gnomonic => {
                let (lx, ly) = (nx * self.sx, ny * self.sy);
                let px = self.c.0 + lx * self.cos - ly * self.sin;
                let py = self.c.1 + lx * self.sin + ly * self.cos;
                (self.f + self.r * px + self.u * py).normalize()
            }
        }
    }
}

/// Shades one document pixel: premultiplied RGBA (coverage in alpha).
#[allow(clippy::too_many_arguments)]
#[inline]
fn shade(
    fr: &Frame,
    img: &PlaceImage,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    fx: bool,
    fy: bool,
) -> [f32; 4] {
    let to_img = |nx: f64, ny: f64| -> (f32, f32) {
        let nx = if fx { -nx } else { nx };
        let ny = if fy { -ny } else { ny };
        (
            ((nx + 1.0) * 0.5 * img.width as f64) as f32,
            ((1.0 - ny) * 0.5 * img.height as f64) as f32,
        )
    };
    let inside = |l: Option<(f64, f64)>| l.filter(|(a, b)| a.abs() <= 1.0 && b.abs() <= 1.0);
    let mut subs = [None; 4];
    let mut n = 0;
    for (k, (ox, oy)) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)]
        .into_iter()
        .enumerate()
    {
        let d = px_to_dir(x as f64 + ox, y as f64 + oy, w, h);
        subs[k] = inside(fr.local(d));
        if subs[k].is_some() {
            n += 1;
        }
    }
    if n == 0 {
        return [0.0; 4];
    }
    if n == 4 {
        let d = px_to_dir(x as f64 + 0.5, y as f64 + 0.5, w, h);
        if let Some((a, b)) = fr.local(d) {
            let (ix, iy) = to_img(a.clamp(-1.0, 1.0), b.clamp(-1.0, 1.0));
            return img.sample_bicubic(ix, iy);
        }
    }
    let mut acc = [0f32; 4];
    for (a, b) in subs.iter().flatten() {
        let (ix, iy) = to_img(*a, *b);
        let c = img.sample_bilinear(ix, iy);
        for i in 0..4 {
            acc[i] += c[i] * 0.25;
        }
    }
    acc
}

/// A premultiplied RGBA16 image ready to be placed.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaceImage {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA, 0–65535.
    pub data: Vec<u16>,
}

impl PlaceImage {
    /// From straight-alpha RGBA8.
    pub fn from_rgba8(width: u32, height: u32, px: &[u8]) -> Self {
        let mut data = Vec::with_capacity(px.len());
        for p in px.chunks_exact(4) {
            let a = p[3] as u32;
            for &c in &p[..3] {
                data.push(((c as u32 * a * 65535 + 255 * 255 / 2) / (255 * 255)) as u16);
            }
            data.push((a * 257) as u16);
        }
        Self {
            width,
            height,
            data,
        }
    }
    /// Straight-alpha RGBA8 (for previews / GPU upload).
    pub fn to_rgba8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len());
        for p in self.data.chunks_exact(4) {
            let a = p[3] as f32;
            for &c in &p[..3] {
                let v = if a > 0.0 { c as f32 / a } else { 0.0 };
                out.push((v.min(1.0) * 255.0 + 0.5) as u8);
            }
            out.push(((a / 257.0) + 0.5) as u8);
        }
        out
    }
    #[inline]
    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        let x = x.clamp(0, self.width as i64 - 1) as usize;
        let y = y.clamp(0, self.height as i64 - 1) as usize;
        let i = (y * self.width as usize + x) * 4;
        let k = 1.0 / 65535.0;
        [
            self.data[i] as f32 * k,
            self.data[i + 1] as f32 * k,
            self.data[i + 2] as f32 * k,
            self.data[i + 3] as f32 * k,
        ]
    }
    /// Bilinear sample at image pixel coordinates (clamped to the edge).
    pub fn sample_bilinear(&self, x: f32, y: f32) -> [f32; 4] {
        let fx = x - 0.5;
        let fy = y - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let a = self.texel(x0, y0);
        let b = self.texel(x0 + 1, y0);
        let c = self.texel(x0, y0 + 1);
        let d = self.texel(x0 + 1, y0 + 1);
        let mut o = [0f32; 4];
        for i in 0..4 {
            let top = a[i] + (b[i] - a[i]) * tx;
            let bot = c[i] + (d[i] - c[i]) * tx;
            o[i] = top + (bot - top) * ty;
        }
        o
    }
    /// Catmull-Rom bicubic sample (premultiplied, clamped to a valid range).
    pub fn sample_bicubic(&self, x: f32, y: f32) -> [f32; 4] {
        let fx = x - 0.5;
        let fy = y - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let wts = |t: f32| -> [f32; 4] {
            let t2 = t * t;
            let t3 = t2 * t;
            [
                -0.5 * t3 + t2 - 0.5 * t,
                1.5 * t3 - 2.5 * t2 + 1.0,
                -1.5 * t3 + 2.0 * t2 + 0.5 * t,
                0.5 * t3 - 0.5 * t2,
            ]
        };
        let (wx, wy) = (wts(tx), wts(ty));
        let mut o = [0f32; 4];
        for (j, wyj) in wy.iter().enumerate() {
            for (i, wxi) in wx.iter().enumerate() {
                let t = self.texel(x0 - 1 + i as i64, y0 - 1 + j as i64);
                let k = wxi * wyj;
                for c in 0..4 {
                    o[c] += t[c] * k;
                }
            }
        }
        o[3] = o[3].clamp(0.0, 1.0);
        for c in 0..3 {
            o[c] = o[c].clamp(0.0, o[3]);
        }
        o
    }
    /// Area-averaging resize.
    pub fn resized(&self, nw: u32, nh: u32) -> PlaceImage {
        let (nw, nh) = (nw.max(1), nh.max(1));
        if (nw, nh) == (self.width, self.height) {
            return self.clone();
        }
        // separable: horizontal then vertical, f32 accumulation
        let (w, h) = (self.width as usize, self.height as usize);
        let src: Vec<f32> = self.data.iter().map(|&v| v as f32 / 65535.0).collect();
        let horiz = resample_x(&src, w, h, nw as usize);
        let vert = resample_axis_f(&horiz, nw as usize, h, nh as usize);
        PlaceImage {
            width: nw,
            height: nh,
            data: vert
                .into_iter()
                .map(|v| (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16)
                .collect(),
        }
    }
    /// Downscales the image so it is not minified (more than 1:1) anywhere on the
    /// sphere when placed with `p` on a document `doc_w` pixels wide.
    pub fn prefiltered_for(&self, p: &Placement, doc_w: u32) -> PlaceImage {
        let (rx, ry) = p.minification(self.width, self.height, doc_w);
        let nw = if rx > 1.0 {
            (self.width as f64 / rx).ceil() as u32
        } else {
            self.width
        };
        let nh = if ry > 1.0 {
            (self.height as f64 / ry).ceil() as u32
        } else {
            self.height
        };
        self.resized(nw.max(2), nh.max(2))
    }
}

/// Box-filter (area) resample along x.
fn resample_x(src: &[f32], w: usize, h: usize, nw: usize) -> Vec<f32> {
    let mut out = vec![0f32; nw * h * 4];
    let scale = w as f64 / nw as f64;
    for ox in 0..nw {
        let a = ox as f64 * scale;
        let b = (ox as f64 + 1.0) * scale;
        let (i0, i1) = (a.floor() as usize, (b.ceil() as usize).min(w));
        let mut taps = Vec::with_capacity(i1 - i0 + 1);
        let mut tot = 0.0;
        for i in i0..i1.max(i0 + 1) {
            let lo = (i as f64).max(a);
            let hi = ((i + 1) as f64).min(b);
            let wgt = if scale <= 1.0 {
                1.0
            } else {
                (hi - lo).max(0.0)
            };
            if wgt > 0.0 {
                taps.push((i.min(w - 1), wgt));
                tot += wgt;
            }
        }
        for y in 0..h {
            let mut acc = [0f64; 4];
            for &(i, wgt) in &taps {
                for c in 0..4 {
                    acc[c] += src[(y * w + i) * 4 + c] as f64 * wgt;
                }
            }
            for c in 0..4 {
                out[(y * nw + ox) * 4 + c] = (acc[c] / tot) as f32;
            }
        }
    }
    out
}

fn resample_axis_f(src: &[f32], w: usize, h: usize, nh: usize) -> Vec<f32> {
    // transpose-free vertical pass
    let mut out = vec![0f32; w * nh * 4];
    let scale = h as f64 / nh as f64;
    for oy in 0..nh {
        let a = oy as f64 * scale;
        let b = (oy as f64 + 1.0) * scale;
        let (j0, j1) = (a.floor() as usize, (b.ceil() as usize).min(h));
        let mut tot = 0.0;
        let mut taps = Vec::new();
        for j in j0..j1.max(j0 + 1) {
            let lo = (j as f64).max(a);
            let hi = ((j + 1) as f64).min(b);
            let wgt = if scale <= 1.0 {
                1.0
            } else {
                (hi - lo).max(0.0)
            };
            if wgt > 0.0 {
                taps.push((j.min(h - 1), wgt));
                tot += wgt;
            }
        }
        for x in 0..w {
            let mut acc = [0f64; 4];
            for &(j, wgt) in &taps {
                for c in 0..4 {
                    acc[c] += src[(j * w + x) * 4 + c] as f64 * wgt;
                }
            }
            for c in 0..4 {
                out[(oy * w + x) * 4 + c] = (acc[c] / tot) as f32;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{dir_from_lonlat, Camera};

    fn close(a: Vec3, b: Vec3, eps: f64) -> bool {
        (a - b).len() < eps
    }

    fn sample_placements() -> Vec<Placement> {
        let mut v = Vec::new();
        for proj in [Projection::Wrap, Projection::Gnomonic] {
            for (lon, lat, rot) in [
                (0.3, 0.1, 0.0),
                (0.0, 0.0, 0.7),        // on the seam
                (6.2, -0.4, 2.0),       // just before the seam
                (1.0, 1.45, 0.3),       // near the north pole
                (2.0, -FRAC_PI_2, 0.0), // exactly at the south pole
            ] {
                let c = dir_from_lonlat(lon, lat);
                let mut p = Placement::new(proj, c, c, 1.2, 1.5, 0.6);
                p.rotation = rot;
                p.clamp();
                v.push(p);
            }
        }
        v
    }

    #[test]
    fn local_dir_roundtrip_both_modes() {
        for p in sample_placements() {
            for &(nx, ny) in &[
                (0.0, 0.0),
                (0.5, -0.3),
                (-0.99, 0.99),
                (1.0, -1.0),
                (0.2, 0.8),
            ] {
                let d = p.dir_at(nx, ny);
                assert!((d.len() - 1.0).abs() < 1e-9);
                let (a, b) = p.local(d).expect("in front");
                assert!(
                    (a - nx).abs() < 1e-7 && (b - ny).abs() < 1e-7,
                    "{:?} ({nx},{ny}) -> ({a},{b})",
                    p.projection
                );
            }
            // centre maps to the origin
            let (a, b) = p.local(p.center).unwrap();
            assert!(a.abs() < 1e-9 && b.abs() < 1e-9);
        }
    }

    #[test]
    fn wrap_patch_is_spherical_coordinates() {
        // Unrotated wrap patch at the equator: local x/y are exactly lon/lat offsets.
        let c = dir_from_lonlat(1.0, 0.0);
        let mut p = Placement::new(Projection::Wrap, c, c, 1.0, 1.0, 1.0);
        p.half_w = 0.4;
        p.half_h = 0.2;
        let d = dir_from_lonlat(1.0 + 0.2, 0.0);
        // Camera::basis uses r = f × up, which points towards decreasing lon here.
        let (a, b) = p.local(d).unwrap();
        assert!((a.abs() - 0.5).abs() < 1e-9 && b.abs() < 1e-9);
        let d = dir_from_lonlat(1.0, 0.1);
        let (a, b) = p.local(d).unwrap();
        assert!(a.abs() < 1e-9 && (b - 0.5).abs() < 1e-9);
    }

    #[test]
    fn gnomonic_is_undistorted_from_its_view() {
        let cam = Camera {
            lon: 2.0,
            lat: 0.5,
            fov: 70.0,
        };
        let (f, _, _) = cam.basis();
        let p = Placement::new(
            Projection::Gnomonic,
            f,
            f,
            70f64.to_radians(),
            4.0 / 3.0,
            0.5,
        );
        let (vw, vh) = (1200.0, 900.0);
        // Points on a regular grid in image space project to a regular, axis-aligned
        // grid on the screen of a camera looking along the projector axis.
        let s = |nx, ny| cam.project_screen(p.dir_at(nx, ny), vw, vh).unwrap();
        let o = s(0.0, 0.0);
        assert!((o.0 - vw / 2.0).abs() < 1e-6 && (o.1 - vh / 2.0).abs() < 1e-6);
        let ex = s(1.0, 0.0).0 - o.0;
        let ey = s(0.0, 1.0).1 - o.1;
        for &(nx, ny) in &[(0.5, 0.5), (-1.0, 1.0), (0.25, -0.75), (1.0, -1.0)] {
            let q = s(nx, ny);
            assert!((q.0 - (o.0 + ex * nx)).abs() < 1e-6, "x linear");
            assert!((q.1 - (o.1 + ey * ny)).abs() < 1e-6, "y linear");
        }
        // aspect is preserved on screen (4:3 image, square pixels)
        assert!(((ex / -ey).abs() - 4.0 / 3.0).abs() < 1e-9);
        // half the vertical FOV
        assert!((-ey / (vh / 2.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn gnomonic_stays_anchored_when_camera_orbits() {
        let v = dir_from_lonlat(0.5, 0.0);
        let p = Placement::new(Projection::Gnomonic, v, v, 1.0, 1.0, 0.5);
        let before: Vec<_> = p.corners().to_vec();
        // the placement does not depend on any camera: same corners regardless
        let q = p;
        for (a, b) in before.iter().zip(q.corners()) {
            assert!(close(*a, b, 1e-12));
        }
        // re-aiming keeps the centre but changes the projector axis
        let mut r = p;
        r.reaim(dir_from_lonlat(0.7, 0.1));
        assert!(close(r.center, p.center, 1e-12));
        assert!(!close(r.view, p.view, 1e-3));
    }

    #[test]
    fn behind_projector_is_rejected() {
        let v = dir_from_lonlat(0.0, 0.0);
        let p = Placement::new(Projection::Gnomonic, v, v, 1.0, 1.0, 0.5);
        assert!(p.local(v * -1.0).is_none());
    }

    #[test]
    fn bounds_cross_the_seam() {
        let (w, h) = (1024, 512);
        let c = dir_from_lonlat(0.0, 0.0);
        for proj in [Projection::Wrap, Projection::Gnomonic] {
            let p = Placement::new(proj, c, c, 1.0, 1.0, 0.5);
            let rs = p.pixel_bounds(w, h);
            assert_eq!(rs.len(), 2, "{proj:?}: {rs:?}");
            assert!(rs.iter().any(|r| r.x == 0));
            assert!(rs.iter().any(|r| r.right() == w));
            let total: u32 = rs.iter().map(|r| r.w).sum();
            assert!(total < w / 2);
            // every pixel inside the image lies inside the bounds
            for y in 0..h {
                for x in 0..w {
                    let d = px_to_dir(x as f64 + 0.5, y as f64 + 0.5, w, h);
                    if let Some((a, b)) = p.local(d) {
                        if a.abs() <= 1.0 && b.abs() <= 1.0 {
                            assert!(
                                rs.iter().any(|r| x >= r.x
                                    && x < r.right()
                                    && y >= r.y
                                    && y < r.bottom()),
                                "{proj:?} pixel {x},{y} outside bounds"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn bounds_cover_the_pole() {
        let (w, h) = (512, 256);
        let c = dir_from_lonlat(1.0, FRAC_PI_2);
        for proj in [Projection::Wrap, Projection::Gnomonic] {
            let p = Placement::new(proj, c, c, 1.0, 1.0, 0.5);
            let rs = p.pixel_bounds(w, h);
            assert_eq!(rs.len(), 1);
            assert_eq!((rs[0].x, rs[0].y, rs[0].w), (0, 0, w));
            assert!(rs[0].h < h / 2);
        }
    }

    #[test]
    fn move_across_seam_and_pole() {
        let c = dir_from_lonlat(6.1, 0.0);
        let mut p = Placement::new(Projection::Wrap, c, c, 1.0, 1.0, 0.3);
        let start = p;
        p.drag_move(&start, dir_from_lonlat(6.1, 0.0), dir_from_lonlat(0.2, 0.0));
        let (lon, _) = crate::math::lonlat_of(p.center);
        assert!((lon - 0.2).abs() < 1e-9);
        // over the pole
        let start = p;
        p.drag_move(&start, start.center, dir_from_lonlat(0.2 + PI, 1.4));
        assert!(close(p.center, dir_from_lonlat(0.2 + PI, 1.4), 1e-9));
    }

    #[test]
    fn image_roundtrip_and_resize() {
        let px: Vec<u8> = (0..16)
            .flat_map(|i| [i * 10, 255 - i * 10, 7, if i % 2 == 0 { 255 } else { 128 }])
            .collect();
        let img = PlaceImage::from_rgba8(4, 4, &px);
        let back = img.to_rgba8();
        for (a, b) in px.iter().zip(&back) {
            assert!((*a as i32 - *b as i32).abs() <= 1);
        }
        let small = img.resized(2, 2);
        assert_eq!((small.width, small.height), (2, 2));
        let solid = PlaceImage::from_rgba8(8, 8, &[200u8, 100, 50, 255].repeat(64));
        let s2 = solid.resized(3, 5);
        let c = s2.sample_bicubic(1.5, 2.5);
        assert!((c[0] - 200.0 / 255.0).abs() < 1e-3 && (c[3] - 1.0).abs() < 1e-4);
    }
}
