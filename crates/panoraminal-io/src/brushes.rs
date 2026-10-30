//! The custom brush model, the on-disk brush library and tip importers
//! (GIMP `.gbr` / `.gih`, plain images; Photoshop `.abr` lives in [`crate::abr`]).

use crate::{IoError, Result};
use panoraminal_core::tip::{PreparedTip, TipDynamics, TipImage};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

/// Something in an imported brush that Panoraminal approximates or ignores.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notice {
    DualBrush,
    Texture,
    Bristle,
    Erodible,
    ColorDynamics,
    WetEdges,
    Noise,
    /// A procedural tip that was replaced by a round tip.
    Computed,
    /// A colour tip whose colours were reduced to a coverage mask.
    ColorTip,
}

impl Notice {
    /// i18n key describing the notice.
    pub fn key(self) -> &'static str {
        match self {
            Notice::DualBrush => "brushlib.notice.dual",
            Notice::Texture => "brushlib.notice.texture",
            Notice::Bristle => "brushlib.notice.bristle",
            Notice::Erodible => "brushlib.notice.erodible",
            Notice::ColorDynamics => "brushlib.notice.color_dynamics",
            Notice::WetEdges => "brushlib.notice.wet_edges",
            Notice::Noise => "brushlib.notice.noise",
            Notice::Computed => "brushlib.notice.computed",
            Notice::ColorTip => "brushlib.notice.color_tip",
        }
    }
}

/// A brush of the library: one or more tips plus dynamics and defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomBrush {
    pub id: String,
    pub name: String,
    pub category: String,
    pub favorite: bool,
    /// Default diameter in pixels.
    pub diameter: f32,
    pub dynamics: TipDynamics,
    pub pressure_size: bool,
    pub pressure_opacity: bool,
    pub min_size: f32,
    /// Where it came from (file name), informational.
    pub source: String,
    pub notices: Vec<Notice>,
    #[serde(skip)]
    pub tips: Vec<TipImage>,
}

impl Default for CustomBrush {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            category: String::new(),
            favorite: false,
            diameter: 60.0,
            dynamics: TipDynamics::default(),
            pressure_size: true,
            pressure_opacity: false,
            min_size: 0.1,
            source: String::new(),
            notices: Vec::new(),
            tips: Vec::new(),
        }
    }
}

fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{:x}{:04x}", t, N.fetch_add(1, Ordering::Relaxed) & 0xffff)
}

impl CustomBrush {
    pub fn new(name: impl Into<String>, tips: Vec<TipImage>, diameter: f32) -> Self {
        Self {
            id: new_id(),
            name: name.into().chars().take(80).collect(),
            diameter: diameter.clamp(1.0, 1000.0),
            tips,
            ..Default::default()
        }
    }
    pub fn prepared(&self) -> Arc<PreparedTip> {
        PreparedTip::new(&self.tips, &self.dynamics)
    }
    /// Grayscale thumbnail (RGBA, `size`²) of the first tip.
    pub fn thumbnail(&self, size: u32, rgb: [u8; 3]) -> Vec<u8> {
        match self.tips.first() {
            Some(t) => t.to_rgba_preview(size, rgb),
            None => vec![0; (size * size * 4) as usize],
        }
    }
}

// ---- tip PNG encoding -------------------------------------------------------------

pub fn encode_tip_png(t: &TipImage) -> Result<Vec<u8>> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(
        &t.data,
        t.width,
        t.height,
        image::ExtendedColorType::L8,
    )?;
    Ok(out)
}

pub fn decode_tip_png(b: &[u8]) -> Result<TipImage> {
    let img = image::load_from_memory(b)?.to_luma8();
    let (w, h) = img.dimensions();
    Ok(TipImage::new(w, h, img.into_raw()))
}

// ---- library ----------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct LibraryFile {
    format: String,
    version: u32,
    brushes: Vec<LibraryEntry>,
}

#[derive(Serialize, Deserialize)]
struct LibraryEntry {
    #[serde(flatten)]
    brush: CustomBrush,
    tips: Vec<String>,
}

/// The user's brush library (persisted in the data directory).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrushLibrary {
    pub brushes: Vec<CustomBrush>,
}

impl BrushLibrary {
    pub fn find(&self, id: &str) -> Option<&CustomBrush> {
        self.brushes.iter().find(|b| b.id == id)
    }
    pub fn find_mut(&mut self, id: &str) -> Option<&mut CustomBrush> {
        self.brushes.iter_mut().find(|b| b.id == id)
    }
    /// Adds brushes; returns how many were added (duplicates by name + identical tips
    /// are skipped).
    pub fn add(&mut self, v: Vec<CustomBrush>) -> usize {
        let mut n = 0;
        for b in v {
            if self
                .brushes
                .iter()
                .any(|o| o.name == b.name && o.tips == b.tips)
            {
                continue;
            }
            let mut b = b;
            while self.brushes.iter().any(|o| o.id == b.id) {
                b.id = new_id();
            }
            self.brushes.push(b);
            n += 1;
        }
        n
    }
    pub fn remove(&mut self, id: &str) -> Option<CustomBrush> {
        let i = self.brushes.iter().position(|b| b.id == id)?;
        Some(self.brushes.remove(i))
    }
    pub fn categories(&self) -> Vec<String> {
        let mut c: Vec<String> = self
            .brushes
            .iter()
            .map(|b| b.category.clone())
            .filter(|c| !c.is_empty())
            .collect();
        c.sort();
        c.dedup();
        c
    }

    /// Writes `library.json` plus one PNG per tip into `dir` (atomically per file);
    /// stale tip files are removed.
    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        let mut entries = Vec::new();
        let mut keep = std::collections::HashSet::new();
        for b in &self.brushes {
            let id = sanitize_id(&b.id);
            let mut names = Vec::new();
            for (i, t) in b.tips.iter().enumerate() {
                let n = format!("{id}_{i}.png");
                let p = dir.join(&n);
                if !p.exists() {
                    crate::write_atomic(&p, &encode_tip_png(t)?)?;
                }
                keep.insert(n.clone());
                names.push(n);
            }
            entries.push(LibraryEntry {
                brush: b.clone(),
                tips: names,
            });
        }
        let f = LibraryFile {
            format: "panoraminal-brushes".into(),
            version: 1,
            brushes: entries,
        };
        crate::write_atomic(
            &dir.join("library.json"),
            serde_json::to_string_pretty(&f)?.as_bytes(),
        )?;
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.ends_with(".png") && !keep.contains(&n) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        Ok(())
    }

    pub fn load(dir: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(dir.join("library.json"))?;
        let f: LibraryFile = serde_json::from_str(&s)?;
        if f.format != "panoraminal-brushes" {
            return Err(IoError::Invalid("not a brush library".into()));
        }
        let mut brushes = Vec::new();
        for e in f.brushes {
            let mut b = e.brush;
            for n in &e.tips {
                if n.contains('/') || n.contains('\\') || n.contains("..") {
                    continue;
                }
                if let Ok(bytes) = std::fs::read(dir.join(n)) {
                    if let Ok(t) = decode_tip_png(&bytes) {
                        b.tips.push(t);
                    }
                }
            }
            if !b.tips.is_empty() {
                b.dynamics.clamp();
                brushes.push(b);
            }
        }
        Ok(Self { brushes })
    }
}

fn sanitize_id(id: &str) -> String {
    let s: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    if s.is_empty() {
        "brush".into()
    } else {
        s
    }
}

/// Serializes brushes for embedding into a `.pano` project: `(json, [(file, png)])`.
pub fn embed_brushes(v: &[CustomBrush]) -> Result<(Vec<u8>, Vec<(String, Vec<u8>)>)> {
    let mut entries = Vec::new();
    let mut files = Vec::new();
    for (bi, b) in v.iter().enumerate() {
        let mut names = Vec::new();
        for (i, t) in b.tips.iter().enumerate() {
            let n = format!("brushes/{bi:03}_{i}.png");
            files.push((n.clone(), encode_tip_png(t)?));
            names.push(n);
        }
        entries.push(LibraryEntry {
            brush: b.clone(),
            tips: names,
        });
    }
    let f = LibraryFile {
        format: "panoraminal-brushes".into(),
        version: 1,
        brushes: entries,
    };
    Ok((serde_json::to_vec_pretty(&f)?, files))
}

/// Inverse of [`embed_brushes`]; `get` returns the bytes of an archive entry.
pub fn unembed_brushes(
    json: &[u8],
    mut get: impl FnMut(&str) -> Option<Vec<u8>>,
) -> Result<Vec<CustomBrush>> {
    let f: LibraryFile = serde_json::from_slice(json)?;
    let mut out = Vec::new();
    for e in f.brushes.into_iter().take(500) {
        let mut b = e.brush;
        for n in &e.tips {
            if let Some(bytes) = get(n) {
                if let Ok(t) = decode_tip_png(&bytes) {
                    b.tips.push(t);
                }
            }
        }
        if !b.tips.is_empty() {
            b.dynamics.clamp();
            out.push(b);
        }
    }
    Ok(out)
}

// ---- GIMP brushes -----------------------------------------------------------------

fn be32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// Parses one GIMP brush (`.gbr`) at the start of `b`; returns it and its length.
fn parse_gbr_one(b: &[u8]) -> Result<(String, TipImage, f32, bool, usize)> {
    let bad = || IoError::Invalid("GBR: invalid brush".into());
    let header = be32(b, 0).ok_or_else(bad)? as usize;
    let version = be32(b, 4).ok_or_else(bad)?;
    let w = be32(b, 8).ok_or_else(bad)?;
    let h = be32(b, 12).ok_or_else(bad)?;
    let bytes = be32(b, 16).ok_or_else(bad)?;
    if !(1..=4).contains(&version) || w == 0 || h == 0 || w > 5000 || h > 5000 {
        return Err(bad());
    }
    let (spacing, name_at) = if version >= 2 {
        if b.get(20..24) != Some(b"GIMP") {
            return Err(bad());
        }
        (be32(b, 24).ok_or_else(bad)? as f32 / 100.0, 28)
    } else {
        (0.25, 20)
    };
    if header < name_at || header > b.len() {
        return Err(bad());
    }
    let name = String::from_utf8_lossy(&b[name_at..header])
        .trim_end_matches('\0')
        .to_string();
    let bpp = bytes as usize;
    let n = w as usize * h as usize * bpp;
    let data = b.get(header..header + n).ok_or_else(bad)?;
    let (mask, color) = match bpp {
        1 => (data.to_vec(), false),
        2 => (data.chunks_exact(2).map(|c| c[0]).collect(), false),
        4 => (data.chunks_exact(4).map(|c| c[3]).collect(), true),
        _ => return Err(bad()),
    };
    Ok((
        name,
        TipImage::new(w, h, mask),
        spacing.max(0.01),
        color,
        header + n,
    ))
}

pub fn parse_gbr(b: &[u8], file_name: &str) -> Result<CustomBrush> {
    let (name, tip, spacing, color, _) = parse_gbr_one(b)?;
    let name = if name.trim().is_empty() {
        file_name.to_string()
    } else {
        name
    };
    let dia = tip.max_side() as f32;
    let mut br = CustomBrush::new(name, vec![tip.trimmed()], dia);
    br.dynamics.spacing = spacing;
    if color {
        br.notices.push(Notice::ColorTip);
    }
    br.source = format!("{file_name}.gbr");
    Ok(br)
}

/// GIMP image pipe (`.gih`): a name line, a parameter line, then N `.gbr` images.
pub fn parse_gih(b: &[u8], file_name: &str) -> Result<CustomBrush> {
    let bad = || IoError::Invalid("GIH: invalid brush pipe".into());
    let nl1 = b.iter().position(|&c| c == b'\n').ok_or_else(bad)?;
    let nl2 = nl1
        + 1
        + b[nl1 + 1..]
            .iter()
            .position(|&c| c == b'\n')
            .ok_or_else(bad)?;
    let name = String::from_utf8_lossy(&b[..nl1]).trim().to_string();
    let params = String::from_utf8_lossy(&b[nl1 + 1..nl2]).to_string();
    let count: usize = params
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(bad)?;
    let random = params.contains("random");
    let mut pos = nl2 + 1;
    let mut tips = Vec::new();
    let mut spacing = 0.25;
    let mut color = false;
    for _ in 0..count.min(256) {
        let Ok((_, t, sp, c, len)) = parse_gbr_one(&b[pos..]) else {
            break;
        };
        tips.push(t.trimmed());
        spacing = sp;
        color |= c;
        pos += len;
        if pos >= b.len() {
            break;
        }
    }
    if tips.is_empty() {
        return Err(bad());
    }
    let dia = tips.iter().map(|t| t.max_side()).max().unwrap_or(32) as f32;
    let mut br = CustomBrush::new(
        if name.is_empty() {
            file_name.to_string()
        } else {
            name
        },
        tips,
        dia,
    );
    br.dynamics.spacing = spacing;
    br.dynamics.selection = if random {
        panoraminal_core::tip::TipSelection::Random
    } else {
        panoraminal_core::tip::TipSelection::Sequential
    };
    if color {
        br.notices.push(Notice::ColorTip);
    }
    br.source = format!("{file_name}.gih");
    Ok(br)
}

/// A tip from an ordinary image: transparency is used as the mask when the image has
/// any, otherwise darkness (black = full paint).
pub fn tip_from_image_bytes(b: &[u8], file_name: &str) -> Result<CustomBrush> {
    let img = image::load_from_memory(b)?.to_rgba8();
    let (w, h) = img.dimensions();
    let px = img.into_raw();
    let has_alpha = px.chunks_exact(4).any(|p| p[3] < 250);
    let mask: Vec<u8> = px
        .chunks_exact(4)
        .map(|p| {
            if has_alpha {
                p[3]
            } else {
                let l = 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32;
                (255.0 - l).round().clamp(0.0, 255.0) as u8
            }
        })
        .collect();
    let tip = TipImage::new(w, h, mask).limited(1024).trimmed();
    let dia = tip.max_side().min(500) as f32;
    let mut br = CustomBrush::new(file_name, vec![tip], dia);
    br.source = file_name.to_string();
    Ok(br)
}

/// Extensions accepted by [`import_brush_file`].
pub const BRUSH_EXTS: &[&str] = &["abr", "gbr", "gih", "png", "jpg", "jpeg", "webp", "bmp"];

/// Imports every brush in a file, dispatching on the extension.
pub fn import_brush_bytes(bytes: &[u8], path: &Path) -> Result<Vec<CustomBrush>> {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Brush".into());
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut v = match ext.as_str() {
        "abr" => crate::abr::parse_abr(bytes, &stem)?,
        "gbr" => vec![parse_gbr(bytes, &stem)?],
        "gih" => vec![parse_gih(bytes, &stem)?],
        _ => vec![tip_from_image_bytes(bytes, &stem)?],
    };
    for b in &mut v {
        if b.category.is_empty() {
            b.category = stem.chars().take(40).collect();
        }
        for t in &mut b.tips {
            *t = t.limited(2048);
        }
    }
    Ok(v)
}

pub fn import_brush_file(path: &Path) -> Result<Vec<CustomBrush>> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > 256 * 1024 * 1024 {
        return Err(IoError::Invalid("file too large".into()));
    }
    import_brush_bytes(&std::fs::read(path)?, path)
}
