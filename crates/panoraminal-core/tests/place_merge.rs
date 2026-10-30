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
