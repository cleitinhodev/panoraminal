//! Layer blend modes.
//!
//! Colours are stored as *straight* (non-premultiplied) sRGB-encoded RGBA, and blending
//! happens in that encoded space, like Paint Tool SAI, Photoshop's default and the HTML
//! canvas. Compositing follows the W3C "Compositing and Blending Level 1" formula:
//!
//! ```text
//! co = αs·(1−αb)·Cs + αs·αb·B(Cb,Cs) + (1−αs)·αb·Cb
//! αo = αs + αb·(1−αs)
//! ```
//!
//! The WGSL compositing shader in `panoraminal-render` implements exactly the same maths;
//! the CPU version here is the reference used for export, merging and tests.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    /// Linear dodge (add), clamped.
    Add,
    Difference,
    SoftLight,
}

impl BlendMode {
    pub const ALL: [BlendMode; 9] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Add,
        BlendMode::Difference,
        BlendMode::SoftLight,
    ];
    /// Index used by the GPU shader (must match `composite.wgsl`).
    pub fn shader_index(self) -> u32 {
        Self::ALL.iter().position(|m| *m == self).unwrap() as u32
    }
    /// Stable identifier (also the i18n key suffix and the project-file value).
    pub fn id(self) -> &'static str {
        match self {
            BlendMode::Normal => "normal",
            BlendMode::Multiply => "multiply",
            BlendMode::Screen => "screen",
            BlendMode::Overlay => "overlay",
            BlendMode::Darken => "darken",
            BlendMode::Lighten => "lighten",
            BlendMode::Add => "add",
            BlendMode::Difference => "difference",
            BlendMode::SoftLight => "soft_light",
        }
    }
    pub fn from_id(s: &str) -> Option<BlendMode> {
        Self::ALL.iter().copied().find(|m| m.id() == s)
    }

    /// Separable blend function `B(Cb, Cs)` for one channel in `[0,1]`.
    #[inline]
    pub fn channel(self, cb: f32, cs: f32) -> f32 {
        match self {
            BlendMode::Normal => cs,
            BlendMode::Multiply => cb * cs,
            BlendMode::Screen => cb + cs - cb * cs,
            BlendMode::Overlay => hard_light(cs, cb),
            BlendMode::Darken => cb.min(cs),
            BlendMode::Lighten => cb.max(cs),
            BlendMode::Add => (cb + cs).min(1.0),
            BlendMode::Difference => (cb - cs).abs(),
            BlendMode::SoftLight => soft_light(cb, cs),
        }
    }
}

#[inline]
fn hard_light(cb: f32, cs: f32) -> f32 {
    // hard_light(Cb, Cs); overlay(Cb,Cs) = hard_light(Cs,Cb)
    if cs <= 0.5 {
        cb * 2.0 * cs
    } else {
        let s = 2.0 * cs - 1.0;
        cb + s - cb * s
    }
}

#[inline]
fn soft_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
    } else {
        let d = if cb <= 0.25 {
            ((16.0 * cb - 12.0) * cb + 4.0) * cb
        } else {
            cb.sqrt()
        };
        cb + (2.0 * cs - 1.0) * (d - cb)
    }
}

/// Composites straight-alpha `src` (with extra `opacity`) over straight-alpha `dst`.
#[inline]
pub fn composite(dst: [f32; 4], src: [f32; 4], opacity: f32, mode: BlendMode) -> [f32; 4] {
    let a_s = src[3] * opacity;
    if a_s <= 0.0 {
        return dst;
    }
    let a_b = dst[3];
    let a_o = a_s + a_b * (1.0 - a_s);
    if a_o <= 0.0 {
        return [0.0; 4];
    }
    let mut out = [0.0f32; 4];
    for i in 0..3 {
        let cs = src[i];
        let cb = dst[i];
        let b = mode.channel(cb, cs);
        let co = a_s * (1.0 - a_b) * cs + a_s * a_b * b + (1.0 - a_s) * a_b * cb;
        out[i] = (co / a_o).clamp(0.0, 1.0);
    }
    out[3] = a_o.clamp(0.0, 1.0);
    out
}

#[inline]
pub fn to_f(p: [u8; 4]) -> [f32; 4] {
    [
        p[0] as f32 / 255.0,
        p[1] as f32 / 255.0,
        p[2] as f32 / 255.0,
        p[3] as f32 / 255.0,
    ]
}
#[inline]
pub fn to_u8(p: [f32; 4]) -> [u8; 4] {
    [
        (p[0] * 255.0 + 0.5) as u8,
        (p[1] * 255.0 + 0.5) as u8,
        (p[2] * 255.0 + 0.5) as u8,
        (p[3] * 255.0 + 0.5) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const GREY: [f32; 4] = [0.5, 0.5, 0.5, 1.0];
    #[test]
    fn normal_opaque_replaces() {
        assert_eq!(composite(GREY, RED, 1.0, BlendMode::Normal), RED);
    }
    #[test]
    fn half_opacity_mixes() {
        let o = composite(GREY, RED, 0.5, BlendMode::Normal);
        assert!((o[0] - 0.75).abs() < 1e-6 && (o[1] - 0.25).abs() < 1e-6 && o[3] == 1.0);
    }
    #[test]
    fn multiply_screen() {
        let m = composite(GREY, RED, 1.0, BlendMode::Multiply);
        assert!((m[0] - 0.5).abs() < 1e-6 && m[1].abs() < 1e-6);
        let s = composite(GREY, RED, 1.0, BlendMode::Screen);
        assert!((s[0] - 1.0).abs() < 1e-6 && (s[1] - 0.5).abs() < 1e-6);
    }
    #[test]
    fn blend_over_transparent_is_source() {
        // Over a fully transparent backdrop every blend mode yields the source colour.
        for m in BlendMode::ALL {
            let o = composite([0.0; 4], [0.2, 0.4, 0.6, 0.8], 1.0, m);
            assert!(
                (o[0] - 0.2).abs() < 1e-5 && (o[3] - 0.8).abs() < 1e-5,
                "{m:?}"
            );
        }
    }
    #[test]
    fn ids_roundtrip() {
        for m in BlendMode::ALL {
            assert_eq!(BlendMode::from_id(m.id()), Some(m));
        }
        assert_eq!(BlendMode::Overlay.shader_index(), 3);
    }
}
