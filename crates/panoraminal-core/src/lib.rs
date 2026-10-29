//! # panoraminal-core
//!
//! The GPU-independent heart of the Panoraminal engine:
//!
//! * [`math`] — equirectangular ↔ sphere mapping, great-circle interpolation, the 360° camera.
//! * [`blend`] — layer blend modes (W3C compositing formulas, straight alpha).
//! * [`document`] — the layered document and CPU reference compositor.
//! * [`brush`] — the spherical brush engine (spacing, pressure, hardness, smoothing,
//!   seam wrapping and polar distortion correction).
//! * [`history`] — tile-based undo/redo.
//! * [`rect`] — integer rectangles and dirty-region tracking.

pub mod blend;
pub mod brush;
pub mod document;
pub mod fill;
pub mod history;
pub mod math;
pub mod rect;

pub use blend::BlendMode;
pub use brush::{BrushSettings, StrokeEngine, ToolKind};
pub use document::{Document, Layer, LayerId};
pub use history::{Edit, History};
pub use math::{Camera, FlatView, Vec3};
pub use rect::{DirtyRegion, IRect};

/// Largest supported document side, in pixels.
pub const MAX_DIM: u32 = 8192;
/// Maximum number of layers in a document.
pub const MAX_LAYERS: usize = 64;
