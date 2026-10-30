//! Brush importers against synthesized fixtures (see `scripts/gen-brush-fixtures.py`).

use panoraminal_core::tip::{TipImage, TipSelection};
use panoraminal_io::abr::parse_abr;
use panoraminal_io::brushes::{
    embed_brushes, import_brush_bytes, parse_gbr, parse_gih, tip_from_image_bytes, unembed_brushes,
    BrushLibrary, Notice,
};
use std::path::Path;

const V1: &[u8] = include_bytes!("fixtures/v1.abr");
const V2: &[u8] = include_bytes!("fixtures/v2.abr");
const V6: &[u8] = include_bytes!("fixtures/v6.abr");
const V10: &[u8] = include_bytes!("fixtures/v10_sub2_16bit.abr");
const GBR: &[u8] = include_bytes!("fixtures/tip.gbr");
const GBR_COLOR: &[u8] = include_bytes!("fixtures/color.gbr");
const GIH: &[u8] = include_bytes!("fixtures/pipe.gih");

/// Same shapes as the generator script.
fn star(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::new();
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let r = (w.min(h)) as f64 / 2.0;
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f64 + 0.5 - cx) / r;
            let dy = (y as f64 + 0.5 - cy) / r;
            let a = dy.atan2(dx);
            let lim = 0.55 + 0.45 * (2.5 * a).cos().abs();
            v.push(if dx.hypot(dy) <= lim { 255 } else { 0 });
        }
    }
    v
}

#[test]
fn abr_v1_computed_and_sampled() {
    let v = parse_abr(V1, "set").unwrap();
    assert_eq!(v.len(), 2);
    let c = &v[0];
    assert_eq!(c.diameter, 40.0);
    assert!((c.dynamics.spacing - 0.25).abs() < 1e-6);
    assert!((c.dynamics.roundness - 0.6).abs() < 1e-6);
    assert_eq!(c.dynamics.angle, 30.0);
    // hardness 90 % → solid core
    let t = &c.tips[0];
    assert_eq!(t.data[(t.height / 2 * t.width + t.width / 2) as usize], 255);
    let s = &v[1];
    assert_eq!(s.name, "set 2");
    assert!((s.dynamics.spacing - 0.5).abs() < 1e-6);
    assert_eq!(s.tips[0], TipImage::new(24, 24, star(24, 24)).trimmed());
    assert_eq!(s.source, "set.abr");
}

#[test]
fn abr_v2_names_and_packbits() {
    let v = parse_abr(V2, "x").unwrap();
    assert_eq!(v.len(), 2);
    assert_eq!(v[0].name, "Anel");
    assert_eq!((v[0].tips[0].width, v[0].tips[0].height), (32, 20));
    assert!((v[0].dynamics.spacing - 0.35).abs() < 1e-6);
    assert_eq!(v[1].name, "Gradiente");
    // gradient: first column is 0 → trimmed away
    assert_eq!(v[1].tips[0].width, 15);
    assert_eq!(v[1].tips[0].data[14], 255);
}

#[test]
fn abr_v6_descriptor_dynamics() {
    let v = parse_abr(V6, "pack").unwrap();
    // 3 presets + 1 sample not referenced by any preset
    assert_eq!(
        v.len(),
        4,
        "{:?}",
        v.iter().map(|b| &b.name).collect::<Vec<_>>()
    );
    let b = &v[0];
    assert_eq!(b.name, "Estrela ✨ 30");
    assert_eq!(b.diameter, 30.0);
    assert_eq!(b.tips[0], TipImage::new(24, 24, star(24, 24)).trimmed());
    let d = &b.dynamics;
    assert!((d.spacing - 0.40).abs() < 1e-6);
    assert_eq!(d.angle, 45.0);
    assert!((d.roundness - 0.5).abs() < 1e-6);
    assert!(d.flip_x && !d.flip_y);
    assert!(!d.flip_x_jitter && d.flip_y_jitter);
    assert!((d.size_jitter - 0.3).abs() < 1e-6);
    assert!((d.min_diameter - 0.2).abs() < 1e-6);
    assert!(b.pressure_size, "szVr control = pen pressure");
    assert!((b.min_size - 0.2).abs() < 1e-6);
    assert!((d.angle_jitter - 0.1).abs() < 1e-6);
    assert!(d.follow_direction, "angle control = direction");
    assert!((d.roundness_jitter - 0.25).abs() < 1e-6);
    assert!((d.min_roundness - 0.4).abs() < 1e-6);
    assert!((d.scatter - 1.5).abs() < 1e-6);
    assert!(d.scatter_both_axes);
    assert_eq!(d.count, 3);
    assert!((d.count_jitter - 0.5).abs() < 1e-6);
    assert!((d.opacity_jitter - 0.2).abs() < 1e-6);
    assert!((d.flow_jitter - 0.1).abs() < 1e-6);
    assert!(b.pressure_opacity);
    for n in [Notice::DualBrush, Notice::Texture, Notice::WetEdges] {
        assert!(b.notices.contains(&n), "missing {n:?}");
    }
    assert!(!b.notices.contains(&Notice::Noise));

    let c = &v[1];
    assert_eq!(c.name, "Round 50");
    assert_eq!(c.diameter, 50.0);
    assert_eq!(c.dynamics.angle, 330.0);
    assert!(c.notices.is_empty(), "computed round brushes are supported");
    assert_eq!(c.tips[0].width, 50);

    let br = &v[2];
    assert_eq!(br.name, "Bristle");
    assert!(br.notices.contains(&Notice::Bristle));
    assert_eq!(br.diameter, 36.0);

    assert_eq!(v[3].name, "pack 2");
    assert_eq!(v[3].tips[0].width, 20);
}

#[test]
fn abr_v10_subversion2_16bit_without_descriptor() {
    let v = parse_abr(V10, "deep").unwrap();
    assert_eq!(v.len(), 1);
    let t = &v[0].tips[0];
    assert_eq!((t.width, t.height), (11, 6)); // first (zero) column trimmed
    assert_eq!(t.data[10], 255);
}

#[test]
fn abr_truncated_files_never_panic() {
    for f in [V1, V2, V6, V10] {
        for n in 0..f.len() {
            let _ = parse_abr(&f[..n], "t");
        }
        // corrupt bytes
        let mut g = f.to_vec();
        for i in (4..g.len()).step_by(7) {
            g[i] ^= 0x5a;
            let _ = parse_abr(&g, "t");
        }
    }
}

#[test]
fn gimp_gbr_and_gih() {
    let b = parse_gbr(GBR, "f").unwrap();
    assert_eq!(b.name, "Ponto");
    assert!((b.dynamics.spacing - 0.6).abs() < 1e-6);
    assert_eq!(b.tips[0].max_side(), 8);
    let c = parse_gbr(GBR_COLOR, "f").unwrap();
    assert!(c.notices.contains(&Notice::ColorTip));
    assert_eq!(c.tips[0].data.len(), 4);
    let p = parse_gih(GIH, "f").unwrap();
    assert_eq!(p.name, "Pipe");
    assert_eq!(p.tips.len(), 2);
    assert_eq!(p.dynamics.selection, TipSelection::Random);
    let v = import_brush_bytes(GIH, Path::new("/x/My pipes.gih")).unwrap();
    assert_eq!(v[0].category, "My pipes");
}

#[test]
fn image_tips_use_alpha_or_luminance() {
    // opaque image: black square on white → luminance (inverted)
    let mut img = image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 255, 255, 255]));
    for y in 2..6 {
        for x in 2..6 {
            img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
        }
    }
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let b = tip_from_image_bytes(&png, "sq").unwrap();
    assert_eq!((b.tips[0].width, b.tips[0].height), (4, 4));
    assert!(b.tips[0].data.iter().all(|&v| v == 255));
    // transparent image → alpha
    let mut img = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 0]));
    img.put_pixel(1, 1, image::Rgba([255, 255, 255, 128]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let b = tip_from_image_bytes(&png, "a").unwrap();
    assert_eq!(b.tips[0].data, vec![128]);
}

#[test]
fn library_roundtrip_and_embedding() {
    let dir = std::env::temp_dir().join(format!("pn-brushlib-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut lib = BrushLibrary::default();
    let mut v = parse_abr(V6, "pack").unwrap();
    v[0].favorite = true;
    assert_eq!(lib.add(v.clone()), 4);
    assert_eq!(lib.add(v), 0, "duplicates are skipped");
    lib.brushes[1].name = "Renamed".into();
    lib.save(&dir).unwrap();
    let back = BrushLibrary::load(&dir).unwrap();
    assert_eq!(back, lib);
    // delete removes the tip files on the next save
    let id = lib.brushes[3].id.clone();
    lib.remove(&id).unwrap();
    lib.save(&dir).unwrap();
    let pngs = std::fs::read_dir(&dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "png")
        })
        .count();
    assert_eq!(pngs, 3);
    let _ = std::fs::remove_dir_all(&dir);

    let (json, files) = embed_brushes(&lib.brushes).unwrap();
    let got = unembed_brushes(&json, |n| {
        files.iter().find(|(f, _)| f == n).map(|(_, b)| b.clone())
    })
    .unwrap();
    assert_eq!(got, lib.brushes);
}
