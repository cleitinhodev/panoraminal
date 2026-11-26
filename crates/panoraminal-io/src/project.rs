//! The `.pano` project format.
//!
//! A `.pano` file is a ZIP archive:
//!
//! ```text
//! manifest.json          format, version, size, layer list and properties
//! layers/000.png …       one straight-alpha RGBA PNG per layer (bottom first)
//! preview.jpg            small flattened preview (for file browsers / future use)
//! brushes/library.json   optional: custom brushes embedded in the project (1.1+)
//! brushes/000_0.png …    their tips (grayscale PNG)
//! lines/003.json …       optional: vector strokes of line layers (1.4+, format minor 2)
//! ```
//!
//! Compatibility: the manifest `version` stays 1 so every Panoraminal 1.x opens the
//! file. Newer, optional data is announced by `minor` (1 = embedded brushes, 2 = line
//! layers) and by optional per-layer fields. A line layer's PNG always holds its
//! rendered strokes, so older versions show it as a normal layer (and save it as one).
//!
//! Legacy Paint360 projects (`.p360.json`, layers as PNG data URLs) are imported too.

use crate::{encode_jpeg, encode_png, IoError, Result};
use base64::Engine;
use panoraminal_core::document::downsample;
use panoraminal_core::{BlendMode, Document, Layer, LineArt, MAX_DIM, MAX_LAYERS};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, Write};
use std::path::Path;

pub const PROJECT_EXT: &str = "pano";
const FORMAT: &str = "panoraminal";
const VERSION: u32 = 1;
/// Optional features written by this version (see the module docs).
const MINOR: u32 = 2;
const LINES_FORMAT: &str = "panoraminal-lines";

/// Owned copy of a document, safe to send to a background thread for saving.
#[derive(Clone, Debug)]
pub struct ProjectSnapshot {
    pub width: u32,
    pub height: u32,
    pub active_index: usize,
    pub layers: Vec<LayerSnapshot>,
    /// Custom brushes embedded in the project (optional).
    pub brushes: Vec<crate::brushes::CustomBrush>,
}

#[derive(Clone, Debug)]
pub struct LayerSnapshot {
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    pub alpha_locked: bool,
    pub pixels: Vec<u8>,
    /// Vector strokes of a line layer (`pixels` is their raster cache).
    pub lines: Option<LineArt>,
}

impl ProjectSnapshot {
    pub fn from_document(doc: &Document) -> Self {
        Self {
            width: doc.width,
            height: doc.height,
            active_index: doc.active_index(),
            layers: doc
                .layers
                .iter()
                .map(|l| LayerSnapshot {
                    name: l.name.clone(),
                    visible: l.visible,
                    opacity: l.opacity,
                    blend: l.blend,
                    alpha_locked: l.alpha_locked,
                    pixels: l.pixels.clone(),
                    lines: l.lines.clone(),
                })
                .collect(),
            brushes: Vec::new(),
        }
    }
    pub fn into_document(self) -> Document {
        let mut d = Document::empty(self.width, self.height);
        for s in self.layers {
            let id = d.alloc_id();
            let mut l = Layer::new(id, s.name, 1, 1);
            l.pixels = s.pixels;
            l.visible = s.visible;
            l.opacity = s.opacity.clamp(0.0, 1.0);
            l.blend = s.blend;
            l.alpha_locked = s.alpha_locked;
            if let Some(art) = s.lines {
                // Keep the cache exactly equal to the strokes.
                if l.pixels.len() == d.pixel_len() {
                    panoraminal_core::lines::render_full(&art, &mut l.pixels, d.width, d.height);
                    l.lines = Some(art);
                }
            }
            d.layers.push(l);
        }
        let i = self.active_index.min(d.layers.len().saturating_sub(1));
        d.active = d.layers[i].id;
        d
    }
    fn flatten(&self) -> Vec<u8> {
        // Reuse the core compositor through a temporary document view.
        let d = self.clone().into_document();
        d.flatten(Some([255, 255, 255]))
    }
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
    #[serde(default)]
    generator: String,
    width: u32,
    height: u32,
    #[serde(default)]
    active_index: usize,
    /// Optional features (0 = 1.0, 1 = embedded brushes, 2 = line layers).
    #[serde(default)]
    minor: u32,
    layers: Vec<ManifestLayer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    brushes: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ManifestLayer {
    name: String,
    #[serde(default = "yes")]
    visible: bool,
    #[serde(default = "one")]
    opacity: f32,
    #[serde(default)]
    blend: String,
    #[serde(default)]
    alpha_locked: bool,
    file: String,
    /// Vector strokes of a line layer (1.4+).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lines: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct LinesFile {
    format: String,
    version: u32,
    #[serde(flatten)]
    art: LineArt,
}
fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}

/// Serializes a project to `.pano` bytes. Layers are PNG-encoded in parallel.
pub fn project_bytes(p: &ProjectSnapshot) -> Result<Vec<u8>> {
    let (w, h) = (p.width, p.height);
    let pngs: Vec<Result<Vec<u8>>> = std::thread::scope(|s| {
        let hs: Vec<_> = p
            .layers
            .iter()
            .map(|l| s.spawn(move || encode_png(&l.pixels, w, h, true)))
            .collect();
        hs.into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(IoError::Invalid("encoder thread panicked".into())))
            })
            .collect()
    });
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let manifest = Manifest {
            format: FORMAT.into(),
            version: VERSION,
            generator: format!("Panoraminal {}", env!("CARGO_PKG_VERSION")),
            width: w,
            height: h,
            active_index: p.active_index,
            minor: MINOR,
            layers: p
                .layers
                .iter()
                .enumerate()
                .map(|(i, l)| ManifestLayer {
                    name: l.name.clone(),
                    visible: l.visible,
                    opacity: l.opacity,
                    blend: l.blend.id().into(),
                    alpha_locked: l.alpha_locked,
                    file: format!("layers/{i:03}.png"),
                    lines: l.lines.as_ref().map(|_| format!("lines/{i:03}.json")),
                })
                .collect(),
            brushes: (!p.brushes.is_empty()).then(|| "brushes/library.json".to_string()),
        };
        zw.start_file("manifest.json", deflated)?;
        zw.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
        for (i, png) in pngs.into_iter().enumerate() {
            zw.start_file(format!("layers/{i:03}.png"), stored)?;
            zw.write_all(&png?)?;
        }
        for (i, l) in p.layers.iter().enumerate() {
            if let Some(art) = &l.lines {
                let f = LinesFile {
                    format: LINES_FORMAT.into(),
                    version: 1,
                    art: art.clone(),
                };
                zw.start_file(format!("lines/{i:03}.json"), deflated)?;
                zw.write_all(&serde_json::to_vec(&f)?)?;
            }
        }
        let pw = 512.min(w);
        let ph = (pw * h / w).max(1);
        let prev = downsample(&p.flatten(), w, h, pw, ph);
        zw.start_file("preview.jpg", stored)?;
        zw.write_all(&encode_jpeg(&prev, pw, ph, 85, false)?)?;
        if !p.brushes.is_empty() {
            let (json, files) = crate::brushes::embed_brushes(&p.brushes)?;
            zw.start_file("brushes/library.json", deflated)?;
            zw.write_all(&json)?;
            for (name, png) in files {
                zw.start_file(name, stored)?;
                zw.write_all(&png)?;
            }
        }
        zw.finish()?;
    }
    Ok(buf.into_inner())
}

/// Saves a project atomically.
pub fn save_project(p: &ProjectSnapshot, path: &Path) -> Result<()> {
    crate::write_atomic(path, &project_bytes(p)?)
}

pub fn load_project(path: &Path) -> Result<ProjectSnapshot> {
    load_project_bytes(&std::fs::read(path)?)
}

/// Loads `.pano` (ZIP) or legacy Paint360 `.p360.json` bytes.
pub fn load_project_bytes(bytes: &[u8]) -> Result<ProjectSnapshot> {
    if bytes.starts_with(b"PK") {
        load_zip(std::io::Cursor::new(bytes))
    } else {
        load_legacy_json(bytes)
    }
}

fn check_dims(w: u32, h: u32) -> Result<()> {
    if !(16..=MAX_DIM).contains(&w) || !(8..=MAX_DIM).contains(&h) {
        return Err(IoError::Invalid(format!("invalid dimensions {w}×{h}")));
    }
    Ok(())
}

fn decode_layer_png(bytes: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let img = image::load_from_memory(bytes)?.to_rgba8();
    let img = if img.dimensions() != (w, h) {
        image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    Ok(img.into_raw())
}

fn load_zip<R: Read + Seek>(r: R) -> Result<ProjectSnapshot> {
    let mut z = zip::ZipArchive::new(r)?;
    let mut s = String::new();
    z.by_name("manifest.json")?.read_to_string(&mut s)?;
    let m: Manifest = serde_json::from_str(&s)?;
    if m.format != FORMAT {
        return Err(IoError::Invalid("not a Panoraminal project".into()));
    }
    if m.version > VERSION {
        return Err(IoError::Invalid(format!(
            "project version {} is newer than this Panoraminal",
            m.version
        )));
    }
    check_dims(m.width, m.height)?;
    if m.layers.is_empty() || m.layers.len() > MAX_LAYERS {
        return Err(IoError::Invalid("bad layer count".into()));
    }
    let mut raw = Vec::new();
    for l in &m.layers {
        let mut b = Vec::new();
        z.by_name(&l.file)?.read_to_end(&mut b)?;
        raw.push(b);
    }
    let (w, h) = (m.width, m.height);
    let decoded: Vec<Result<Vec<u8>>> = std::thread::scope(|sc| {
        let hs: Vec<_> = raw
            .iter()
            .map(|b| sc.spawn(move || decode_layer_png(b, w, h)))
            .collect();
        hs.into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(IoError::Invalid("decoder thread panicked".into())))
            })
            .collect()
    });
    let mut layers = Vec::new();
    for (l, px) in m.layers.into_iter().zip(decoded) {
        // Unreadable stroke data degrades to the raster cache.
        let lines = l.lines.as_deref().and_then(|f| {
            if !f.starts_with("lines/") {
                return None;
            }
            let mut b = Vec::new();
            z.by_name(f).ok()?.read_to_end(&mut b).ok()?;
            let lf: LinesFile = serde_json::from_slice(&b).ok()?;
            if lf.format != LINES_FORMAT {
                return None;
            }
            let mut art = lf.art;
            art.sanitize();
            Some(art)
        });
        layers.push(LayerSnapshot {
            name: l.name.chars().take(80).collect(),
            visible: l.visible,
            opacity: if l.opacity.is_finite() {
                l.opacity.clamp(0.0, 1.0)
            } else {
                1.0
            },
            blend: BlendMode::from_id(&l.blend).unwrap_or_default(),
            alpha_locked: l.alpha_locked,
            pixels: px?,
            lines,
        });
    }
    let mut brushes = Vec::new();
    if let Some(bf) = m.brushes.as_deref() {
        let mut json = Vec::new();
        if z.by_name(bf)
            .and_then(|mut f| Ok(f.read_to_end(&mut json)?))
            .is_ok()
        {
            brushes = crate::brushes::unembed_brushes(&json, |n| {
                if !n.starts_with("brushes/") {
                    return None;
                }
                let mut b = Vec::new();
                z.by_name(n).ok()?.read_to_end(&mut b).ok()?;
                Some(b)
            })
            .unwrap_or_default();
        }
    }
    Ok(ProjectSnapshot {
        width: w,
        height: h,
        active_index: m.active_index,
        layers,
        brushes,
    })
}

#[derive(Deserialize)]
struct LegacyProject {
    format: String,
    width: u32,
    height: u32,
    #[serde(default, rename = "activeIndex")]
    active_index: Option<i64>,
    layers: Vec<LegacyLayer>,
}
#[derive(Deserialize)]
struct LegacyLayer {
    #[serde(default)]
    name: String,
    #[serde(default = "yes")]
    visible: bool,
    #[serde(default = "one")]
    opacity: f32,
    #[serde(default)]
    blend: String,
    data: String,
}

fn load_legacy_json(bytes: &[u8]) -> Result<ProjectSnapshot> {
    let p: LegacyProject = serde_json::from_slice(bytes)?;
    if p.format != "paint360" {
        return Err(IoError::Invalid("unknown project format".into()));
    }
    check_dims(p.width, p.height)?;
    if p.layers.is_empty() || p.layers.len() > MAX_LAYERS {
        return Err(IoError::Invalid("bad layer count".into()));
    }
    let mut layers = Vec::new();
    for l in p.layers {
        let b64 = l
            .data
            .split_once(',')
            .map(|x| x.1)
            .ok_or_else(|| IoError::Invalid("bad layer data".into()))?;
        let png = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| IoError::Invalid(e.to_string()))?;
        layers.push(LayerSnapshot {
            name: if l.name.is_empty() {
                "Layer".into()
            } else {
                l.name.chars().take(80).collect()
            },
            visible: l.visible,
            opacity: if l.opacity.is_finite() {
                l.opacity.clamp(0.0, 1.0)
            } else {
                1.0
            },
            blend: BlendMode::from_id(&l.blend).unwrap_or_default(),
            alpha_locked: false,
            pixels: decode_layer_png(&png, p.width, p.height)?,
            lines: None,
        });
    }
    let n = layers.len();
    let ai = p
        .active_index
        .unwrap_or(n as i64 - 1)
        .clamp(0, n as i64 - 1) as usize;
    Ok(ProjectSnapshot {
        width: p.width,
        height: p.height,
        active_index: ai,
        layers,
        brushes: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoraminal_core::document::Background;

    fn sample() -> Document {
        let mut d = Document::new_blank(128, 64, Background::Sky, "Fundo", "Camada 1");
        let l = d.layers.last_mut().unwrap();
        l.blend = BlendMode::Overlay;
        l.opacity = 0.5;
        l.name = "Céu ✨".into();
        for (i, p) in l.pixels.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(&[(i % 255) as u8, 20, 200, (i % 7 * 30) as u8]);
        }
        d
    }

    #[test]
    fn pano_roundtrip_is_lossless() {
        let d = sample();
        let bytes = project_bytes(&ProjectSnapshot::from_document(&d)).unwrap();
        let p = load_project_bytes(&bytes).unwrap();
        let d2 = p.into_document();
        assert_eq!(d2.layers.len(), 2);
        assert_eq!(d2.layers[1].name, "Céu ✨");
        assert_eq!(d2.layers[1].blend, BlendMode::Overlay);
        assert_eq!(d2.layers[1].opacity, 0.5);
        assert_eq!(d2.flatten(None), d.flatten(None));
        assert_eq!(d2.active_index(), 1);
    }

    #[test]
    fn embedded_brushes_roundtrip() {
        let d = sample();
        let mut snap = ProjectSnapshot::from_document(&d);
        let tip = panoraminal_core::tip::TipImage::ellipse(12, 0.5, 0.3);
        let mut b = crate::brushes::CustomBrush::new("Mine", vec![tip], 12.0);
        b.dynamics.scatter = 2.0;
        snap.brushes.push(b);
        let bytes = project_bytes(&snap).unwrap();
        let p = load_project_bytes(&bytes).unwrap();
        assert_eq!(p.brushes, snap.brushes);
        // projects without brushes carry none
        let bytes = project_bytes(&ProjectSnapshot::from_document(&d)).unwrap();
        assert!(load_project_bytes(&bytes).unwrap().brushes.is_empty());
    }

    fn with_lines() -> Document {
        use panoraminal_core::history::{ops, History};
        use panoraminal_core::{LinePoint, LineStroke};
        let mut d = sample();
        let mut h = History::new();
        let id = ops::add_line_layer(&mut d, &mut h, "Linhas").unwrap();
        let art = LineArt {
            strokes: vec![LineStroke {
                points: vec![
                    LinePoint {
                        lon: 350.0,
                        lat: 5.0,
                        w: 1.0,
                        sharp: false,
                    },
                    LinePoint {
                        lon: 20.0,
                        lat: 30.0,
                        w: 2.5,
                        sharp: true,
                    },
                    LinePoint {
                        lon: 60.0,
                        lat: -10.0,
                        w: 0.5,
                        sharp: false,
                    },
                ],
                color: [255, 216, 26],
                width: 5.0,
                taper_start: 0.2,
                taper_end: 0.4,
                ..Default::default()
            }],
        };
        ops::set_lines(&mut d, &mut h, id, art, "hist.line_add");
        d.layer_mut(id).unwrap().blend = BlendMode::Screen;
        d
    }

    #[test]
    fn line_layers_roundtrip() {
        let d = with_lines();
        let bytes = project_bytes(&ProjectSnapshot::from_document(&d)).unwrap();
        let d2 = load_project_bytes(&bytes).unwrap().into_document();
        assert_eq!(d2.layers.len(), 3);
        assert_eq!(d2.layers[2].lines, d.layers[2].lines);
        assert_eq!(d2.layers[2].pixels, d.layers[2].pixels);
        assert_eq!(d2.layers[2].blend, BlendMode::Screen);
        assert!(!d2.layers[1].is_lines());
        assert_eq!(d2.flatten(None), d.flatten(None));
    }

    #[test]
    fn line_layers_degrade_for_older_readers() {
        let d = with_lines();
        let bytes = project_bytes(&ProjectSnapshot::from_document(&d)).unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let mut s = String::new();
        z.by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        // 1.0–1.3 accept version 1 and ignore the unknown fields...
        assert_eq!(v["version"], 1);
        assert_eq!(v["minor"], 2);
        assert_eq!(v["layers"][2]["lines"], "lines/002.json");
        assert!(v["layers"][1].get("lines").is_none());
        // ...and read the layer PNG, which holds the rendered strokes.
        let mut png = Vec::new();
        z.by_name("layers/002.png")
            .unwrap()
            .read_to_end(&mut png)
            .unwrap();
        assert_eq!(decode_layer_png(&png, 128, 64).unwrap(), d.layers[2].pixels);
        // A 1.3-style manifest without the new fields still loads (as raster).
        let old = s.replace("\"lines\": \"lines/002.json\"", "\"x\": 0");
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut out);
            let o = zip::write::SimpleFileOptions::default();
            for i in 0..z.len() {
                let mut f = z.by_index(i).unwrap();
                let name = f.name().unwrap().to_string();
                let mut b = Vec::new();
                f.read_to_end(&mut b).unwrap();
                zw.start_file(name.clone(), o).unwrap();
                if name == "manifest.json" {
                    zw.write_all(old.as_bytes()).unwrap();
                } else {
                    zw.write_all(&b).unwrap();
                }
            }
            zw.finish().unwrap();
        }
        let d3 = load_project_bytes(out.get_ref()).unwrap().into_document();
        assert!(!d3.layers[2].is_lines());
        assert_eq!(d3.layers[2].pixels, d.layers[2].pixels);
    }

    #[test]
    fn corrupt_line_data_falls_back_to_pixels() {
        let d = with_lines();
        let bytes = project_bytes(&ProjectSnapshot::from_document(&d)).unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut out);
            let o = zip::write::SimpleFileOptions::default();
            for i in 0..z.len() {
                let mut f = z.by_index(i).unwrap();
                let name = f.name().unwrap().to_string();
                let mut b = Vec::new();
                f.read_to_end(&mut b).unwrap();
                zw.start_file(name.clone(), o).unwrap();
                zw.write_all(if name.starts_with("lines/") {
                    b"{oops"
                } else {
                    &b
                })
                .unwrap();
            }
            zw.finish().unwrap();
        }
        let d2 = load_project_bytes(out.get_ref()).unwrap().into_document();
        assert!(!d2.layers[2].is_lines());
        assert_eq!(d2.layers[2].pixels, d.layers[2].pixels);
    }

    #[test]
    fn legacy_p360_import() {
        let d = sample();
        let png0 = encode_png(&d.layers[0].pixels, 128, 64, true).unwrap();
        let png1 = encode_png(&d.layers[1].pixels, 128, 64, true).unwrap();
        let b = |p: &[u8]| {
            format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(p)
            )
        };
        let json = format!(
            r#"{{"format":"paint360","version":1,"width":128,"height":64,"activeIndex":0,"layers":[{{"name":"Fundo","visible":true,"opacity":1,"blend":"normal","data":"{}"}},{{"name":"Camada 1","visible":false,"opacity":0.5,"blend":"overlay","data":"{}"}}]}}"#,
            b(&png0),
            b(&png1)
        );
        let p = load_project_bytes(json.as_bytes()).unwrap();
        assert_eq!(p.layers.len(), 2);
        assert_eq!(p.active_index, 0);
        assert!(!p.layers[1].visible);
        assert_eq!(p.layers[1].blend, BlendMode::Overlay);
        assert_eq!(p.layers[1].pixels, d.layers[1].pixels);
    }

    #[test]
    fn rejects_garbage() {
        assert!(load_project_bytes(
            b"{\"format\":\"other\",\"width\":10,\"height\":10,\"layers\":[]}"
        )
        .is_err());
        assert!(load_project_bytes(b"PK\x03\x04garbage").is_err());
        assert!(load_project_bytes(b"not json").is_err());
    }
}
