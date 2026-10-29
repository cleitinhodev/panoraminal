//! # panoraminal-io
//!
//! * Import of equirectangular images (PNG, JPEG, WebP, BMP, TIFF).
//! * Export to PNG and JPEG; JPEGs get a Google Photo Sphere (GPano) XMP packet so
//!   Facebook, Google Photos and other 360° viewers recognise them as panoramas.
//! * The `.pano` project format: a ZIP archive with a JSON manifest and one PNG per layer.
//! * Import of legacy Paint360 projects (`.p360.json`).

mod project;
mod xmp;

pub use project::{
    load_project, load_project_bytes, save_project, LayerSnapshot, ProjectSnapshot, PROJECT_EXT,
};
pub use xmp::{gpano_xmp, has_gpano, insert_xmp_into_jpeg};

use image::{ImageEncoder, RgbaImage};
use panoraminal_core::{Document, MAX_DIM};
use std::io::Write;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum IoError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("image error: {0}")]
    Image(#[from] image::ImageError),
    #[error("archive error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("invalid project: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}
pub type Result<T> = std::result::Result<T, IoError>;

/// A decoded image ready to become a document layer.
pub struct ImportedImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    /// True if the image was downscaled to fit [`MAX_DIM`] (or the GPU limit).
    pub downscaled: bool,
    pub had_gpano: bool,
}

/// Decodes an image, downscaling it so neither side exceeds `max_dim`.
pub fn import_image_bytes(bytes: &[u8], max_dim: u32) -> Result<ImportedImage> {
    let max_dim = max_dim.clamp(16, MAX_DIM);
    let img = image::load_from_memory(bytes)?;
    let had_gpano = has_gpano(bytes);
    let mut rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut downscaled = false;
    if w > max_dim || h > max_dim {
        let k = (max_dim as f64 / w as f64).min(max_dim as f64 / h as f64);
        let nw = ((w as f64 * k).round() as u32).max(1);
        let nh = ((h as f64 * k).round() as u32).max(1);
        rgba = image::imageops::resize(&rgba, nw, nh, image::imageops::FilterType::Lanczos3);
        downscaled = true;
    }
    let (width, height) = rgba.dimensions();
    if width < 16 || height < 8 {
        return Err(IoError::Invalid("image too small".into()));
    }
    Ok(ImportedImage {
        width,
        height,
        pixels: rgba.into_raw(),
        downscaled,
        had_gpano,
    })
}

pub fn import_image(path: &Path, max_dim: u32) -> Result<ImportedImage> {
    import_image_bytes(&std::fs::read(path)?, max_dim)
}

/// Encodes straight RGBA8 pixels as PNG.
pub fn encode_png(pixels: &[u8], w: u32, h: u32, fast: bool) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let ct = if fast {
        image::codecs::png::CompressionType::Fast
    } else {
        image::codecs::png::CompressionType::Default
    };
    image::codecs::png::PngEncoder::new_with_quality(
        &mut out,
        ct,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(pixels, w, h, image::ExtendedColorType::Rgba8)?;
    Ok(out)
}

/// Encodes straight RGBA8 pixels (alpha is dropped — flatten first) as JPEG,
/// optionally embedding the GPano XMP packet.
pub fn encode_jpeg(pixels: &[u8], w: u32, h: u32, quality: u8, gpano: bool) -> Result<Vec<u8>> {
    let rgb: Vec<u8> = pixels
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
        .write_image(&rgb, w, h, image::ExtendedColorType::Rgb8)?;
    if gpano {
        out = insert_xmp_into_jpeg(&out, &gpano_xmp(w, h))?;
    }
    Ok(out)
}

/// Writes `bytes` to `path` atomically (temporary file + rename).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        // Windows can refuse to replace an existing file in some situations.
        let _ = std::fs::remove_file(path);
        std::fs::rename(&tmp, path).map_err(|_| e)?;
    }
    Ok(())
}

/// Export format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Png,
    Jpeg { quality: u8, gpano: bool },
}

/// Flattens and encodes a document (convenience for tests / CLI).
pub fn export_document(doc: &Document, fmt: ExportFormat) -> Result<Vec<u8>> {
    match fmt {
        ExportFormat::Png => encode_png(&doc.flatten(None), doc.width, doc.height, false),
        ExportFormat::Jpeg { quality, gpano } => encode_jpeg(
            &doc.flatten(Some([255, 255, 255])),
            doc.width,
            doc.height,
            quality,
            gpano,
        ),
    }
}

/// Encodes an already flattened buffer for `fmt`.
pub fn encode_flat(flat: &[u8], w: u32, h: u32, fmt: ExportFormat) -> Result<Vec<u8>> {
    match fmt {
        ExportFormat::Png => encode_png(flat, w, h, false),
        ExportFormat::Jpeg { quality, gpano } => encode_jpeg(flat, w, h, quality, gpano),
    }
}

/// Decodes PNG/JPEG bytes to an `RgbaImage` (used for icons and tests).
pub fn decode_rgba(bytes: &[u8]) -> Result<RgbaImage> {
    Ok(image::load_from_memory(bytes)?.to_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoraminal_core::document::Background;

    #[test]
    fn png_roundtrip() {
        let d = Document::new_blank(64, 32, Background::Sky, "bg", "l1");
        let png = export_document(&d, ExportFormat::Png).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let img = import_image_bytes(&png, 8192).unwrap();
        assert_eq!((img.width, img.height), (64, 32));
        assert_eq!(img.pixels, d.flatten(None));
    }

    #[test]
    fn jpeg_has_gpano_and_decodes() {
        let d = Document::new_blank(128, 64, Background::Sky, "bg", "l1");
        let jpg = export_document(
            &d,
            ExportFormat::Jpeg {
                quality: 90,
                gpano: true,
            },
        )
        .unwrap();
        assert_eq!(&jpg[0..2], &[0xFF, 0xD8]);
        assert_eq!(&jpg[jpg.len() - 2..], &[0xFF, 0xD9]);
        assert!(has_gpano(&jpg));
        let s = String::from_utf8_lossy(&jpg);
        assert!(s.contains("<GPano:ProjectionType>equirectangular</GPano:ProjectionType>"));
        assert!(s.contains("<GPano:FullPanoWidthPixels>128</GPano:FullPanoWidthPixels>"));
        let img = import_image_bytes(&jpg, 8192).unwrap();
        assert_eq!((img.width, img.height), (128, 64));
        assert!(img.had_gpano);
        let plain = export_document(
            &d,
            ExportFormat::Jpeg {
                quality: 90,
                gpano: false,
            },
        )
        .unwrap();
        assert!(!has_gpano(&plain));
    }

    #[test]
    fn import_downscales() {
        let d = Document::new_blank(256, 128, Background::White, "bg", "l1");
        let png = export_document(&d, ExportFormat::Png).unwrap();
        let img = import_image_bytes(&png, 64).unwrap();
        assert!(img.downscaled);
        assert_eq!((img.width, img.height), (64, 32));
    }

    #[test]
    fn atomic_write() {
        let dir = std::env::temp_dir().join(format!("pano-io-test-{}", std::process::id()));
        let p = dir.join("a.bin");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        std::fs::remove_dir_all(dir).ok();
    }
}
