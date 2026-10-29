//! Equirectangular / spherical math shared by the brush engine, the renderer and the UI.
//!
//! Conventions (identical to the WGSL display shader):
//! * A direction `d` on the unit sphere has longitude `lon = atan2(d.z, d.x)` and
//!   latitude `lat = asin(d.y)` (up is +Y).
//! * Image coordinates: `u = lon / 2π` (wrapped to `[0,1)`), `v = 0.5 − lat/π`
//!   (0 at the zenith, 1 at the nadir). Pixel = `(u·W, v·H)`.
//! * The camera sits at the sphere centre, looking at `dir(lon, lat)`; the initial
//!   view (`lon = π`) looks at the centre of the image.

use std::f64::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const UP: Vec3 = Vec3 {
        x: 0.0,
        y: 1.0,
        z: 0.0,
    };
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    pub fn len(self) -> f64 {
        self.dot(self).sqrt()
    }
    pub fn normalize(self) -> Vec3 {
        let l = self.len();
        if l < 1e-12 {
            Vec3::new(1.0, 0.0, 0.0)
        } else {
            self * (1.0 / l)
        }
    }
    pub fn lerp(self, o: Vec3, t: f64) -> Vec3 {
        self + (o - self) * t
    }
}
impl std::ops::Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl std::ops::Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl std::ops::Mul<f64> for Vec3 {
    type Output = Vec3;
    fn mul(self, k: f64) -> Vec3 {
        Vec3::new(self.x * k, self.y * k, self.z * k)
    }
}

/// Unit direction for a longitude/latitude pair (radians).
pub fn dir_from_lonlat(lon: f64, lat: f64) -> Vec3 {
    let c = lat.cos();
    Vec3::new(c * lon.cos(), lat.sin(), c * lon.sin())
}

/// Longitude in `[0, 2π)` and latitude in `[-π/2, π/2]` of a direction.
pub fn lonlat_of(d: Vec3) -> (f64, f64) {
    let lat = d.y.clamp(-1.0, 1.0).asin();
    let mut lon = d.z.atan2(d.x);
    if lon < 0.0 {
        lon += TAU;
    }
    (lon, lat)
}

/// A point in document pixel space plus its spherical coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelPos {
    pub x: f64,
    pub y: f64,
    pub u: f64,
    pub lat: f64,
}

/// Direction → document pixel (x wraps, y in `[0,H]`).
pub fn dir_to_px(d: Vec3, w: u32, h: u32) -> PixelPos {
    let (lon, lat) = lonlat_of(d);
    let mut u = lon / TAU;
    u -= u.floor();
    PixelPos {
        x: u * w as f64,
        y: (0.5 - lat / PI) * h as f64,
        u,
        lat,
    }
}

/// Document pixel → direction (inverse of [`dir_to_px`]).
pub fn px_to_dir(x: f64, y: f64, w: u32, h: u32) -> Vec3 {
    dir_from_lonlat(x / w as f64 * TAU, (0.5 - y / h as f64) * PI)
}

/// Spherical linear interpolation between unit vectors.
pub fn slerp(a: Vec3, b: Vec3, t: f64) -> Vec3 {
    let d = a.dot(b).clamp(-1.0, 1.0);
    let ang = d.acos();
    if ang < 1e-9 {
        return a;
    }
    let s = ang.sin();
    if s.abs() < 1e-9 {
        // Antipodal: no unique great circle, fall back to a lerp through an orthogonal axis.
        return a.lerp(b, t).normalize();
    }
    let wa = ((1.0 - t) * ang).sin() / s;
    let wb = (t * ang).sin() / s;
    (a * wa + b * wb).normalize()
}

/// Great-circle angle between unit vectors.
pub fn angle_between(a: Vec3, b: Vec3) -> f64 {
    a.dot(b).clamp(-1.0, 1.0).acos()
}

/// The 360° camera: a pinhole at the sphere centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Longitude the camera looks at (radians).
    pub lon: f64,
    /// Latitude the camera looks at (radians).
    pub lat: f64,
    /// Vertical field of view (degrees).
    pub fov: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            lon: PI,
            lat: 0.0,
            fov: 75.0,
        }
    }
}

impl Camera {
    pub const MIN_FOV: f64 = 12.0;
    pub const MAX_FOV: f64 = 120.0;
    const LAT_LIMIT: f64 = PI / 2.0 - 0.01;

    pub fn clamp(&mut self) {
        self.lat = self.lat.clamp(-Self::LAT_LIMIT, Self::LAT_LIMIT);
        self.fov = self.fov.clamp(Self::MIN_FOV, Self::MAX_FOV);
        self.lon = self.lon.rem_euclid(TAU);
    }
    /// Orthonormal basis `(forward, right, up)`.
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let f = dir_from_lonlat(self.lon, self.lat);
        let r = f.cross(Vec3::UP).normalize();
        let u = r.cross(f).normalize();
        (f, r, u)
    }
    /// Ray direction for normalized device coords (`nx`,`ny` in `[-1,1]`, `ny` up).
    pub fn ray(&self, nx: f64, ny: f64, aspect: f64) -> Vec3 {
        let (f, r, u) = self.basis();
        let ty = (self.fov.to_radians() * 0.5).tan();
        let tx = ty * aspect;
        (f + r * (nx * tx) + u * (ny * ty)).normalize()
    }
    /// Ray for a screen position in a viewport of `vw × vh` (y down).
    pub fn ray_screen(&self, sx: f64, sy: f64, vw: f64, vh: f64) -> Vec3 {
        let nx = sx / vw * 2.0 - 1.0;
        let ny = -(sy / vh * 2.0 - 1.0);
        self.ray(nx, ny, vw / vh.max(1.0))
    }
    /// Projects a direction to screen coords; `None` if behind the camera.
    pub fn project_screen(&self, d: Vec3, vw: f64, vh: f64) -> Option<(f64, f64)> {
        let (f, r, u) = self.basis();
        let z = d.dot(f);
        if z <= 1e-6 {
            return None;
        }
        let ty = (self.fov.to_radians() * 0.5).tan();
        let tx = ty * vw / vh.max(1.0);
        let nx = d.dot(r) / z / tx;
        let ny = d.dot(u) / z / ty;
        Some(((nx + 1.0) * 0.5 * vw, (1.0 - ny) * 0.5 * vh))
    }
    /// Rotates the view by a screen-space drag of (`dx`,`dy`) pixels ("grab" semantics).
    pub fn drag(&mut self, dx: f64, dy: f64, vh: f64) {
        let k = self.fov.to_radians() / vh.max(1.0);
        self.lon -= dx * k;
        self.lat += dy * k;
        self.clamp();
    }
    pub fn zoom(&mut self, factor: f64) {
        self.fov *= factor;
        self.clamp();
    }
}

/// The flat (2D equirectangular) view: the image is shown with a width of
/// `vw * zoom` screen pixels; `(cx, cy)` is the image UV at the viewport centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlatView {
    pub cx: f64,
    pub cy: f64,
    pub zoom: f64,
}

impl Default for FlatView {
    fn default() -> Self {
        Self {
            cx: 0.5,
            cy: 0.5,
            zoom: 0.92,
        }
    }
}

impl FlatView {
    pub const MIN_ZOOM: f64 = 0.2;
    pub const MAX_ZOOM: f64 = 32.0;
    /// Screen (viewport-local) → image UV. `u` is *not* wrapped.
    pub fn screen_to_uv(&self, sx: f64, sy: f64, vw: f64, vh: f64) -> (f64, f64) {
        let iw = vw * self.zoom;
        (
            self.cx + (sx - vw * 0.5) / iw,
            self.cy + (sy - vh * 0.5) / (iw * 0.5),
        )
    }
    pub fn uv_to_screen(&self, u: f64, v: f64, vw: f64, vh: f64) -> (f64, f64) {
        let iw = vw * self.zoom;
        (
            vw * 0.5 + (u - self.cx) * iw,
            vh * 0.5 + (v - self.cy) * iw * 0.5,
        )
    }
    /// Screen px per image px.
    pub fn scale(&self, vw: f64, img_w: u32) -> f64 {
        vw * self.zoom / img_w as f64
    }
    pub fn pan(&mut self, dx: f64, dy: f64, vw: f64) {
        let iw = vw * self.zoom;
        self.cx = (self.cx - dx / iw).rem_euclid(1.0);
        self.cy = (self.cy - dy / (iw * 0.5)).clamp(-0.5, 1.5);
    }
    /// Zooms keeping the image point under (`sx`,`sy`) fixed.
    pub fn zoom_at(&mut self, factor: f64, sx: f64, sy: f64, vw: f64, vh: f64) {
        let (u, v) = self.screen_to_uv(sx, sy, vw, vh);
        self.zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let iw = vw * self.zoom;
        self.cx = u - (sx - vw * 0.5) / iw;
        self.cy = v - (sy - vh * 0.5) / (iw * 0.5);
    }
    /// Fits the whole panorama in the viewport.
    pub fn fit(vw: f64, vh: f64) -> Self {
        let zoom = (0.94f64).min(0.94 * 2.0 * vh / vw.max(1.0));
        Self {
            cx: 0.5,
            cy: 0.5,
            zoom,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(a: f64, b: f64, e: f64) -> bool {
        (a - b).abs() < e
    }
    #[test]
    fn roundtrip_px_dir() {
        for &(x, y) in &[
            (0.5, 512.0),
            (1024.0, 512.0),
            (2047.0, 3.0),
            (100.0, 1000.0),
        ] {
            let d = px_to_dir(x, y, 2048, 1024);
            let p = dir_to_px(d, 2048, 1024);
            assert!(
                close(p.x, x, 1e-6) && close(p.y, y, 1e-6),
                "{x},{y} -> {p:?}"
            );
        }
    }
    #[test]
    fn camera_centre_is_image_centre() {
        let c = Camera::default();
        let d = c.ray_screen(400.0, 300.0, 800.0, 600.0);
        let p = dir_to_px(d, 2048, 1024);
        assert!(close(p.x, 1024.0, 1e-6) && close(p.y, 512.0, 1e-6));
    }
    #[test]
    fn not_mirrored_right_is_increasing_u() {
        let c = Camera::default();
        let a = dir_to_px(c.ray_screen(400.0, 300.0, 800.0, 600.0), 2048, 1024);
        let b = dir_to_px(c.ray_screen(600.0, 300.0, 800.0, 600.0), 2048, 1024);
        assert!(b.x > a.x);
        let up = dir_to_px(c.ray_screen(400.0, 100.0, 800.0, 600.0), 2048, 1024);
        assert!(up.y < a.y, "screen up must be image up");
    }
    #[test]
    fn project_inverts_ray() {
        let c = Camera {
            lon: 1.0,
            lat: 0.4,
            fov: 80.0,
        };
        let d = c.ray_screen(123.0, 456.0, 1000.0, 700.0);
        let (x, y) = c.project_screen(d, 1000.0, 700.0).unwrap();
        assert!(close(x, 123.0, 1e-6) && close(y, 456.0, 1e-6));
    }
    #[test]
    fn slerp_midpoint_on_great_circle() {
        let a = dir_from_lonlat(0.0, 0.0);
        let b = dir_from_lonlat(PI / 2.0, 0.0);
        let m = slerp(a, b, 0.5);
        assert!(close(m.len(), 1.0, 1e-9));
        assert!(close(lonlat_of(m).0, PI / 4.0, 1e-9));
    }
    #[test]
    fn camera_clamps() {
        let mut c = Camera {
            lon: -1.0,
            lat: 3.0,
            fov: 500.0,
        };
        c.clamp();
        assert!(c.lat < PI / 2.0 && c.fov == Camera::MAX_FOV && c.lon >= 0.0);
    }

    #[test]
    fn flat_view_roundtrip_and_zoom_anchor() {
        let mut f = FlatView::default();
        let (u, v) = f.screen_to_uv(300.0, 120.0, 800.0, 600.0);
        let (x, y) = f.uv_to_screen(u, v, 800.0, 600.0);
        assert!((x - 300.0).abs() < 1e-9 && (y - 120.0).abs() < 1e-9);
        f.zoom_at(2.0, 300.0, 120.0, 800.0, 600.0);
        let (u2, v2) = f.screen_to_uv(300.0, 120.0, 800.0, 600.0);
        assert!((u - u2).abs() < 1e-9 && (v - v2).abs() < 1e-9);
        let c = FlatView::fit(800.0, 600.0);
        assert_eq!(c.screen_to_uv(400.0, 300.0, 800.0, 600.0), (0.5, 0.5));
    }
}
