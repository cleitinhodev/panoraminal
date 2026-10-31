//! A procedurally generated demo panorama (used by `--demo`, the README
//! screenshots and as a quick way to try the app).

use panoraminal_core::document::Background;
use panoraminal_core::math::dir_from_lonlat;
use panoraminal_core::{BlendMode, BrushSettings, Document, StrokeEngine, ToolKind};
use panoraminal_i18n::tr;
use std::f64::consts::{PI, TAU};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn ridge(x: f64, seed: f64) -> f64 {
    // Integer frequencies keep the silhouette seamless at 0°/360°.
    let ks = [
        (2.0, 0.30),
        (3.0, 0.22),
        (5.0, 0.16),
        (7.0, 0.11),
        (11.0, 0.07),
        (17.0, 0.05),
        (29.0, 0.025),
    ];
    ks.iter()
        .enumerate()
        .map(|(i, (k, a))| a * (TAU * k * x + seed * (i as f64 + 1.3)).sin())
        .sum::<f64>()
}

pub fn build(w: u32, h: u32) -> Document {
    let mut d = Document::new_blank(
        w,
        h,
        Background::Night,
        tr("layer.background"),
        tr("demo.layer.mountains"),
    );
    let horizon = h as f64 * 0.5;
    // Stars on the background layer.
    {
        let bg = &mut d.layers[0].pixels;
        let mut rng = Lcg(42);
        for _ in 0..(w * h / 900) {
            let x = (rng.next() * w as f64) as u32;
            let y = (rng.next() * horizon * 0.9) as u32;
            let b = (150.0 + rng.next() * 105.0) as u8;
            let i = ((y * w + x) * 4) as usize;
            bg[i..i + 4].copy_from_slice(&[b, b, 255, 255]);
            if rng.next() > 0.85 && x + 1 < w && y + 1 < h {
                for (dx, dy) in [(1, 0), (0, 1)] {
                    let j = (((y + dy) * w + x + dx) * 4) as usize;
                    bg[j..j + 4].copy_from_slice(&[b / 2, b / 2, 220, 255]);
                }
            }
        }
    }
    // Mountains + lake reflection, with anti-aliased silhouettes.
    {
        let m = &mut d.layers[1].pixels;
        let hf = h as f64;
        let far_at = |u: f64| horizon - (0.11 + 0.07 * ridge(u, 1.0)) * hf;
        let near_at = |u: f64| horizon - (0.06 + 0.05 * ridge(u, 4.0)).max(0.01) * hf;
        let far_c = [2.0, 48.0, 160.0];
        let near_c = [4.0, 22.0, 70.0];
        let water_c = [3.0, 12.0, 40.0];
        let mix = |a: [f64; 3], b: [f64; 3], t: f64| {
            [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ]
        };
        for x in 0..w {
            let u = x as f64 / w as f64;
            let far = far_at(u);
            let near = near_at(u);
            for y in 0..h {
                let yf = y as f64 + 0.5;
                let i = ((y * w + x) * 4) as usize;
                if yf < horizon {
                    // Coverage of each silhouette at this pixel (1 px soft edge).
                    let cf = (yf - far + 0.5).clamp(0.0, 1.0);
                    let cn = (yf - near + 0.5).clamp(0.0, 1.0);
                    if cf <= 0.0 {
                        continue;
                    }
                    let tf = ((yf - far) / (horizon - far)).clamp(0.0, 1.0);
                    let tn = ((yf - near) / (horizon - near)).clamp(0.0, 1.0);
                    let fc = [
                        far_c[0] + 9.0 * tf,
                        far_c[1] + 60.0 * tf,
                        far_c[2] + 70.0 * tf,
                    ];
                    let nc = [
                        near_c[0] + 6.0 * tn,
                        near_c[1] + 30.0 * tn,
                        near_c[2] + 60.0 * tn,
                    ];
                    let c = mix(fc, nc, cn);
                    m[i..i + 4].copy_from_slice(&[
                        c[0] as u8,
                        c[1] as u8,
                        c[2] as u8,
                        (255.0 * cf) as u8,
                    ]);
                } else {
                    // Water: mirrored mountains, gently wobbling sideways and
                    // fading with depth.
                    let depth = (yf - horizon) / (hf - horizon);
                    let wobble = 0.0012 * (1.0 - depth) * (yf * 0.23 + u * 31.0).sin();
                    let ur = (u + wobble).rem_euclid(1.0);
                    let my = 2.0 * horizon - yf;
                    let cf = (my - far_at(ur) + 0.5).clamp(0.0, 1.0);
                    let cn = (my - near_at(ur) + 0.5).clamp(0.0, 1.0);
                    let refl = mix(mix(water_c, [2.0, 40.0, 128.0], cf), [3.0, 20.0, 62.0], cn);
                    // Reflections dim towards the nadir.
                    let k = (1.0 - depth * 1.6).clamp(0.0, 1.0);
                    let c = mix(water_c, refl, k);
                    let a = 235.0 - 60.0 * depth;
                    m[i..i + 4].copy_from_slice(&[c[0] as u8, c[1] as u8, c[2] as u8, a as u8]);
                }
            }
        }
    }
    // Neon strokes painted with the real brush engine.
    let aurora = d.new_layer(tr("demo.layer.aurora"));
    let aid = aurora.id;
    d.layers.push(aurora);
    let neon = d.new_layer(tr("demo.layer.neon"));
    let nid = neon.id;
    d.layers.push(neon);
    if let Some(l) = d.layer_mut(aid) {
        l.blend = BlendMode::Screen;
        l.opacity = 0.9;
    }
    if let Some(l) = d.layer_mut(nid) {
        l.blend = BlendMode::Add;
    }
    let k = w as f32 / 4096.0;
    let mut eng = StrokeEngine::new();
    let mut stroke =
        |d: &mut Document, layer, tool: ToolKind, size: f32, rgb: [u8; 3], pts: Vec<(f64, f64)>| {
            d.active = layer;
            let mut s = BrushSettings::preset(tool);
            s.size = size * k;
            s.smoothing = 0.0;
            s.pressure_size = false;
            let first = pts[0];
            if eng.begin(d, tool, s, rgb, dir_from_lonlat(first.0, first.1), 1.0) {
                for (lon, lat) in pts.iter().skip(1) {
                    eng.add_point(dir_from_lonlat(*lon, *lat), 1.0, true);
                }
                eng.end(d);
            }
        };
    let n = 360;
    let band = |amp: f64, base: f64, f: f64| {
        (0..=n)
            .map(move |i| {
                let lon = TAU * i as f64 / n as f64;
                (lon, (base + amp * (f * lon).sin()).to_radians())
            })
            .collect::<Vec<_>>()
    };
    stroke(
        &mut d,
        aid,
        ToolKind::Airbrush,
        260.0,
        [0x1b, 0x99, 0xf9],
        band(7.0, 38.0, 3.0),
    );
    stroke(
        &mut d,
        aid,
        ToolKind::Airbrush,
        160.0,
        [0x5f, 0xd0, 0xfd],
        band(5.0, 30.0, 2.0),
    );
    stroke(
        &mut d,
        aid,
        ToolKind::Airbrush,
        120.0,
        [0x8a, 0x5c, 0xff],
        band(6.0, 46.0, 4.0),
    );
    // A tilted "orbit" ring like the one in the icon.
    let ring: Vec<_> = (0..=n)
        .map(|i| {
            let t = TAU * i as f64 / n as f64;
            let lat = (18.0f64).to_radians() * (t - PI * 0.4).sin() + 0.05;
            (t, lat)
        })
        .collect();
    stroke(
        &mut d,
        nid,
        ToolKind::Pen,
        10.0,
        [0xa1, 0xdb, 0xfb],
        ring.clone(),
    );
    stroke(&mut d, nid, ToolKind::Brush, 34.0, [0x06, 0x60, 0xe9], ring);
    // A few neon sparks.
    let mut rng = Lcg(7);
    for _ in 0..14 {
        let lon = rng.next() * TAU;
        let lat = (12.0 + rng.next() * 40.0).to_radians();
        let pts = vec![
            (lon - 0.02, lat),
            (lon + 0.02, lat),
            (lon, lat),
            (lon, lat + 0.03),
            (lon, lat - 0.03),
        ];
        stroke(&mut d, nid, ToolKind::Pen, 6.0, [0xe8, 0xf4, 0xfa], pts);
    }
    d.active = nid;
    d.take_dirty();
    d
}

/// Half sizes of the synthetic room (metres; the camera is at the origin).
const ROOM: [f64; 3] = [3.2, 1.6, 4.0];
const CEIL: f64 = 1.2;

/// The empty picture area on the far wall (`z = +ROOM[2]`): corners TL, TR, BR, BL.
pub fn room_wall_target() -> [[f64; 3]; 4] {
    let z = ROOM[2];
    let (x0, x1, y0, y1) = (-1.25, 1.15, -0.35, 0.85);
    [[x1, y1, z], [x0, y1, z], [x0, y0, z], [x1, y0, z]]
}

fn hash2(a: i64, b: i64) -> f64 {
    let mut h = (a.wrapping_mul(374761393) ^ b.wrapping_mul(668265263)) as u64;
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    ((h ^ (h >> 16)) & 0xffff) as f64 / 65535.0
}

fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Linear-ish RGB (0–1) seen along direction `d` from the room centre.
fn room_color(d: [f64; 3]) -> [f64; 3] {
    let lim = [ROOM[0], if d[1] > 0.0 { CEIL } else { ROOM[1] }, ROOM[2]];
    let mut t = f64::MAX;
    let mut axis = 0;
    for k in 0..3 {
        if d[k].abs() > 1e-9 {
            let tk = lim[k] / d[k].abs();
            if tk < t {
                t = tk;
                axis = k;
            }
        }
    }
    let p = [d[0] * t, d[1] * t, d[2] * t];
    // Ambient occlusion towards the room's edges.
    let edge = |v: f64, lo: f64, hi: f64| smooth(0.0, 0.9, (v - lo).min(hi - v));
    let ao_x = edge(p[0], -ROOM[0], ROOM[0]);
    let ao_y = edge(p[1], -ROOM[1], CEIL);
    let ao_z = edge(p[2], -ROOM[2], ROOM[2]);
    let ao = match axis {
        0 => ao_y * ao_z,
        1 => ao_x * ao_z,
        _ => ao_x * ao_y,
    };
    let ao = 0.55 + 0.45 * ao;
    // A ceiling lamp.
    let lamp = [0.0, CEIL - 0.05, 0.6];
    let dl =
        ((p[0] - lamp[0]).powi(2) + (p[1] - lamp[1]).powi(2) + (p[2] - lamp[2]).powi(2)).sqrt();
    let light = 0.62 + 0.55 / (1.0 + dl * dl * 0.12);
    let mut c = match axis {
        1 if d[1] < 0.0 => {
            // Wooden floor: planks along z.
            let pw = 0.22;
            let i = (p[0] / pw).floor() as i64;
            let len = 1.6 + hash2(i, 7) * 0.8;
            let j = ((p[2] + hash2(i, 3) * 3.0) / len).floor() as i64;
            let v = hash2(i, j);
            let grain = ((p[2] * 9.0 + v * 20.0).sin() * 0.5 + 0.5) * 0.06;
            let fx = (p[0] / pw).fract();
            let fz = ((p[2] + hash2(i, 3) * 3.0) / len).fract();
            let seam = smooth(0.0, 0.03, fx.min(1.0 - fx)) * smooth(0.0, 0.01, fz.min(1.0 - fz));
            let k = (0.78 + v * 0.22 - grain) * (0.55 + 0.45 * seam);
            [0.52 * k, 0.34 * k, 0.2 * k]
        }
        1 => {
            // Ceiling with the lamp.
            let r = ((p[0] - lamp[0]).powi(2) + (p[2] - lamp[2]).powi(2)).sqrt();
            if r < 0.22 {
                [1.6, 1.5, 1.3]
            } else {
                let ring = if r < 0.27 { 0.85 } else { 1.0 };
                [0.93 * ring, 0.92 * ring, 0.89 * ring]
            }
        }
        _ => {
            let (u, side) = if axis == 0 {
                (p[2], if d[0] > 0.0 { 0 } else { 1 })
            } else {
                (p[0], if d[2] > 0.0 { 2 } else { 3 })
            };
            let y = p[1];
            let base: [f64; 3] = match side {
                2 => [0.55, 0.64, 0.55], // the picture wall: sage green
                0 => [0.80, 0.74, 0.64],
                1 => [0.78, 0.73, 0.66],
                _ => [0.76, 0.72, 0.66],
            };
            let noise = (hash2((u * 40.0) as i64, (y * 40.0) as i64) - 0.5) * 0.025;
            let mut c = base.map(|v| v + noise);
            if y < -ROOM[1] + 0.14 {
                c = [0.95, 0.94, 0.92]; // skirting board
            } else if y < -ROOM[1] + 0.16 {
                c = [0.6, 0.6, 0.58];
            }
            if y > CEIL - 0.08 {
                c = [0.94, 0.93, 0.9]; // cornice
            }
            // Window on the left wall.
            if side == 1 && (-1.4..1.0).contains(&u) && (-0.45..0.95).contains(&y) {
                let fr = 0.06;
                let in_glass =
                    (-1.4 + fr..1.0 - fr).contains(&u) && (-0.45 + fr..0.95 - fr).contains(&y);
                let mull = (u + 0.2).abs() < 0.025 || (y - 0.25).abs() < 0.025;
                c = if in_glass && !mull {
                    let s = smooth(-0.45, 0.95, y);
                    [0.75 + 0.15 * s, 0.88 + 0.07 * s, 1.0 + 0.2 * s]
                } else {
                    [0.96, 0.96, 0.95]
                };
                return c.map(|v| v * 1.05);
            }
            // Door on the back wall.
            if side == 3 && (0.6..1.5).contains(&u) && y < 0.55 {
                let panel =
                    ((u - 1.05).abs() < 0.34) && ((y - 0.25).abs() < 0.2 || (y + 0.6).abs() < 0.55);
                c = if panel {
                    [0.62, 0.46, 0.33]
                } else {
                    [0.68, 0.51, 0.37]
                };
                if (u - 1.38).abs() < 0.03 && (y + 0.45).abs() < 0.03 {
                    c = [0.85, 0.75, 0.4];
                }
            }
            // A thin shadow line around the empty picture area (where a frame hung).
            if side == 2 {
                let t = room_wall_target();
                let (x0, x1, y0, y1) = (t[1][0], t[0][0], t[2][1], t[0][1]);
                let dx = (u - x0).min(x1 - u);
                let dy = (y - y0).min(y1 - y);
                let out = dx.min(dy);
                if (-0.05..0.0).contains(&out) {
                    let k = 0.8 + 0.2 * smooth(-0.05, 0.0, -out);
                    c = c.map(|v| v * k);
                }
                // Light shelf below.
                if (x0 - 0.2..x1 + 0.2).contains(&u) && (y0 - 0.55..y0 - 0.5).contains(&y) {
                    c = [0.36, 0.25, 0.17];
                }
            }
            c
        }
    };
    let k = ao * light;
    c = c.map(|v| v * k);
    c
}

/// An equirectangular 360° photo-like render of a simple room (docs screenshots and
/// the wall-fit demo).
pub fn room_panorama(w: u32, h: u32) -> Vec<u8> {
    let mut px = vec![0u8; (w * h * 4) as usize];
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 16);
    let rows = (h as usize).div_ceil(threads);
    std::thread::scope(|s| {
        for (ci, chunk) in px.chunks_mut(rows * w as usize * 4).enumerate() {
            s.spawn(move || {
                for (k, out) in chunk.chunks_mut(4).enumerate() {
                    let y = ci * rows + k / w as usize;
                    let x = k % w as usize;
                    let mut acc = [0.0f64; 3];
                    for (ox, oy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                        let lon = (x as f64 + ox) / w as f64 * TAU;
                        let lat = (0.5 - (y as f64 + oy) / h as f64) * PI;
                        let d = dir_from_lonlat(lon, lat);
                        let c = room_color([d.x, d.y, d.z]);
                        for i in 0..3 {
                            acc[i] += c[i] * 0.25;
                        }
                    }
                    for i in 0..3 {
                        // Soft tone curve.
                        let v = acc[i] / (1.0 + acc[i] * 0.25) * 1.25;
                        out[i] = (v.clamp(0.0, 1.0).powf(1.0 / 1.1) * 255.0 + 0.5) as u8;
                    }
                    out[3] = 255;
                }
            });
        }
    });
    px
}

/// A document with the room panorama as background and an empty layer above it.
pub fn room_document(w: u32, h: u32) -> Document {
    let mut d = Document::new_blank(
        w,
        h,
        Background::White,
        tr("layer.background"),
        &panoraminal_i18n::trf("layer.default_name", &[("n", &1)]),
    );
    let px = room_panorama(w, h);
    d.layers[0].pixels.copy_from_slice(&px);
    d
}
