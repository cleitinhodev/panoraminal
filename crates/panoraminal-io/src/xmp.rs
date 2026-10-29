//! GPano (Google Photo Sphere) XMP metadata for equirectangular JPEGs.

use crate::{IoError, Result};

const XMP_NS: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

/// Builds the XMP packet describing a full 360°×180° equirectangular panorama.
pub fn gpano_xmp(w: u32, h: u32) -> String {
    format!(
        concat!(
            "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n",
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Panoraminal\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">",
            "<rdf:Description rdf:about=\"\" xmlns:GPano=\"http://ns.google.com/photos/1.0/panorama/\">",
            "<GPano:UsePanoramaViewer>True</GPano:UsePanoramaViewer>",
            "<GPano:ProjectionType>equirectangular</GPano:ProjectionType>",
            "<GPano:FullPanoWidthPixels>{w}</GPano:FullPanoWidthPixels><GPano:FullPanoHeightPixels>{h}</GPano:FullPanoHeightPixels>",
            "<GPano:CroppedAreaImageWidthPixels>{w}</GPano:CroppedAreaImageWidthPixels><GPano:CroppedAreaImageHeightPixels>{h}</GPano:CroppedAreaImageHeightPixels>",
            "<GPano:CroppedAreaLeftPixels>0</GPano:CroppedAreaLeftPixels><GPano:CroppedAreaTopPixels>0</GPano:CroppedAreaTopPixels>",
            "<GPano:PoseHeadingDegrees>0</GPano:PoseHeadingDegrees><GPano:InitialViewHeadingDegrees>180</GPano:InitialViewHeadingDegrees>",
            "<GPano:InitialViewPitchDegrees>0</GPano:InitialViewPitchDegrees><GPano:InitialHorizontalFOVDegrees>90</GPano:InitialHorizontalFOVDegrees>",
            "<GPano:StitchingSoftware>Panoraminal</GPano:StitchingSoftware>",
            "</rdf:Description></rdf:RDF></x:xmpmeta>\n<?xpacket end=\"w\"?>"
        ),
        w = w,
        h = h
    )
}

/// Inserts an APP1/XMP segment right after SOI (and after a JFIF APP0 if present).
pub fn insert_xmp_into_jpeg(jpeg: &[u8], xmp: &str) -> Result<Vec<u8>> {
    if jpeg.len() < 4 || jpeg[0] != 0xFF || jpeg[1] != 0xD8 {
        return Err(IoError::Invalid("not a JPEG".into()));
    }
    let len = 2 + XMP_NS.len() + xmp.len();
    if len > 0xFFFF {
        return Err(IoError::Invalid("XMP packet too large".into()));
    }
    let mut pos = 2;
    if jpeg[2] == 0xFF && jpeg[3] == 0xE0 && jpeg.len() > 6 {
        pos = 4 + ((jpeg[4] as usize) << 8 | jpeg[5] as usize);
    }
    let mut out = Vec::with_capacity(jpeg.len() + len + 2);
    out.extend_from_slice(&jpeg[..pos]);
    out.extend_from_slice(&[0xFF, 0xE1, (len >> 8) as u8, (len & 0xFF) as u8]);
    out.extend_from_slice(XMP_NS);
    out.extend_from_slice(xmp.as_bytes());
    out.extend_from_slice(&jpeg[pos..]);
    Ok(out)
}

/// Quick check whether a file carries a GPano equirectangular tag.
pub fn has_gpano(bytes: &[u8]) -> bool {
    let needle = b"GPano:ProjectionType";
    bytes
        .windows(needle.len())
        .take(256 * 1024)
        .any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_non_jpeg() {
        assert!(insert_xmp_into_jpeg(b"\x89PNG....", "x").is_err());
    }
    #[test]
    fn segment_length_is_correct() {
        let fake = [0xFF, 0xD8, 0xFF, 0xD9];
        let x = gpano_xmp(10, 5);
        let out = insert_xmp_into_jpeg(&fake, &x).unwrap();
        let len = (out[4] as usize) << 8 | out[5] as usize;
        assert_eq!(out[2..4], [0xFF, 0xE1]);
        assert_eq!(2 + len + 2, out.len() - 2);
    }
}
