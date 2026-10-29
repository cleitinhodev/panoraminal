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
