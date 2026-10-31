//! Integration test: place an image on the sphere, merge it, undo/redo, and place
//! one as a new layer.

use panoraminal_core::document::Background;
use panoraminal_core::math::{dir_from_lonlat, dir_to_px};
use panoraminal_core::place::{commit, PlaceImage, Placement, Projection};
use panoraminal_core::{Document, History};

fn checker(w: u32, h: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let red = (x / 8 + y / 8) % 2 == 0;
            px.extend_from_slice(if red {
                &[255, 0, 0, 255]
            } else {
                &[0, 0, 255, 255]
            });
        }
    }
    px
}

#[test]
fn place_merge_undo_redo() {
    let mut doc = Document::new_blank(1024, 512, Background::White, "bg", "paint");
    let mut hist = History::new();
    let layer = doc.active;
    let before = doc.active_layer().unwrap().pixels.clone();
    let img = PlaceImage::from_rgba8(64, 32, &checker(64, 32));
    for proj in [Projection::Wrap, Projection::Gnomonic] {
        // centred on the seam to exercise wrapping
        let c = dir_from_lonlat(0.0, 0.2);
        let p = Placement::new(proj, c, c, 1.2, 2.0, 0.4);
        let pre = img.prefiltered_for(&p, doc.width);
        let edit = p.bake(&pre, &mut doc, layer).expect("pixels changed");
        hist.push(edit, "hist.place_image", layer, layer);
        let l = doc.active_layer().unwrap();
        // the image centre is opaque, an opposite point untouched
        let cp = dir_to_px(c, 1024, 512);
        let i = ((cp.y as u32 * 1024 + cp.x as u32 % 1024) * 4) as usize;
        assert_eq!(l.pixels[i + 3], 255, "{proj:?}");
        let op = dir_to_px(dir_from_lonlat(std::f64::consts::PI, 0.0), 1024, 512);
        let j = ((op.y as u32 * 1024 + op.x as u32) * 4) as usize;
        assert_eq!(l.pixels[j + 3], 0);
        // both sides of the seam got paint
        let row = cp.y as u32 * 1024;
        assert!(l.pixels[((row + 3) * 4 + 3) as usize] > 0);
        assert!(l.pixels[((row + 1020) * 4 + 3) as usize] > 0);
        // checker colours survive (no smearing to purple in the middle of a square)
        let mut pure = 0;
        for x in 0..1024 {
            let k = ((row + x) * 4) as usize;
            if l.pixels[k + 3] == 255 && (l.pixels[k] > 240 || l.pixels[k + 2] > 240) {
                pure += 1;
            }
        }
        assert!(pure > 20);
        assert!(hist.undo(&mut doc));
        assert_eq!(doc.active_layer().unwrap().pixels, before, "undo restores");
        assert!(hist.redo(&mut doc));
        assert_ne!(doc.active_layer().unwrap().pixels, before);
        assert!(hist.undo(&mut doc));
    }
}

#[test]
fn place_as_new_layer_is_one_undo_step() {
    let mut doc = Document::new_blank(512, 256, Background::Sky, "bg", "paint");
    let mut hist = History::new();
    let n = doc.layers.len();
    let active = doc.active;
    let c = dir_from_lonlat(1.0, -0.3);
    let p = Placement::new(Projection::Gnomonic, c, c, 1.0, 1.0, 0.5);
    let img = PlaceImage::from_rgba8(16, 16, &[0, 255, 0, 200].repeat(256));
    let id = commit(&mut doc, &mut hist, &p, &img, Some("Image")).unwrap();
    assert_eq!(doc.layers.len(), n + 1);
    assert_eq!(doc.active, id);
    let baked = doc.layer(id).unwrap().pixels.clone();
    assert!(baked.chunks(4).any(|p| p[1] == 255 && p[3] > 150));
    assert_eq!(hist.len(), 1, "one undo step");
    assert!(hist.undo(&mut doc));
    assert_eq!(doc.layers.len(), n);
    assert_eq!(doc.active, active);
    assert!(hist.redo(&mut doc));
    assert_eq!(
        doc.layer(id).unwrap().pixels,
        baked,
        "redo restores the image"
    );
}

/// Four solid quadrants: TL red, TR green, BR blue, BL yellow.
fn quadrants(w: u32, h: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(match (x < w / 2, y < h / 2) {
                (true, true) => &[255, 0, 0, 255],
                (false, true) => &[0, 255, 0, 255],
                (false, false) => &[0, 0, 255, 255],
                (true, false) => &[255, 255, 0, 255],
            });
        }
    }
    px
}

#[test]
fn corner_pin_fits_a_wall() {
    use panoraminal_core::math::Vec3;
    let (w, h) = (2048u32, 1024u32);
    // A planar wall rectangle x = 1 (seen from the room centre), off-axis in z and
    // straddling the seam at lon = 0.
    let wall = |a: f64, b: f64| Vec3::new(1.0, 0.1 + 0.45 * b, 0.35 + 0.6 * a).normalize();
    let targets = [
        wall(-1.0, 1.0),
        wall(1.0, 1.0),
        wall(1.0, -1.0),
        wall(-1.0, -1.0),
    ];
    let colors: [[u8; 3]; 4] = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]];
    let img = PlaceImage::from_rgba8(256, 128, &quadrants(256, 128));
    for proj in [Projection::Gnomonic, Projection::Wrap] {
        let mut doc = Document::new_blank(w, h, Background::Transparent, "bg", "paint");
        let layer = doc.active;
        // The projector looks somewhere else than the wall normal: the pin must
        // still produce the wall's perspective.
        let view = Vec3::new(1.0, 0.05, 0.6).normalize();
        let mut p = Placement::new(proj, view, view, 1.2, 2.0, 0.3);
        for (k, t) in targets.iter().enumerate() {
            let (x, y) = p.local(*t).unwrap();
            p.warp.corners[k] = [x, y];
        }
        p.warp.validate().expect("valid quad");
        let pre = img.prefiltered_for(&p, w);
        p.bake(&pre, &mut doc, layer).expect("baked");
        let px = &doc.layer(layer).unwrap().pixels;
        let at = |d: Vec3| {
            let q = dir_to_px(d, w, h);
            let i = (((q.y as u32).min(h - 1) * w + (q.x as u32) % w) * 4) as usize;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        for (k, (a, b)) in [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)]
            .into_iter()
            .enumerate()
        {
            // Just inside the corner (wall coordinates): that quadrant's colour.
            let c = at(wall(a * 0.95, b * 0.95));
            assert!(c[3] > 250, "{proj:?} corner {k} alpha {c:?}");
            for i in 0..3 {
                assert!(
                    (c[i] as i32 - colors[k][i] as i32).abs() < 40,
                    "{proj:?} corner {k}: {c:?}"
                );
            }
            // Just outside: untouched.
            assert_eq!(at(wall(a * 1.06, b * 1.06))[3], 0, "{proj:?} outside {k}");
        }
        if proj == Projection::Gnomonic {
            // Perspective-correct: the wall's centre is the image centre, and the
            // wall's quarter lines are the image's quarter lines.
            for (a, b) in [(0.0, 0.0), (0.5, 0.5), (-0.5, 0.25)] {
                let (sx, sy) = p.image_local(wall(a, b)).unwrap();
                assert!((sx - a).abs() < 1e-6 && (sy - b).abs() < 1e-6, "{sx} {sy}");
            }
        }
        // The image crosses the seam: both sides have paint.
        let row = dir_to_px(wall(0.0, 0.0), w, h).y as u32;
        let painted = |x: u32| px[((row * w + x) * 4 + 3) as usize] > 0;
        assert!(painted(2) && painted(w - 3), "{proj:?} seam");
    }
}
