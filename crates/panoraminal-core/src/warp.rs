//! Point-based distortion of a placed image: a corner-pin homography (perspective)
//! plus an optional mesh warp on top of it.
//!
//! Coordinates are the normalized image-frame coordinates of
//! [`crate::place::Frame`] (`[-1, 1]²`, +y up). The chain from image to sphere is
//!
//! ```text
//! image s ──mesh (bilinear cells)──▶ t ──homography H──▶ frame-local l ──Frame──▶ sphere
//! ```
//!
//! With the default (identity) distortion `l = t = s`, so placements without a
//! distortion behave exactly as before.

use serde::{Deserialize, Serialize};

/// Control points per side of the warp mesh (3×3 cells).
pub const MESH_N: usize = 4;
/// Number of mesh control points.
pub const MESH_LEN: usize = MESH_N * MESH_N;

/// The unit square corners in pin order: TL, TR, BR, BL.
pub const SQUARE: [[f64; 2]; 4] = [[-1.0, 1.0], [1.0, 1.0], [1.0, -1.0], [-1.0, -1.0]];

/// A projective transform of the plane (row-major 3×3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography(pub [f64; 9]);

impl Homography {
    pub const IDENTITY: Homography = Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    /// The homography mapping `src[i]` to `dst[i]` (direct linear solve), or `None`
    /// when the points are degenerate (three collinear, coincident…).
    pub fn from_points(src: &[[f64; 2]; 4], dst: &[[f64; 2]; 4]) -> Option<Homography> {
        // Unknowns h0..h7 (h8 = 1).
        let mut a = [[0f64; 9]; 8];
        for i in 0..4 {
            let [x, y] = src[i];
            let [u, v] = dst[i];
            a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
            a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
        }
        // Gaussian elimination with partial pivoting.
        for c in 0..8 {
            let p = (c..8).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
            if a[p][c].abs() < 1e-12 {
                return None;
            }
            a.swap(c, p);
            let pivot = a[c];
            for (r, row) in a.iter_mut().enumerate() {
                if r != c {
                    let k = row[c] / pivot[c];
                    if k != 0.0 {
                        for (x, pv) in row.iter_mut().zip(pivot.iter()).skip(c) {
                            *x -= k * pv;
                        }
                    }
                }
            }
        }
        let mut h = [0f64; 9];
        for i in 0..8 {
            h[i] = a[i][8] / a[i][i];
        }
        h[8] = 1.0;
        let hh = Homography(h);
        (hh.det().abs() > 1e-12 && h.iter().all(|v| v.is_finite())).then_some(hh)
    }

    /// Homography from the unit square [`SQUARE`] to the quad `q`.
    pub fn square_to_quad(q: &[[f64; 2]; 4]) -> Option<Homography> {
        Self::from_points(&SQUARE, q)
    }

    pub fn det(&self) -> f64 {
        let m = &self.0;
        m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
            + m[2] * (m[3] * m[7] - m[4] * m[6])
    }

    pub fn inverse(&self) -> Option<Homography> {
        let m = &self.0;
        let d = self.det();
        if d.abs() < 1e-15 || !d.is_finite() {
            return None;
        }
        let i = 1.0 / d;
        Some(Homography([
            (m[4] * m[8] - m[5] * m[7]) * i,
            (m[2] * m[7] - m[1] * m[8]) * i,
            (m[1] * m[5] - m[2] * m[4]) * i,
            (m[5] * m[6] - m[3] * m[8]) * i,
            (m[0] * m[8] - m[2] * m[6]) * i,
            (m[2] * m[3] - m[0] * m[5]) * i,
            (m[3] * m[7] - m[4] * m[6]) * i,
            (m[1] * m[6] - m[0] * m[7]) * i,
            (m[0] * m[4] - m[1] * m[3]) * i,
        ]))
    }

    /// Applies the transform; `None` on (or numerically at) the line at infinity.
    #[inline]
    pub fn apply(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let m = &self.0;
        let w = m[6] * x + m[7] * y + m[8];
        if w.abs() < 1e-12 {
            return None;
        }
        Some((
            (m[0] * x + m[1] * y + m[2]) / w,
            (m[3] * x + m[4] * y + m[5]) / w,
        ))
    }
}

/// Signed area of a polygon (positive = counter-clockwise with +y up).
fn signed_area(q: &[[f64; 2]]) -> f64 {
    let n = q.len();
    (0..n)
        .map(|i| {
            let a = q[i];
            let b = q[(i + 1) % n];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        * 0.5
}

/// Why a quad cannot be used as a corner pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    /// Zero area / coincident or collinear points.
    Degenerate,
    /// Self-intersecting ("bow tie") or concave.
    Concave,
}

/// Checks that `q` is a strictly convex, non-degenerate quad (either orientation).
/// `min_area` is relative to the default square (area 4).
pub fn check_quad(q: &[[f64; 2]; 4], min_area: f64) -> Result<(), Invalid> {
    if q.iter().flatten().any(|v| !v.is_finite()) {
        return Err(Invalid::Degenerate);
    }
    let area = signed_area(q);
    if area.abs() < min_area {
        return Err(Invalid::Degenerate);
    }
    let mut sign = 0.0;
    for i in 0..4 {
        let a = q[i];
        let b = q[(i + 1) % 4];
        let c = q[(i + 2) % 4];
        let e1 = [b[0] - a[0], b[1] - a[1]];
        let e2 = [c[0] - b[0], c[1] - b[1]];
        let l1 = e1[0].hypot(e1[1]);
        let l2 = e2[0].hypot(e2[1]);
        if l1 < 1e-6 || l2 < 1e-6 {
            return Err(Invalid::Degenerate);
        }
        let cr = (e1[0] * e2[1] - e1[1] * e2[0]) / (l1 * l2);
        if cr.abs() < 1e-3 {
            return Err(Invalid::Degenerate);
        }
        if sign == 0.0 {
            sign = cr.signum();
        } else if cr.signum() != sign {
            return Err(Invalid::Concave);
        }
    }
    if area.signum() != sign {
        return Err(Invalid::Concave);
    }
    Ok(())
}

#[inline]
fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

/// Inverse bilinear interpolation: `(u, v)` with
/// `p = a + (b-a)u + (d-a)v + (a-b+c-d)uv` for the quad `a b c d`
/// (a = (0,0), b = (1,0), c = (1,1), d = (0,1)). `None` when outside.
pub fn inv_bilinear(
    p: [f64; 2],
    a: [f64; 2],
    b: [f64; 2],
    c: [f64; 2],
    d: [f64; 2],
) -> Option<(f64, f64)> {
    const EPS: f64 = 1e-9;
    let e = [b[0] - a[0], b[1] - a[1]];
    let f = [d[0] - a[0], d[1] - a[1]];
    let g = [a[0] - b[0] + c[0] - d[0], a[1] - b[1] + c[1] - d[1]];
    let h = [p[0] - a[0], p[1] - a[1]];
    let k2 = cross(g, f);
    let k1 = cross(e, f) + cross(h, g);
    let k0 = cross(h, e);
    let solve_u = |v: f64| -> Option<f64> {
        let dx = e[0] + g[0] * v;
        let dy = e[1] + g[1] * v;
        if dx.abs() >= dy.abs() {
            (dx.abs() > 1e-14).then(|| (h[0] - f[0] * v) / dx)
        } else {
            Some((h[1] - f[1] * v) / dy)
        }
    };
    let ok = |t: f64| (-EPS..=1.0 + EPS).contains(&t);
    let scale = k1.abs().max(k0.abs()).max(1e-300);
    if k2.abs() < 1e-10 * scale {
        if k1.abs() < 1e-300 {
            return None;
        }
        let v = -k0 / k1;
        let u = solve_u(v)?;
        return (ok(u) && ok(v)).then_some((u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)));
    }
    let w = k1 * k1 - 4.0 * k0 * k2;
    if w < 0.0 {
        return None;
    }
    let w = w.sqrt();
    // Numerically stable roots.
    let q = -0.5 * (k1 + k1.signum() * w);
    let roots = [q / k2, if q.abs() > 1e-300 { k0 / q } else { f64::NAN }];
    for v in roots {
        if v.is_finite() && ok(v) {
            if let Some(u) = solve_u(v) {
                if ok(u) {
                    return Some((u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)));
                }
            }
        }
    }
    None
}

/// Corner pin + mesh warp of a placed image (part of [`crate::place::Placement`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Distortion {
    /// Where the image corners (TL, TR, BR, BL) land in frame-local coordinates.
    pub corners: [[f64; 2]; 4],
    /// Mesh control points (row-major, row 0 = top) in the corner-pin square.
    pub mesh: [[f64; 2]; MESH_LEN],
}

impl Default for Distortion {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Rest position of mesh point `(i, j)` (column, row from the top).
pub const fn mesh_rest(i: usize, j: usize) -> [f64; 2] {
    let s = (MESH_N - 1) as f64;
    [-1.0 + 2.0 * i as f64 / s, 1.0 - 2.0 * j as f64 / s]
}

const fn rest_mesh() -> [[f64; 2]; MESH_LEN] {
    let mut m = [[0.0; 2]; MESH_LEN];
    let mut k = 0;
    while k < MESH_LEN {
        m[k] = mesh_rest(k % MESH_N, k / MESH_N);
        k += 1;
    }
    m
}

impl Distortion {
    pub const IDENTITY: Distortion = Distortion {
        corners: SQUARE,
        mesh: rest_mesh(),
    };

    pub fn is_identity(&self) -> bool {
        self.perspective_is_identity() && !self.has_mesh()
    }
    pub fn perspective_is_identity(&self) -> bool {
        close(&self.corners, &SQUARE)
    }
    pub fn has_mesh(&self) -> bool {
        !close(&self.mesh, &rest_mesh())
    }
    pub fn reset_mesh(&mut self) {
        self.mesh = rest_mesh();
    }

    /// The square → corner-quad homography.
    pub fn homography(&self) -> Option<Homography> {
        if self.perspective_is_identity() {
            return Some(Homography::IDENTITY);
        }
        Homography::square_to_quad(&self.corners)
    }

    /// Checks the corner quad and every mesh cell.
    pub fn validate(&self) -> Result<(), Invalid> {
        check_quad(&self.corners, 1e-4)?;
        if self.has_mesh() {
            let mut sign = 0.0;
            for cj in 0..MESH_N - 1 {
                for ci in 0..MESH_N - 1 {
                    let q = self.cell(ci, cj);
                    check_quad(&q, 1e-6)?;
                    let s = signed_area(&q).signum();
                    if sign == 0.0 {
                        sign = s;
                    } else if s != sign {
                        return Err(Invalid::Concave);
                    }
                }
            }
        }
        self.homography().map(|_| ()).ok_or(Invalid::Degenerate)
    }

    /// Mesh cell `(ci, cj)` as a quad TL, TR, BR, BL.
    pub fn cell(&self, ci: usize, cj: usize) -> [[f64; 2]; 4] {
        let p = |i: usize, j: usize| self.mesh[j * MESH_N + i];
        [p(ci, cj), p(ci + 1, cj), p(ci + 1, cj + 1), p(ci, cj + 1)]
    }

    /// Image coordinates → corner-pin square (mesh warp only).
    pub fn mesh_forward(&self, sx: f64, sy: f64) -> (f64, f64) {
        if !self.has_mesh() {
            return (sx, sy);
        }
        let n = (MESH_N - 1) as f64;
        let gx = ((sx + 1.0) * 0.5 * n).clamp(0.0, n);
        let gy = ((1.0 - sy) * 0.5 * n).clamp(0.0, n);
        let ci = (gx.floor() as usize).min(MESH_N - 2);
        let cj = (gy.floor() as usize).min(MESH_N - 2);
        let (u, v) = (gx - ci as f64, gy - cj as f64);
        let [a, b, c, d] = self.cell(ci, cj);
        let x = a[0] * (1.0 - u) * (1.0 - v)
            + b[0] * u * (1.0 - v)
            + c[0] * u * v
            + d[0] * (1.0 - u) * v;
        let y = a[1] * (1.0 - u) * (1.0 - v)
            + b[1] * u * (1.0 - v)
            + c[1] * u * v
            + d[1] * (1.0 - u) * v;
        (x, y)
    }

    /// Corner-pin square → image coordinates (inverse mesh warp); `None` outside
    /// the mesh.
    pub fn mesh_inverse(&self, tx: f64, ty: f64) -> Option<(f64, f64)> {
        if !self.has_mesh() {
            return Some((tx, ty));
        }
        let n = (MESH_N - 1) as f64;
        for cj in 0..MESH_N - 1 {
            for ci in 0..MESH_N - 1 {
                let [a, b, c, d] = self.cell(ci, cj);
                if let Some((u, v)) = inv_bilinear([tx, ty], a, b, c, d) {
                    let gx = ci as f64 + u;
                    let gy = cj as f64 + v;
                    return Some((gx / n * 2.0 - 1.0, 1.0 - gy / n * 2.0));
                }
            }
        }
        None
    }

    /// Image coordinates → frame-local coordinates.
    pub fn forward_with(&self, h: &Homography, sx: f64, sy: f64) -> Option<(f64, f64)> {
        let (tx, ty) = self.mesh_forward(sx, sy);
        h.apply(tx, ty)
    }

    /// Frame-local coordinates → image coordinates (`hinv` = inverse homography).
    /// Results outside `[-1, 1]²` are outside the image.
    pub fn inverse_with(&self, hinv: &Homography, lx: f64, ly: f64) -> Option<(f64, f64)> {
        let (tx, ty) = hinv.apply(lx, ly)?;
        if self.has_mesh() {
            // Outside the corner-pin square the mesh never reaches: cheap reject only
            // when far, as dragged mesh points may leave the square.
            self.mesh_inverse(tx, ty)
        } else {
            Some((tx, ty))
        }
    }

    pub fn forward(&self, sx: f64, sy: f64) -> Option<(f64, f64)> {
        self.forward_with(&self.homography()?, sx, sy)
    }
    pub fn inverse(&self, lx: f64, ly: f64) -> Option<(f64, f64)> {
        self.inverse_with(&self.homography()?.inverse()?, lx, ly)
    }
}

fn close<const N: usize>(a: &[[f64; 2]; N], b: &[[f64; 2]; N]) -> bool {
    a.iter()
        .zip(b.iter())
        .all(|(p, q)| (p[0] - q[0]).abs() < 1e-12 && (p[1] - q[1]).abs() < 1e-12)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> [[f64; 2]; 4] {
        [[-0.7, 0.9], [1.2, 0.5], [0.9, -0.4], [-0.8, -1.1]]
    }

    #[test]
    fn unit_square_is_identity() {
        let h = Homography::square_to_quad(&SQUARE).unwrap();
        for (a, b) in h.0.iter().zip(Homography::IDENTITY.0.iter()) {
            assert!((a - b).abs() < 1e-12, "{h:?}");
        }
        let d = Distortion::IDENTITY;
        assert!(d.is_identity());
        assert_eq!(d.forward(0.3, -0.2), Some((0.3, -0.2)));
        assert_eq!(d.inverse(0.3, -0.2), Some((0.3, -0.2)));
    }

    #[test]
    fn homography_solve_and_inverse_roundtrip() {
        let q = quad();
        let h = Homography::square_to_quad(&q).unwrap();
        for (s, t) in SQUARE.iter().zip(q.iter()) {
            let (x, y) = h.apply(s[0], s[1]).unwrap();
            assert!((x - t[0]).abs() < 1e-9 && (y - t[1]).abs() < 1e-9);
        }
        let hi = h.inverse().unwrap();
        for k in 0..50 {
            let x = (k as f64 * 0.37).sin();
            let y = (k as f64 * 0.73).cos();
            let (a, b) = h.apply(x, y).unwrap();
            let (c, d) = hi.apply(a, b).unwrap();
            assert!((c - x).abs() < 1e-9 && (d - y).abs() < 1e-9, "{k}");
        }
        // Straight lines stay straight: midpoint of an edge lies on the mapped edge.
        let (mx, my) = h.apply(0.0, 1.0).unwrap();
        let (a, b) = (q[0], q[1]);
        let c = (b[0] - a[0]) * (my - a[1]) - (b[1] - a[1]) * (mx - a[0]);
        assert!(c.abs() < 1e-9);
    }

    #[test]
    fn degenerate_and_concave_detection() {
        assert_eq!(check_quad(&SQUARE, 1e-4), Ok(()));
        // Mirrored square is still a valid pin.
        let m = [SQUARE[1], SQUARE[0], SQUARE[3], SQUARE[2]];
        assert_eq!(check_quad(&m, 1e-4), Ok(()));
        // Bow tie.
        let bow = [SQUARE[0], SQUARE[1], SQUARE[3], SQUARE[2]];
        assert!(check_quad(&bow, 1e-4).is_err());
        // Concave (dart).
        let dart = [[-1.0, 1.0], [1.0, 1.0], [-0.5, 0.5], [-1.0, -1.0]];
        assert_eq!(check_quad(&dart, 1e-4), Err(Invalid::Concave));
        // Collinear / coincident.
        let line = [[-1.0, 0.0], [0.0, 0.0], [1.0, 0.0], [0.5, 0.0]];
        assert_eq!(check_quad(&line, 1e-4), Err(Invalid::Degenerate));
        let same = [[0.0, 0.0]; 4];
        assert!(check_quad(&same, 1e-4).is_err());
        assert!(Homography::square_to_quad(&same).is_none());
        let mut d = Distortion::IDENTITY;
        d.corners = bow;
        assert!(d.validate().is_err());
        let mut d = Distortion::IDENTITY;
        // Fold a mesh cell.
        d.mesh[5] = [1.5, -1.5];
        assert!(d.validate().is_err());
    }

    #[test]
    fn mesh_roundtrip() {
        let mut d = Distortion::IDENTITY;
        d.corners = quad();
        d.mesh[5][0] += 0.15;
        d.mesh[5][1] -= 0.1;
        d.mesh[10][1] += 0.2;
        d.mesh[0] = [-1.1, 1.05];
        assert!(d.validate().is_ok());
        assert!(d.has_mesh());
        for k in 0..200 {
            let x = (k as f64 * 0.137).sin() * 0.999;
            let y = (k as f64 * 0.311).cos() * 0.999;
            let (a, b) = d.forward(x, y).unwrap();
            let (c, e) = d.inverse(a, b).unwrap();
            assert!(
                (c - x).abs() < 1e-7 && (e - y).abs() < 1e-7,
                "{k} {x} {y} {c} {e}"
            );
        }
        // Mesh points land where they were dragged.
        let h = d.homography().unwrap();
        let [rx, ry] = mesh_rest(1, 1);
        let l = d.forward(rx, ry).unwrap();
        let t = h.apply(d.mesh[5][0], d.mesh[5][1]).unwrap();
        assert!((l.0 - t.0).abs() < 1e-9 && (l.1 - t.1).abs() < 1e-9);
    }
}
