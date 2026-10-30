//! Photoshop brush (`.abr`) import.
//!
//! Supported layouts:
//!
//! * **v1 / v2** (Photoshop ≤ 6): a `u16` brush count followed by brush records;
//!   type 1 = *computed* (diameter, roundness, angle, hardness → we synthesize an
//!   elliptical tip), type 2 = *sampled* (an 8-bit grayscale image, raw or PackBits),
//!   v2 adds a UTF-16 name.
//! * **v6 / v7 / v10** (Photoshop 7 – CC): a sequence of `8BIM` sections. `samp`
//!   holds the sampled tips (UUID key + image), `desc` an Action-descriptor tree
//!   with the presets: name, diameter, angle, roundness, spacing, hardness, flip,
//!   shape dynamics (size/angle/roundness jitter, pen-pressure control), scattering,
//!   count and transfer (opacity/flow jitter). `patt` (textures) and other sections
//!   are skipped.
//!
//! Features with no equivalent in Panoraminal (dual brush, texture, bristle and
//! erodible tips, colour dynamics, wet edges, noise) are reported as
//! [`Notice`]s; the closest supported approximation is used.

use crate::brushes::{CustomBrush, Notice};
use crate::{IoError, Result};
use panoraminal_core::tip::TipImage;
use std::collections::HashMap;

/// Largest accepted tip side (Photoshop allows 5000 px).
const MAX_TIP: u32 = 5000;
/// Largest number of brushes read from one file.
const MAX_BRUSHES: usize = 2000;

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

fn bad(msg: &str) -> IoError {
    IoError::Invalid(format!("ABR: {msg}"))
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, pos: 0 }
    }
    fn left(&self) -> usize {
        self.b.len().saturating_sub(self.pos)
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.left() < n {
            return Err(bad("unexpected end of file"));
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn i64(&mut self) -> Result<i64> {
        let b = self.bytes(8)?;
        Ok(i64::from_be_bytes(b.try_into().unwrap()))
    }
    fn f64(&mut self) -> Result<f64> {
        let b = self.bytes(8)?;
        Ok(f64::from_be_bytes(b.try_into().unwrap()))
    }
    fn tag(&mut self) -> Result<[u8; 4]> {
        Ok(self.bytes(4)?.try_into().unwrap())
    }
    /// UTF-16BE string with a `u32` character count (a trailing NUL is dropped).
    fn ucs2(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        if n > 1 << 20 {
            return Err(bad("string too long"));
        }
        let b = self.bytes(n * 2)?;
        let units: Vec<u16> = b
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        let mut s = String::from_utf16_lossy(&units);
        while s.ends_with('\0') {
            s.pop();
        }
        Ok(s)
    }
    /// Descriptor "class ID / key": `u32` length, 0 means a 4-character code.
    fn key(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        let n = if n == 0 { 4 } else { n };
        if n > 1 << 16 {
            return Err(bad("key too long"));
        }
        Ok(String::from_utf8_lossy(self.bytes(n)?).into_owned())
    }
}

// ---- descriptors ------------------------------------------------------------------

/// A value of an Action descriptor.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Obj(Descriptor),
    List(Vec<Value>),
    Double(f64),
    /// Unit code (`#Prc`, `#Pxl`, `#Ang`…) and value.
    Unit(String, f64),
    Text(String),
    Enum(String, String),
    Int(i64),
    Bool(bool),
    Class(String),
    Raw,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Descriptor {
    pub class: String,
    pub items: Vec<(String, Value)>,
}

impl Descriptor {
    pub fn get(&self, k: &str) -> Option<&Value> {
        self.items.iter().find(|(n, _)| n == k).map(|(_, v)| v)
    }
    pub fn obj(&self, k: &str) -> Option<&Descriptor> {
        match self.get(k) {
            Some(Value::Obj(d)) => Some(d),
            _ => None,
        }
    }
    pub fn num(&self, k: &str) -> Option<f64> {
        match self.get(k)? {
            Value::Double(v) | Value::Unit(_, v) => Some(*v),
            Value::Int(v) => Some(*v as f64),
            _ => None,
        }
    }
    pub fn boolean(&self, k: &str) -> Option<bool> {
        match self.get(k)? {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn text(&self, k: &str) -> Option<&str> {
        match self.get(k)? {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }
}

fn read_descriptor(r: &mut Reader, depth: u32) -> Result<Descriptor> {
    if depth > 32 {
        return Err(bad("descriptor nested too deeply"));
    }
    let _name = r.ucs2()?;
    let class = r.key()?;
    let n = r.u32()? as usize;
    if n > 100_000 {
        return Err(bad("too many descriptor items"));
    }
    let mut items = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        let k = r.key()?;
        let v = read_value(r, depth)?;
        items.push((k, v));
    }
    Ok(Descriptor { class, items })
}

fn read_value(r: &mut Reader, depth: u32) -> Result<Value> {
    let t = r.tag()?;
    Ok(match &t {
        b"Objc" | b"GlbO" => Value::Obj(read_descriptor(r, depth + 1)?),
        b"VlLs" => {
            let n = r.u32()? as usize;
            if n > 100_000 {
                return Err(bad("list too long"));
            }
            let mut v = Vec::with_capacity(n.min(4096));
            for _ in 0..n {
                v.push(read_value(r, depth + 1)?);
            }
            Value::List(v)
        }
        b"doub" => Value::Double(r.f64()?),
        b"UntF" => {
            let u = String::from_utf8_lossy(&r.tag()?).into_owned();
            Value::Unit(u, r.f64()?)
        }
        b"UnFl" => {
            let u = String::from_utf8_lossy(&r.tag()?).into_owned();
            let n = r.u32()? as usize;
            let mut first = 0.0;
            for i in 0..n {
                let v = r.f64()?;
                if i == 0 {
                    first = v;
                }
            }
            Value::Unit(u, first)
        }
        b"TEXT" => Value::Text(r.ucs2()?),
        b"enum" => {
            let ty = r.key()?;
            Value::Enum(ty, r.key()?)
        }
        b"long" => Value::Int(r.i32()? as i64),
        b"comp" => Value::Int(r.i64()?),
        b"bool" => Value::Bool(r.u8()? != 0),
        b"type" | b"GlbC" => {
            let _ = r.ucs2()?;
            Value::Class(r.key()?)
        }
        b"alis" | b"tdta" | b"Pth " => {
            let n = r.u32()? as usize;
            r.skip(n)?;
            Value::Raw
        }
        b"obj " => {
            let n = r.u32()? as usize;
            for _ in 0..n.min(1000) {
                let kind = r.tag()?;
                match &kind {
                    b"prop" => {
                        r.ucs2()?;
                        r.key()?;
                        r.key()?;
                    }
                    b"Clss" => {
                        r.ucs2()?;
                        r.key()?;
                    }
                    b"Enmr" => {
                        r.ucs2()?;
                        r.key()?;
                        r.key()?;
                        r.key()?;
                    }
                    b"rele" => {
                        r.ucs2()?;
                        r.key()?;
                        r.u32()?;
                    }
                    b"Idnt" | b"indx" => {
                        r.u32()?;
                    }
                    b"name" => {
                        r.ucs2()?;
                        r.key()?;
                        r.ucs2()?;
                    }
                    _ => return Err(bad("unknown reference item")),
                }
            }
            Value::Raw
        }
        _ => {
            return Err(bad(&format!(
                "unknown descriptor type {:?}",
                String::from_utf8_lossy(&t)
            )))
        }
    })
}

// ---- image data -------------------------------------------------------------------

/// PackBits decoding of `height` rows (each preceded by a `u16` byte count table).
fn read_rle(r: &mut Reader, row_bytes: usize, height: usize) -> Result<Vec<u8>> {
    let mut lens = Vec::with_capacity(height);
    for _ in 0..height {
        lens.push(r.u16()? as usize);
    }
    let mut out = Vec::with_capacity(row_bytes * height);
    for len in lens {
        let row = r.bytes(len)?;
        let start = out.len();
        let mut i = 0;
        while i < row.len() && out.len() - start < row_bytes {
            let n = row[i] as i8;
            i += 1;
            if n >= 0 {
                let c = n as usize + 1;
                let end = (i + c).min(row.len());
                out.extend_from_slice(&row[i..end]);
                i = end;
            } else if n != -128 {
                let c = (-(n as i32)) as usize + 1;
                let v = *row.get(i).ok_or_else(|| bad("bad RLE data"))?;
                i += 1;
                out.extend(std::iter::repeat_n(v, c));
            }
        }
        out.resize(start + row_bytes, 0);
    }
    Ok(out)
}

/// Reads a sampled tip: bounds (top, left, bottom, right), depth, compression, data.
fn read_sample(r: &mut Reader) -> Result<TipImage> {
    let top = r.i32()?;
    let left = r.i32()?;
    let bottom = r.i32()?;
    let right = r.i32()?;
    let depth = r.u16()?;
    let compress = r.u8()?;
    let w = right.checked_sub(left).ok_or_else(|| bad("bad bounds"))?;
    let h = bottom.checked_sub(top).ok_or_else(|| bad("bad bounds"))?;
    if w <= 0 || h <= 0 || w as u32 > MAX_TIP || h as u32 > MAX_TIP {
        return Err(bad(&format!("unsupported tip size {w}×{h}")));
    }
    let (w, h) = (w as usize, h as usize);
    let bpp = match depth {
        8 => 1,
        16 => 2,
        _ => return Err(bad(&format!("unsupported depth {depth}"))),
    };
    let raw = if compress == 0 {
        r.bytes(w * h * bpp)?.to_vec()
    } else {
        read_rle(r, w * bpp, h)?
    };
    let data = if bpp == 2 {
        raw.chunks_exact(2).map(|c| c[0]).collect()
    } else {
        raw
    };
    Ok(TipImage::new(w as u32, h as u32, data))
}

// ---- v1 / v2 ----------------------------------------------------------------------

fn parse_v12(r: &mut Reader, version: u16, file_name: &str) -> Result<Vec<CustomBrush>> {
    let count = r.u16()? as usize;
    let mut out = Vec::new();
    for i in 0..count.min(MAX_BRUSHES) {
        if r.left() < 6 {
            break;
        }
        let ty = r.u16()?;
        let size = r.u32()? as usize;
        let start = r.pos;
        let next = start.saturating_add(size);
        let body = (|| -> Result<Option<CustomBrush>> {
            match ty {
                1 => {
                    let _misc = r.u32()?;
                    let spacing = r.u16()?;
                    let diameter = r.u16()?.max(1);
                    let roundness = r.u16()?;
                    let angle = r.i16()?;
                    let hardness = r.u16()?;
                    let tip = TipImage::ellipse(
                        diameter as u32,
                        1.0,
                        (hardness as f32 / 100.0).clamp(0.0, 1.0),
                    );
                    let mut b = CustomBrush::new(
                        format!("{file_name} {}", i + 1),
                        vec![tip],
                        diameter as f32,
                    );
                    b.dynamics.spacing = (spacing.max(1) as f32 / 100.0).clamp(0.01, 10.0);
                    b.dynamics.roundness = (roundness as f32 / 100.0).clamp(0.01, 1.0);
                    b.dynamics.angle = angle as f32;
                    Ok(Some(b))
                }
                2 => {
                    let _misc = r.u32()?;
                    let spacing = r.u16()?;
                    let mut name = String::new();
                    if version == 2 {
                        name = r.ucs2()?;
                    }
                    let _aa = r.u8()?;
                    r.skip(8)?; // short bounds
                    let tip = read_sample(r)?;
                    if name.trim().is_empty() {
                        name = format!("{file_name} {}", i + 1);
                    }
                    let dia = tip.max_side() as f32;
                    let mut b = CustomBrush::new(name, vec![tip], dia);
                    b.dynamics.spacing = (spacing.max(1) as f32 / 100.0).clamp(0.01, 10.0);
                    Ok(Some(b))
                }
                _ => Ok(None),
            }
        })();
        match body {
            Ok(Some(b)) => out.push(b),
            Ok(None) => {}
            Err(e) if out.is_empty() && i + 1 == count => return Err(e),
            Err(_) => {}
        }
        if next > r.b.len() {
            break;
        }
        r.pos = next;
    }
    Ok(out)
}

// ---- v6+ --------------------------------------------------------------------------

fn parse_samp(sec: &[u8], subversion: u16) -> Vec<(String, TipImage)> {
    let mut r = Reader::new(sec);
    let mut out = Vec::new();
    while r.left() >= 4 && out.len() < MAX_BRUSHES {
        let Ok(size) = r.u32() else { break };
        let size = size as usize;
        let start = r.pos;
        let padded = size.div_ceil(4) * 4;
        let next = start.saturating_add(padded);
        let one = (|| -> Result<(String, TipImage)> {
            let klen = r.u8()? as usize;
            let key = String::from_utf8_lossy(r.bytes(klen)?).into_owned();
            // Skip to the image header: 47 (sub 1) or 301 (sub 2) bytes from the
            // start of the record, counting the Pascal key.
            let skip = if subversion == 1 { 47 } else { 301 };
            r.pos = start + skip;
            let tip = read_sample(&mut r)?;
            Ok((key, tip))
        })();
        if let Ok(x) = one {
            out.push(x);
        }
        if next > sec.len() || padded == 0 {
            break;
        }
        r.pos = next;
    }
    out
}

/// Unit-aware percentage (0–1) from a descriptor value.
fn pct(d: &Descriptor, k: &str) -> Option<f32> {
    d.num(k).map(|v| (v / 100.0) as f32)
}

/// A `brVr` dynamics object: `(control, jitter 0–1, minimum 0–1)`.
fn var(d: &Descriptor, k: &str) -> Option<(i64, f32, f32)> {
    let o = d.obj(k)?;
    let ctl = match o.get("bVTy") {
        Some(Value::Int(v)) => *v,
        _ => 0,
    };
    Some((
        ctl,
        pct(o, "jitter").unwrap_or(0.0),
        pct(o, "Mnm ").unwrap_or(0.0),
    ))
}

const CTL_PRESSURE: i64 = 2;
const CTL_INITIAL_DIRECTION: i64 = 6;
const CTL_DIRECTION: i64 = 7;

fn preset_to_brush(
    p: &Descriptor,
    samples: &HashMap<String, TipImage>,
    file_name: &str,
    index: usize,
) -> Option<CustomBrush> {
    let tip_desc = p.obj("Brsh")?;
    let name = p
        .text("Nm  ")
        .map(str::to_owned)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{file_name} {}", index + 1));
    let mut notices = Vec::new();
    let diameter = tip_desc.num("Dmtr").unwrap_or(0.0) as f32;
    let tip = if let Some(uuid) = tip_desc.text("sampledData") {
        samples.get(uuid)?.clone()
    } else {
        match tip_desc.class.as_str() {
            "computedBrush" => {}
            c if c.contains("ristle") => notices.push(Notice::Bristle),
            c if c.contains("rodible") => notices.push(Notice::Erodible),
            _ => notices.push(Notice::Computed),
        }
        let d = diameter.clamp(1.0, 2500.0);
        TipImage::ellipse(
            d.round() as u32,
            1.0,
            pct(tip_desc, "Hrdn").unwrap_or(1.0).clamp(0.0, 1.0),
        )
    };
    let dia = if diameter >= 1.0 {
        diameter
    } else {
        tip.max_side() as f32
    };
    let mut b = CustomBrush::new(name, vec![tip], dia);
    // Photoshop presets only use pen pressure when their dynamics say so.
    b.pressure_size = false;
    let dy = &mut b.dynamics;
    let spacing_on = tip_desc.boolean("Intr").unwrap_or(true);
    dy.spacing = if spacing_on {
        pct(tip_desc, "Spcn").unwrap_or(0.25)
    } else {
        0.25
    };
    dy.angle = tip_desc.num("Angl").unwrap_or(0.0) as f32;
    dy.roundness = pct(tip_desc, "Rndn").unwrap_or(1.0);
    dy.flip_x = tip_desc.boolean("flipX").unwrap_or(false);
    dy.flip_y = tip_desc.boolean("flipY").unwrap_or(false);

    if p.boolean("useTipDynamics").unwrap_or(false) {
        if let Some((ctl, jit, min)) = var(p, "szVr") {
            dy.size_jitter = jit;
            if ctl == CTL_PRESSURE {
                b.pressure_size = true;
                b.min_size = pct(p, "minimumDiameter").unwrap_or(min);
            }
            dy.min_diameter = pct(p, "minimumDiameter").unwrap_or(min);
        }
        if let Some((ctl, jit, _)) = var(p, "angleDynamics") {
            dy.angle_jitter = jit;
            if ctl == CTL_DIRECTION || ctl == CTL_INITIAL_DIRECTION {
                dy.follow_direction = true;
            }
        }
        if let Some((_, jit, _)) = var(p, "roundnessDynamics") {
            dy.roundness_jitter = jit;
            dy.min_roundness = pct(p, "minimumRoundness").unwrap_or(0.25);
        }
        dy.flip_x_jitter = p.boolean("flipX").unwrap_or(false);
        dy.flip_y_jitter = p.boolean("flipY").unwrap_or(false);
    }
    if p.boolean("useScatter").unwrap_or(false) {
        if let Some((_, jit, _)) = var(p, "scatterDynamics") {
            dy.scatter = jit;
        }
        dy.scatter_both_axes = p.boolean("bothAxes").unwrap_or(false);
        dy.count = p.num("Cnt ").unwrap_or(1.0).round().clamp(1.0, 16.0) as u32;
        if let Some((_, jit, _)) = var(p, "countDynamics") {
            dy.count_jitter = jit;
        }
    }
    if p.boolean("usePaintDynamics").unwrap_or(false) {
        if let Some((ctl, jit, _)) = var(p, "opVr") {
            dy.opacity_jitter = jit;
            if ctl == CTL_PRESSURE {
                b.pressure_opacity = true;
            }
        }
        if let Some((ctl, jit, _)) = var(p, "prVr") {
            dy.flow_jitter = jit;
            if ctl == CTL_PRESSURE {
                b.pressure_opacity = true;
            }
        }
    }
    for (key, notice) in [
        ("useDualBrush", Notice::DualBrush),
        ("useTexture", Notice::Texture),
        ("useColorDynamics", Notice::ColorDynamics),
        ("Wtdg", Notice::WetEdges),
        ("Nose", Notice::Noise),
    ] {
        if p.boolean(key).unwrap_or(false) {
            notices.push(notice);
        }
    }
    b.dynamics.clamp();
    for n in notices {
        if !b.notices.contains(&n) {
            b.notices.push(n);
        }
    }
    Some(b)
}

fn parse_v6(r: &mut Reader, file_name: &str) -> Result<Vec<CustomBrush>> {
    let sub = r.u16()?;
    if sub != 1 && sub != 2 {
        return Err(bad(&format!("unsupported subversion {sub}")));
    }
    let mut samples: Vec<(String, TipImage)> = Vec::new();
    let mut desc: Option<Descriptor> = None;
    let mut extra = Vec::new();
    while r.left() >= 12 {
        let sig = r.tag()?;
        if &sig != b"8BIM" && &sig != b"8B64" {
            break;
        }
        let key = r.tag()?;
        let len = r.u32()? as usize;
        let len = len.min(r.left());
        let sec = r.bytes(len)?;
        // sections are padded to even (some writers) — tolerate both
        match &key {
            b"samp" => samples = parse_samp(sec, sub),
            b"desc" => {
                let mut dr = Reader::new(sec);
                let parsed = (|| -> Result<Descriptor> {
                    let _ver = dr.u32()?;
                    read_descriptor(&mut dr, 0)
                })();
                if let Ok(d) = parsed {
                    desc = Some(d);
                }
            }
            b"patt" => extra.push(Notice::Texture),
            _ => {}
        }
    }
    let map: HashMap<String, TipImage> = samples.iter().cloned().collect();
    let mut out = Vec::new();
    let mut used = std::collections::HashSet::new();
    if let Some(d) = &desc {
        if let Some(Value::List(list)) = d.get("Brsh") {
            for (i, v) in list.iter().enumerate().take(MAX_BRUSHES) {
                if let Value::Obj(p) = v {
                    if let Some(b) = preset_to_brush(p, &map, file_name, i) {
                        if let Some(u) = p.obj("Brsh").and_then(|t| t.text("sampledData")) {
                            used.insert(u.to_owned());
                        }
                        out.push(b);
                    }
                }
            }
        }
    }
    // Samples without a preset (or files without a descriptor).
    for (i, (key, tip)) in samples.into_iter().enumerate() {
        if used.contains(&key) {
            continue;
        }
        let dia = tip.max_side() as f32;
        out.push(CustomBrush::new(
            format!("{file_name} {}", i + 1),
            vec![tip],
            dia,
        ));
    }
    let _ = extra;
    Ok(out)
}

/// Parses an `.abr` file. `file_name` names brushes that have no name of their own.
pub fn parse_abr(bytes: &[u8], file_name: &str) -> Result<Vec<CustomBrush>> {
    let mut r = Reader::new(bytes);
    let version = r.u16()?;
    let mut v = match version {
        1 | 2 => parse_v12(&mut r, version, file_name)?,
        6 | 7 | 10 => parse_v6(&mut r, file_name)?,
        _ => return Err(bad(&format!("unsupported version {version}"))),
    };
    for b in &mut v {
        b.source = format!("{file_name}.abr");
        for t in &mut b.tips {
            *t = t.trimmed();
        }
    }
    if v.is_empty() {
        return Err(bad("no brushes found"));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packbits() {
        // row of 6: repeat 0xAA ×3, then literal 1,2,3
        let mut f = vec![0u8, 6];
        f.extend_from_slice(&[0xFE, 0xAA, 0x02, 1, 2, 3]);
        let mut r = Reader::new(&f);
        assert_eq!(
            read_rle(&mut r, 6, 1).unwrap(),
            vec![0xAA, 0xAA, 0xAA, 1, 2, 3]
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_abr(b"", "x").is_err());
        assert!(parse_abr(&[0, 99, 0, 0], "x").is_err());
        assert!(parse_abr(&[0, 6, 0, 9], "x").is_err());
        // truncated v2 file
        assert!(parse_abr(&[0, 2, 0, 1, 0, 2, 0, 0, 0, 50, 1, 2], "x").is_err());
    }
}
