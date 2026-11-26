//! The spherical brush engine.
//!
//! Strokes are sampled along great circles of the panorama sphere, so a stroke that
//! crosses the 0°/360° seam or passes over a pole stays continuous. Each dab is an
//! ellipse in equirectangular space whose horizontal radius is divided by `cos(lat)`
//! (capped), which makes it look round once wrapped onto the sphere ("polar
//! correction").
//!
//! Dabs accumulate into a floating-point *stroke mask*; while painting, the renderer
//! previews `layer ⊕ mask` on the GPU, and on release the mask is committed into the
//! layer pixels with tile-based undo data.

use crate::document::{Document, LayerId};
use crate::history::{Edit, Tile};
use crate::math::{angle_between, dir_to_px, slerp, Vec3};
use crate::paint::{hash01, mix_color, value_noise1, PaperTexture, TextureField, TipShape};
use crate::rect::{DirtyRegion, IRect};
use crate::tip::{PreparedTip, Rng};
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;
use std::sync::Arc;

/// Maximum horizontal stretch `1/cos(lat)` applied near the poles.
pub const MAX_STRETCH: f64 = 12.0;
/// Side of an undo tile, in pixels.
pub const TILE: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// Hard, pressure-sensitive inking pen.
    Pen,
    /// Soft round brush.
    Brush,
    /// Very soft, low-flow brush that builds up.
    Airbrush,
    Eraser,
    /// Sampled tip from the brush library (Photoshop ABR, GIMP GBR/GIH, images).
    Custom,
}

impl ToolKind {
    pub const ALL: [ToolKind; 5] = [
        ToolKind::Pen,
        ToolKind::Brush,
        ToolKind::Airbrush,
        ToolKind::Eraser,
        ToolKind::Custom,
    ];
    pub fn id(self) -> &'static str {
        match self {
            ToolKind::Pen => "pen",
            ToolKind::Brush => "brush",
            ToolKind::Airbrush => "airbrush",
            ToolKind::Eraser => "eraser",
            ToolKind::Custom => "custom",
        }
    }
    pub fn is_eraser(self) -> bool {
        self == ToolKind::Eraser
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrushSettings {
    /// Diameter in document pixels (at the equator).
    pub size: f32,
    /// Maximum opacity of a stroke, 0–1.
    pub opacity: f32,
    /// Per-dab alpha, 0–1 (lower values build up). Called *density* in the UI.
    pub flow: f32,
    /// Edge hardness, 0 (soft) – 1 (hard).
    pub hardness: f32,
    /// Dab spacing as a fraction of the diameter.
    pub spacing: f32,
    /// Input smoothing / stabilizer, 0–1.
    pub smoothing: f32,
    pub pressure_size: bool,
    pub pressure_opacity: bool,
    /// Smallest size at zero pressure, as a fraction of `size`.
    pub min_size: f32,
    /// Keep dabs round on the sphere (stretch horizontally by 1/cos(lat)).
    pub polar_correction: bool,

    // ---- 1.3: tip shape, texture, paint mixing, pressure feel, spray ----------------
    // Every default below is *neutral*: settings saved by 1.0–1.2 paint exactly as before.
    /// Shape of the tip (built-in tools; custom brushes use their own tips).
    pub tip_shape: TipShape,
    /// Tip angle in degrees (counter-clockwise).
    pub tip_angle: f32,
    /// Turn the tip with the stroke direction.
    pub follow_stroke: bool,
    /// Paper / canvas texture applied to the coverage.
    pub texture: PaperTexture,
    /// Texture strength, 0–1.
    pub texture_strength: f32,
    /// Texture scale, 0.25–8 (1 = default grain size).
    pub texture_scale: f32,
    /// How much of the colour already on the layer is picked up by the brush, 0–1.
    pub blending: f32,
    /// Thins the paint where the layer is transparent (watery strokes), 0–1.
    pub dilution: f32,
    /// How long the picked-up colour stays in the brush (colour stretch), 0–1.
    pub persistence: f32,
    /// Mix colours like paint (yellow + blue = green) instead of RGB.
    pub pigment: bool,
    /// Broken, uneven mixing and dry-brush gaps along the bristles, 0–1.
    pub irregularity: f32,
    /// Light pressure blends more (smudges), full pressure lays the brush colour.
    pub pressure_blend: bool,
    /// Pressure curve, −1 (soft: needs more pressure) … +1 (firm: reaches full early).
    pub pressure_feel: f32,
    /// Smallest density (flow) at zero pressure when pressure controls opacity.
    pub min_density: f32,
    /// Spray amount, 0 = smooth airbrush, 1 = particles only.
    pub particles: f32,
    /// How many particles each dab throws, 0–1.
    pub particle_density: f32,
    /// Particle diameter as a fraction of the brush size.
    pub particle_size: f32,
    /// Random particle size variation, 0–1.
    pub particle_size_jitter: f32,
    /// Spray radius as a multiple of the brush radius (0.2–3).
    pub spread: f32,
    /// 0 = particles spread evenly, 1 = concentrated in the centre.
    pub spread_falloff: f32,
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self {
            size: 40.0,
            opacity: 1.0,
            flow: 1.0,
            hardness: 0.7,
            spacing: 0.12,
            smoothing: 0.2,
            pressure_size: true,
            pressure_opacity: false,
            min_size: 0.08,
            polar_correction: true,
            tip_shape: TipShape::Round,
            tip_angle: 0.0,
            follow_stroke: false,
            texture: PaperTexture::None,
            texture_strength: 0.5,
            texture_scale: 1.0,
            blending: 0.0,
            dilution: 0.0,
            persistence: 0.0,
            pigment: false,
            irregularity: 0.0,
            pressure_blend: false,
            pressure_feel: 0.0,
            min_density: 0.05,
            particles: 0.0,
            particle_density: 0.5,
            particle_size: 0.03,
            particle_size_jitter: 0.6,
            spread: 1.0,
            spread_falloff: 0.5,
        }
    }
}

impl BrushSettings {
    /// The tool presets of Panoraminal 1.0–1.2 (the "Classic" styles).
    pub fn classic(tool: ToolKind) -> Self {
        let d = Self::default();
        match tool {
            ToolKind::Pen => Self {
                size: 12.0,
                hardness: 0.95,
                spacing: 0.08,
                smoothing: 0.35,
                ..d
            },
            ToolKind::Brush => Self {
                size: 40.0,
                hardness: 0.6,
                ..d
            },
            ToolKind::Airbrush => Self {
                size: 140.0,
                hardness: 0.0,
                flow: 0.08,
                spacing: 0.06,
                pressure_size: false,
                pressure_opacity: true,
                ..d
            },
            ToolKind::Eraser => Self {
                size: 60.0,
                hardness: 0.8,
                ..d
            },
            ToolKind::Custom => Self {
                size: 80.0,
                hardness: 1.0,
                spacing: 0.25,
                smoothing: 0.15,
                ..d
            },
        }
    }
    /// Default settings of a tool (the first built-in style). Since 1.3 the brush
    /// defaults to the oil style; every other tool keeps its classic preset.
    pub fn preset(tool: ToolKind) -> Self {
        builtin_styles(tool)
            .first()
            .map(|s| s.settings)
            .unwrap_or_else(|| Self::classic(tool))
    }
    /// The paint-mixing part of `style` applied on top of `self` (keeps the size,
    /// opacity, smoothing and pressure toggles). Used to upgrade 1.2 brush settings.
    pub fn with_style_of(self, style: &BrushSettings) -> Self {
        Self {
            size: self.size,
            opacity: self.opacity,
            smoothing: self.smoothing,
            pressure_size: self.pressure_size,
            pressure_opacity: self.pressure_opacity,
            min_size: self.min_size,
            polar_correction: self.polar_correction,
            ..*style
        }
    }
    pub fn clamp(&mut self) {
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let d = Self::default();
        self.size = fin(self.size, d.size).clamp(1.0, 1000.0);
        self.opacity = fin(self.opacity, 1.0).clamp(0.01, 1.0);
        self.flow = fin(self.flow, 1.0).clamp(0.01, 1.0);
        self.hardness = fin(self.hardness, d.hardness).clamp(0.0, 1.0);
        self.spacing = fin(self.spacing, d.spacing).clamp(0.02, 2.0);
        self.smoothing = fin(self.smoothing, 0.0).clamp(0.0, 1.0);
        self.min_size = fin(self.min_size, d.min_size).clamp(0.0, 1.0);
        self.tip_angle = fin(self.tip_angle, 0.0).clamp(-360.0, 360.0);
        self.texture_strength = fin(self.texture_strength, 0.5).clamp(0.0, 1.0);
        self.texture_scale = fin(self.texture_scale, 1.0).clamp(0.25, 8.0);
        self.blending = fin(self.blending, 0.0).clamp(0.0, 1.0);
        self.dilution = fin(self.dilution, 0.0).clamp(0.0, 1.0);
        self.persistence = fin(self.persistence, 0.0).clamp(0.0, 1.0);
        self.irregularity = fin(self.irregularity, 0.0).clamp(0.0, 1.0);
        self.pressure_feel = fin(self.pressure_feel, 0.0).clamp(-1.0, 1.0);
        self.min_density = fin(self.min_density, d.min_density).clamp(0.0, 1.0);
        self.particles = fin(self.particles, 0.0).clamp(0.0, 1.0);
        self.particle_density = fin(self.particle_density, 0.5).clamp(0.0, 1.0);
        self.particle_size = fin(self.particle_size, 0.03).clamp(0.005, 0.5);
        self.particle_size_jitter = fin(self.particle_size_jitter, 0.6).clamp(0.0, 1.0);
        self.spread = fin(self.spread, 1.0).clamp(0.2, 3.0);
        self.spread_falloff = fin(self.spread_falloff, 0.5).clamp(0.0, 1.0);
    }
    /// Applies the pressure curve (`pressure_feel`) to a raw pressure.
    #[inline]
    pub fn curve(&self, p: f32) -> f32 {
        let p = p.clamp(0.0, 1.0);
        if self.pressure_feel == 0.0 {
            p
        } else {
            p.powf(4f32.powf(-self.pressure_feel))
        }
    }
    fn press_size(&self, p: f32) -> f32 {
        if self.pressure_size {
            self.min_size + (1.0 - self.min_size) * self.curve(p)
        } else {
            1.0
        }
    }
    fn press_density(&self, p: f32) -> f32 {
        if self.pressure_opacity {
            self.min_density + (1.0 - self.min_density) * self.curve(p)
        } else {
            1.0
        }
    }
    /// Whether strokes with these settings mix with the colours on the layer
    /// (painted directly into the layer instead of through the stroke mask).
    pub fn is_wet(&self, tool: ToolKind) -> bool {
        !tool.is_eraser() && (self.blending > 0.0 || self.dilution > 0.0 || self.persistence > 0.0)
    }
}

/// A named, built-in style of a tool (SAI-like presets, our own values).
#[derive(Clone, Copy, Debug)]
pub struct BuiltinStyle {
    /// Stable id; the UI translates `style.<id>`.
    pub id: &'static str,
    pub settings: BrushSettings,
}

/// Built-in styles of a tool, the default first.
pub fn builtin_styles(tool: ToolKind) -> Vec<BuiltinStyle> {
    let c = BrushSettings::classic(tool);
    let st = |id, settings| BuiltinStyle { id, settings };
    match tool {
        ToolKind::Brush => vec![
            st(
                "oil",
                BrushSettings {
                    size: 40.0,
                    hardness: 0.6,
                    flow: 0.9,
                    spacing: 0.1,
                    tip_shape: TipShape::Bristle,
                    follow_stroke: true,
                    blending: 0.5,
                    persistence: 0.65,
                    pigment: true,
                    irregularity: 0.5,
                    pressure_blend: true,
                    texture: PaperTexture::Canvas,
                    texture_strength: 0.2,
                    ..c
                },
            ),
            st(
                "dry_brush",
                BrushSettings {
                    size: 46.0,
                    flow: 0.8,
                    spacing: 0.1,
                    tip_shape: TipShape::Bristle,
                    follow_stroke: true,
                    blending: 0.35,
                    persistence: 0.4,
                    pigment: true,
                    irregularity: 0.9,
                    texture: PaperTexture::Canvas,
                    texture_strength: 0.55,
                    ..c
                },
            ),
            st(
                "blender",
                BrushSettings {
                    size: 50.0,
                    hardness: 0.3,
                    flow: 0.6,
                    blending: 0.9,
                    persistence: 0.9,
                    pigment: true,
                    irregularity: 0.15,
                    ..c
                },
            ),
            st(
                "watercolor",
                BrushSettings {
                    size: 60.0,
                    hardness: 0.2,
                    flow: 0.35,
                    blending: 0.6,
                    dilution: 0.6,
                    persistence: 0.5,
                    pigment: true,
                    irregularity: 0.2,
                    texture: PaperTexture::Paper,
                    texture_strength: 0.35,
                    ..c
                },
            ),
            st("classic", c),
        ],
        ToolKind::Airbrush => vec![
            st("soft", c),
            st(
                "spray",
                BrushSettings {
                    particles: 0.85,
                    particle_density: 0.55,
                    particle_size: 0.025,
                    particle_size_jitter: 0.6,
                    spread: 1.0,
                    spread_falloff: 0.55,
                    ..c
                },
            ),
            st(
                "splatter",
                BrushSettings {
                    size: 120.0,
                    spacing: 0.15,
                    particles: 1.0,
                    particle_density: 0.2,
                    particle_size: 0.07,
                    particle_size_jitter: 0.8,
                    spread: 1.4,
                    spread_falloff: 0.2,
                    ..c
                },
            ),
            st(
                "grainy",
                BrushSettings {
                    flow: 0.12,
                    texture: PaperTexture::Grain,
                    texture_strength: 0.75,
                    ..c
                },
            ),
        ],
        ToolKind::Pen => vec![
            st("ink", c),
            st(
                "pencil",
                BrushSettings {
                    size: 6.0,
                    hardness: 0.6,
                    flow: 0.7,
                    spacing: 0.1,
                    tip_shape: TipShape::Chalk,
                    texture: PaperTexture::Paper,
                    texture_strength: 0.6,
                    pressure_opacity: true,
                    min_density: 0.15,
                    ..c
                },
            ),
            st(
                "marker",
                BrushSettings {
                    size: 24.0,
                    flow: 0.5,
                    opacity: 0.85,
                    tip_shape: TipShape::Flat,
                    follow_stroke: true,
                    tip_angle: 30.0,
                    blending: 0.15,
                    persistence: 0.2,
                    pressure_size: false,
                    ..c
                },
            ),
        ],
        ToolKind::Eraser => vec![
            st("hard_eraser", c),
            st(
                "soft_eraser",
                BrushSettings {
                    size: 120.0,
                    hardness: 0.0,
                    flow: 0.3,
                    spacing: 0.08,
                    ..c
                },
            ),
            st(
                "textured_eraser",
                BrushSettings {
                    size: 60.0,
                    tip_shape: TipShape::Chalk,
                    texture: PaperTexture::Grain,
                    texture_strength: 0.5,
                    ..c
                },
            ),
        ],
        ToolKind::Custom => vec![st("classic", c)],
    }
}

/// One dab of a wet (mixing) stroke, waiting to be painted into the layer.
#[derive(Debug, Default)]
struct WetDab {
    /// (pixel index, coverage, across-stroke offset −1…1)
    px: Vec<(u32, f32, f32)>,
    pressure: f32,
}

/// Live stroke state.
///
/// Two paths share the same dab rasterizers:
///
/// * **mask strokes** (pen, classic brush, airbrush, eraser, custom brushes): dabs
///   accumulate into a floating-point coverage mask that the renderer previews on the
///   GPU and that is committed into the layer on release;
/// * **wet strokes** ([`BrushSettings::is_wet`], e.g. the oil brush): every dab picks up
///   the colours under it and is painted straight into the layer, so colours mix as
///   you paint. The tiles are captured for undo the first time a dab touches them.
///   Dabs are queued by [`StrokeEngine::add_point`] and painted by
///   [`StrokeEngine::flush`] (called every frame, and by [`StrokeEngine::end`]).
pub struct StrokeEngine {
    active: bool,
    layer: LayerId,
    tool: ToolKind,
    settings: BrushSettings,
    rgb: [u8; 3],
    mask: Vec<f32>,
    w: u32,
    h: u32,
    tiles_x: u32,
    touched: Vec<bool>,
    any_dab: bool,
    raw_last: Vec3,
    smooth_dir: Vec3,
    smooth_p: f32,
    last: Vec3,
    last_p: f32,
    gap: f64,
    tip: Option<Arc<PreparedTip>>,
    /// Procedural tip of the current tip shape (built-in tools), with its cache key.
    shape_tip: Option<(ShapeKey, Arc<PreparedTip>)>,
    texture: TextureField,
    texture_key: Option<(PaperTexture, u32, u32, u32, u32)>,
    rng: Rng,
    seq: usize,
    last_dab: Option<(f64, f64)>,
    /// Direction of the stroke at the current dab (east, north), unit length.
    tdir: (f64, f64),
    /// First dab of a direction-following tip, painted once the direction is known.
    deferred: Option<(Vec3, f32)>,
    // wet strokes
    wet: bool,
    dab: Vec<(u32, f32, f32)>,
    pending: Vec<WetDab>,
    wet_tiles: Vec<Tile>,
    /// Paint carried by the brush: straight RGB and amount (0–1).
    reservoir: ([f32; 3], f32),
    mix_seed: u64,
    /// Seed of the jitter generator for the next stroke (strokes are reproducible).
    pub seed: u64,
    /// Regions of the mask changed since the renderer last uploaded it.
    pub mask_dirty: DirtyRegion,
    /// End point of the previous committed stroke (for Shift+click straight lines).
    pub last_end: Option<Vec3>,
}

type ShapeKey = (TipShape, u32, bool, u32);

impl Default for StrokeEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl StrokeEngine {
    pub fn new() -> Self {
        Self {
            active: false,
            layer: LayerId(0),
            tool: ToolKind::Brush,
            settings: BrushSettings::default(),
            rgb: [0, 0, 0],
            mask: Vec::new(),
            w: 0,
            h: 0,
            tiles_x: 0,
            touched: Vec::new(),
            any_dab: false,
            raw_last: Vec3::default(),
            smooth_dir: Vec3::default(),
            smooth_p: 1.0,
            last: Vec3::default(),
            last_p: 1.0,
            gap: 0.0,
            tip: None,
            shape_tip: None,
            texture: TextureField::default(),
            texture_key: None,
            rng: Rng::new(1),
            seq: 0,
            last_dab: None,
            tdir: (1.0, 0.0),
            deferred: None,
            wet: false,
            dab: Vec::new(),
            pending: Vec::new(),
            wet_tiles: Vec::new(),
            reservoir: ([0.0; 3], 1.0),
            mix_seed: 1,
            seed: 1,
            mask_dirty: DirtyRegion::default(),
            last_end: None,
        }
    }
    pub fn is_active(&self) -> bool {
        self.active
    }
    pub fn layer(&self) -> LayerId {
        self.layer
    }
    pub fn tool(&self) -> ToolKind {
        self.tool
    }
    pub fn opacity(&self) -> f32 {
        self.settings.opacity
    }
    pub fn color(&self) -> [u8; 3] {
        self.rgb
    }
    /// Whether the current stroke paints directly into the layer (mixing colours).
    pub fn is_wet(&self) -> bool {
        self.wet
    }
    /// The stroke coverage mask (`w*h` floats), valid while a stroke is active.
    pub fn mask(&self) -> &[f32] {
        &self.mask
    }
    pub fn mask_size(&self) -> (u32, u32) {
        (self.w, self.h)
    }
    /// Forgets document-specific state (call when the document is replaced).
    pub fn reset(&mut self) {
        let tip = self.tip.take();
        let shape = self.shape_tip.take();
        *self = Self::new();
        self.tip = tip;
        self.shape_tip = shape;
    }
    /// Sets the sampled tip used by [`ToolKind::Custom`] strokes (`None` = round dab).
    pub fn set_tip(&mut self, tip: Option<Arc<PreparedTip>>) {
        self.tip = tip;
    }
    pub fn tip(&self) -> Option<&Arc<PreparedTip>> {
        self.tip.as_ref()
    }

    fn ensure_buffers(&mut self, w: u32, h: u32) {
        if self.w != w || self.h != h || self.mask.len() != (w * h) as usize {
            self.w = w;
            self.h = h;
            self.mask = vec![0.0; (w * h) as usize];
            self.tiles_x = w.div_ceil(TILE);
            self.touched = vec![false; (self.tiles_x * h.div_ceil(TILE)) as usize];
            self.mask_dirty.mark_full();
            self.texture_key = None;
        }
    }

    fn prepare_style(&mut self) {
        let s = self.settings;
        // procedural tip of the tip shape
        if self.tool != ToolKind::Custom && s.tip_shape != TipShape::Round {
            let key: ShapeKey = (
                s.tip_shape,
                s.tip_angle.to_bits(),
                s.follow_stroke,
                ((s.hardness * 20.0).round() as u32),
            );
            if self.shape_tip.as_ref().map(|(k, _)| *k) != Some(key) {
                let img = s.tip_shape.image(160, s.hardness);
                let dy = s.tip_shape.dynamics(s.tip_angle, s.follow_stroke);
                self.shape_tip = Some((key, PreparedTip::new(&[img], &dy)));
            }
        }
        // texture field
        let tk = (
            s.texture,
            s.texture_strength.to_bits(),
            s.texture_scale.to_bits(),
            self.w,
            self.h,
        );
        if self.texture_key != Some(tk) {
            self.texture = TextureField::new(
                s.texture,
                s.texture_strength,
                s.texture_scale,
                self.w,
                self.h,
            );
            self.texture_key = Some(tk);
        }
    }

    /// Starts a stroke on the document's active layer. Returns `false` if the layer
    /// is missing or hidden.
    pub fn begin(
        &mut self,
        doc: &Document,
        tool: ToolKind,
        settings: BrushSettings,
        rgb: [u8; 3],
        dir: Vec3,
        pressure: f32,
    ) -> bool {
        let Some(layer) = doc.active_layer() else {
            return false;
        };
        if !layer.visible {
            return false;
        }
        self.ensure_buffers(doc.width, doc.height);
        let mut settings = settings;
        settings.clamp();
        self.active = true;
        self.layer = layer.id;
        self.tool = tool;
        self.settings = settings;
        self.rgb = rgb;
        self.any_dab = false;
        self.wet = settings.is_wet(tool);
        self.pending.clear();
        self.wet_tiles.clear();
        self.dab.clear();
        self.reservoir = (rgb.map(|c| c as f32 / 255.0), 1.0);
        self.prepare_style();
        let p = pressure.clamp(0.0, 1.0);
        self.raw_last = dir;
        self.smooth_dir = dir;
        self.smooth_p = p;
        self.last = dir;
        self.last_p = p;
        self.rng = Rng::new(self.seed);
        self.mix_seed = self.seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ 0xA5A5;
        self.seed = self.seed.wrapping_add(0x5851_F42D);
        self.seq = 0;
        self.last_dab = None;
        self.tdir = (1.0, 0.0);
        self.deferred = None;
        self.stamp(dir, p);
        self.gap = self.gap_for(p);
        true
    }

    /// Adds an input sample. `direct` bypasses smoothing (used for the final sample).
    pub fn add_point(&mut self, dir: Vec3, pressure: f32, direct: bool) {
        if !self.active {
            return;
        }
        let p = pressure.clamp(0.0, 1.0);
        self.raw_last = dir;
        let s = if direct {
            0.0
        } else {
            self.settings.smoothing as f64 * 0.92
        };
        if s > 0.0 {
            self.smooth_dir = self.smooth_dir.lerp(dir, 1.0 - s).normalize();
            self.smooth_p += (p - self.smooth_p) * (1.0 - s as f32);
        } else {
            self.smooth_dir = dir;
            self.smooth_p = p;
        }
        let (a, pa, b, pb) = (self.last, self.last_p, self.smooth_dir, self.smooth_p);
        self.segment(a, pa, b, pb);
        self.last = b;
        self.last_p = pb;
    }

    /// Straight great-circle line from `a` to `b` (Shift+click).
    pub fn line(
        &mut self,
        doc: &Document,
        tool: ToolKind,
        settings: BrushSettings,
        rgb: [u8; 3],
        a: Vec3,
        b: Vec3,
        pressure: f32,
    ) -> bool {
        if !self.begin(doc, tool, settings, rgb, a, pressure) {
            return false;
        }
        self.settings.smoothing = 0.0;
        self.add_point(b, pressure, true);
        true
    }

    fn gap_for(&self, p: f32) -> f64 {
        let dia = self.settings.size as f64 * self.settings.press_size(p) as f64;
        let px_ang = TAU / self.w as f64;
        (dia * px_ang * self.settings.spacing as f64).max(px_ang * 0.5)
    }

    fn segment(&mut self, a: Vec3, pa: f32, b: Vec3, pb: f32) {
        let ang = angle_between(a, b);
        if ang < 1e-12 {
            return;
        }
        let mut pos = self.gap;
        let mut guard = 0u32;
        while pos <= ang && guard < 200_000 {
            let t = pos / ang;
            let d = slerp(a, b, t);
            let p = pa + (pb - pa) * t as f32;
            self.stamp(d, p);
            pos += self.gap_for(p);
            guard += 1;
        }
        self.gap = pos - ang;
    }

    /// Adds coverage `a` at mask pixel `i` (texture applied). `v` is the signed
    /// across-stroke offset used for bristle streaks in wet strokes.
    #[inline]
    fn put(&mut self, i: usize, a: f32, v: f32) {
        let mut a = a;
        if self.texture.is_active() {
            let (x, y) = (i as u32 % self.w, i as u32 / self.w);
            a *= self.texture.factor(x, y);
        }
        if a <= 0.0 {
            return;
        }
        if self.wet {
            self.dab.push((i as u32, a.min(1.0), v));
        } else {
            let m = &mut self.mask[i];
            *m += a * (1.0 - *m);
        }
    }

    /// The tip used by the current stroke (custom or tip shape), if not round.
    fn current_tip(&self) -> Option<Arc<PreparedTip>> {
        if self.tool == ToolKind::Custom {
            self.tip.clone()
        } else if self.settings.tip_shape != TipShape::Round {
            self.shape_tip.as_ref().map(|(_, t)| t.clone())
        } else {
            None
        }
    }

    fn stretch_at(&self, pp: &crate::math::PixelPos) -> f64 {
        if self.settings.polar_correction {
            1.0 / pp.lat.cos().max(1.0 / MAX_STRETCH)
        } else {
            1.0
        }
    }

    /// One dab of the stroke. Tracks the stroke direction; the first dab of a tip that
    /// turns with the stroke waits until the direction is known.
    fn stamp(&mut self, dir: Vec3, pressure: f32) {
        let (w, h) = (self.w, self.h);
        let pp = dir_to_px(dir, w, h);
        let stretch = self.stretch_at(&pp);
        let tip = self.current_tip();
        let follows = tip
            .as_ref()
            .map(|t| t.dynamics.follow_direction)
            .unwrap_or(false);
        // Stroke direction in the tangent frame (east, north).
        if let Some((lx, ly)) = self.last_dab {
            let mut ddx = pp.x - lx;
            let wf = w as f64;
            if ddx > wf / 2.0 {
                ddx -= wf;
            } else if ddx < -wf / 2.0 {
                ddx += wf;
            }
            let ky = 2.0 * h as f64 / w as f64;
            let (ex, ny) = (ddx / stretch, -(pp.y - ly) / ky);
            let l = (ex * ex + ny * ny).sqrt();
            if l > 1e-9 {
                self.tdir = (ex / l, ny / l);
            }
        } else if follows {
            self.last_dab = Some((pp.x, pp.y));
            self.deferred = Some((dir, pressure));
            return;
        }
        self.last_dab = Some((pp.x, pp.y));
        if let Some((d0, p0)) = self.deferred.take() {
            let pp0 = dir_to_px(d0, w, h);
            let st0 = self.stretch_at(&pp0);
            self.stamp_at(&pp0, st0, p0, tip.clone());
        }
        self.stamp_at(&pp, stretch, pressure, tip);
    }

    /// Paints a deferred first dab (single clicks with a direction-following tip).
    fn stamp_deferred(&mut self) {
        if let Some((d0, p0)) = self.deferred.take() {
            let pp0 = dir_to_px(d0, self.w, self.h);
            let st0 = self.stretch_at(&pp0);
            let tip = self.current_tip();
            self.stamp_at(&pp0, st0, p0, tip);
        }
    }

    /// The tip (round, shape or custom), then spray particles.
    fn stamp_at(
        &mut self,
        pp: &crate::math::PixelPos,
        stretch: f64,
        pressure: f32,
        tip: Option<Arc<PreparedTip>>,
    ) {
        let spray = self.settings.particles;
        if spray < 1.0 {
            match tip {
                Some(t) => self.stamp_tip(&t, pp, stretch, pressure, 1.0 - spray),
                None => self.stamp_round(pp, stretch, pressure, 1.0 - spray),
            }
        }
        if spray > 0.0 {
            self.spray(pp, stretch, pressure);
        }
        if self.wet && !self.dab.is_empty() {
            let px = std::mem::take(&mut self.dab);
            self.pending.push(WetDab { px, pressure });
            self.any_dab = true;
        }
    }

    /// Rasterizes one elliptical dab (wrapping horizontally at the seam).
    fn stamp_round(&mut self, pp: &crate::math::PixelPos, stretch: f64, pressure: f32, k: f32) {
        let (w, h) = (self.w, self.h);
        let s = self.settings;
        let dia = (s.size * s.press_size(pressure)).max(0.5) as f64;
        let r = dia / 2.0;
        let ry = r * (2.0 * h as f64 / w as f64);
        let rx = (r * stretch).min(w as f64 / 2.0 - 1.0).max(0.5);
        let ry = ry.max(0.5);
        let alpha = s.flow * s.press_density(pressure) * k;
        if alpha <= 0.0 {
            return;
        }
        let hard = s.hardness;
        let tdir = self.tdir;
        // AA width in normalized units (one pixel)
        let aa = 1.0 / rx.min(ry).max(0.5);
        let y0 = (pp.y - ry - 1.0).floor().max(0.0) as i64;
        let y1 = (pp.y + ry + 1.0).ceil().min(h as f64) as i64;
        let x0 = (pp.x - rx - 1.0).floor() as i64;
        let x1 = (pp.x + rx + 1.0).ceil() as i64;
        if y1 <= y0 {
            return;
        }
        // Tiny brushes: scale coverage by area so 1px lines aren't too heavy.
        let small = ((rx * ry) / 0.25).min(1.0) as f32;
        for y in y0..y1 {
            let dy = (y as f64 + 0.5 - pp.y) / ry;
            if dy.abs() > 1.0 + aa {
                continue;
            }
            let row = (y as u32 * w) as usize;
            for x in x0..x1 {
                let dx = (x as f64 + 0.5 - pp.x) / rx;
                let d = (dx * dx + dy * dy).sqrt();
                let cov = if hard >= 0.98 {
                    ((1.0 - d) / aa + 0.5).clamp(0.0, 1.0) as f32
                } else {
                    let hh = hard as f64;
                    if d >= 1.0 {
                        0.0
                    } else if d <= hh {
                        1.0
                    } else {
                        let t = (d - hh) / (1.0 - hh);
                        (1.0 - t * t * (3.0 - 2.0 * t)) as f32
                    }
                };
                if cov <= 0.0 {
                    continue;
                }
                let a = cov * alpha * small;
                let xi = x.rem_euclid(w as i64) as usize;
                let v = (-dx * tdir.1 - dy * tdir.0) as f32;
                self.put(row + xi, a, v);
            }
        }
        self.mark_box(x0, x1, y0, y1);
        self.any_dab = true;
    }

    /// Spray particles: small soft dots thrown inside `spread` × the brush radius.
    /// Positions come from the stroke's seeded generator (reproducible strokes).
    fn spray(&mut self, pp: &crate::math::PixelPos, stretch: f64, pressure: f32) {
        let (w, h) = (self.w, self.h);
        let s = self.settings;
        let dia = (s.size * s.press_size(pressure)).max(0.5) as f64;
        let r = dia / 2.0 * s.spread as f64;
        let ky = 2.0 * h as f64 / w as f64;
        let n = (s.particle_density
            * s.particle_density
            * 140.0
            * s.particles
            * (s.spacing / 0.06).clamp(0.3, 10.0))
        .round()
        .min(600.0) as u32;
        let n = n.max(u32::from(s.particle_density > 0.0));
        let alpha_base = s.press_density(pressure);
        // exponent > 0.5 concentrates the particles in the centre
        let e = 0.5 + 1.6 * s.spread_falloff as f64;
        for _ in 0..n {
            let rng = &mut self.rng;
            let ang = rng.next_f32() as f64 * TAU;
            let rad = (rng.next_f32() as f64).powf(e) * r;
            let size_f = 1.0 - s.particle_size_jitter * rng.next_f32();
            let pa = (0.55 + 0.45 * rng.next_f32()) * alpha_base;
            let (ex, ny) = (ang.cos() * rad, ang.sin() * rad);
            let cx = pp.x + ex * stretch;
            let cy = (pp.y - ny * ky).clamp(0.0, h as f64 - 0.001);
            let pr = (dia * s.particle_size as f64 * size_f as f64 * 0.5).max(0.5);
            let rx = (pr * stretch).min(w as f64 / 4.0).max(0.5);
            let ry = (pr * ky).max(0.5);
            let y0 = (cy - ry - 1.0).floor().max(0.0) as i64;
            let y1 = (cy + ry + 1.0).ceil().min(h as f64) as i64;
            let x0 = (cx - rx - 1.0).floor() as i64;
            let x1 = (cx + rx + 1.0).ceil() as i64;
            // tiny dots: keep their total paint proportional to their area
            let small = ((rx * ry) / 0.6).min(1.0) as f32;
            let vv = (ex * -self.tdir.1 + ny * self.tdir.0) / r.max(1e-6);
            for y in y0..y1 {
                let dy = (y as f64 + 0.5 - cy) / ry;
                let row = (y as u32 * w) as usize;
                for x in x0..x1 {
                    let dx = (x as f64 + 0.5 - cx) / rx;
                    let d = (dx * dx + dy * dy).sqrt();
                    if d >= 1.0 {
                        continue;
                    }
                    let t = d.max(0.5) * 2.0 - 1.0;
                    let cov = (1.0 - t * t * (3.0 - 2.0 * t)) as f32;
                    let xi = x.rem_euclid(w as i64) as usize;
                    self.put(row + xi, cov * pa * small, vv as f32);
                }
            }
            self.mark_box(x0, x1, y0, y1);
            self.any_dab = true;
        }
    }

    /// Marks mask rows `y0..y1` × columns `x0..x1` (may extend past the seam) dirty.
    fn mark_box(&mut self, x0: i64, x1: i64, y0: i64, y1: i64) {
        if self.wet {
            // wet dabs mark the layer when they are painted (see `flush`)
            return;
        }
        let w = self.w;
        let yr = (y0.max(0) as u32, (y1.min(self.h as i64)) as u32);
        if yr.1 <= yr.0 {
            return;
        }
        let mut mark = |xa: i64, xb: i64| {
            let xa = xa.clamp(0, w as i64) as u32;
            let xb = xb.clamp(0, w as i64) as u32;
            if xb <= xa {
                return;
            }
            self.mask_dirty
                .add(IRect::new(xa, yr.0, xb - xa, yr.1 - yr.0));
            for ty in yr.0 / TILE..=(yr.1 - 1) / TILE {
                for tx in xa / TILE..=(xb - 1) / TILE {
                    self.touched[(ty * self.tiles_x + tx) as usize] = true;
                }
            }
        };
        if x1 - x0 >= w as i64 {
            mark(0, w as i64);
        } else {
            mark(x0, x1);
            if x0 < 0 {
                mark(x0 + w as i64, w as i64);
            }
            if x1 > w as i64 {
                mark(0, x1 - w as i64);
            }
        }
    }

    /// Stamps a sampled tip (with its shape dynamics) centred on `pp`.
    ///
    /// The tip is rasterized in the local tangent frame: one tip "radius" spans `rx`
    /// pixels horizontally (`r / cos(lat)` with polar correction) and `ry` vertically,
    /// so stamps keep their shape on the sphere and wrap across the seam.
    fn stamp_tip(
        &mut self,
        tip: &PreparedTip,
        pp: &crate::math::PixelPos,
        stretch: f64,
        pressure: f32,
        k: f32,
    ) {
        let (w, h) = (self.w, self.h);
        let s = self.settings;
        let dy = &tip.dynamics;
        let base_dia = (s.size * s.press_size(pressure)).max(0.5) as f64;
        let ky = 2.0 * h as f64 / w as f64;
        let tdir = self.tdir;
        let mut count = dy.count.max(1) as f32;
        if dy.count_jitter > 0.0 {
            count = (count * (1.0 - dy.count_jitter * self.rng.next_f32())).max(1.0);
        }
        let n_var = tip.variants.len();
        for _ in 0..count.round() as u32 {
            let rng = &mut self.rng;
            // size / angle / roundness jitter
            let mut size_f = 1.0f64;
            if dy.size_jitter > 0.0 {
                size_f = (1.0 - dy.size_jitter as f64 * rng.next_f32() as f64)
                    .max(dy.min_diameter as f64);
            }
            let dia = (base_dia * size_f).max(0.5);
            let r = dia / 2.0;
            let mut ang = (dy.angle as f64).to_radians();
            if dy.follow_direction {
                ang += tdir.1.atan2(tdir.0);
            }
            if dy.angle_jitter > 0.0 {
                ang += dy.angle_jitter as f64 * std::f64::consts::PI * rng.signed() as f64;
            }
            let mut round = dy.roundness as f64;
            if dy.roundness_jitter > 0.0 {
                round *= (1.0 - dy.roundness_jitter as f64 * rng.next_f32() as f64)
                    .max(dy.min_roundness as f64);
            }
            let round = round.clamp(0.01, 1.0);
            let flip_x = dy.flip_x ^ (dy.flip_x_jitter && rng.next_f32() < 0.5);
            let flip_y = dy.flip_y ^ (dy.flip_y_jitter && rng.next_f32() < 0.5);
            let mut alpha = s.flow * k;
            if dy.flow_jitter > 0.0 {
                alpha *= 1.0 - dy.flow_jitter * rng.next_f32();
            }
            if dy.opacity_jitter > 0.0 {
                alpha *= 1.0 - dy.opacity_jitter * rng.next_f32();
            }
            alpha *= s.press_density(pressure);
            // scattering (perpendicular to the stroke; optionally along it too)
            let (mut cx, mut cy) = (pp.x, pp.y);
            if dy.scatter > 0.0 {
                let perp = (-tdir.1, tdir.0);
                let a = dy.scatter as f64 * dia * 0.5 * rng.signed() as f64;
                let (mut ex, mut ny) = (perp.0 * a, perp.1 * a);
                if dy.scatter_both_axes {
                    let b = dy.scatter as f64 * dia * 0.5 * rng.signed() as f64;
                    ex += tdir.0 * b;
                    ny += tdir.1 * b;
                }
                cx += ex * stretch;
                cy = (cy - ny * ky).clamp(0.0, h as f64 - 0.001);
            }
            let variant = match dy.selection {
                crate::tip::TipSelection::Sequential => self.seq,
                crate::tip::TipSelection::Random => (rng.next_f32() * n_var as f32) as usize,
            } % n_var;
            self.seq = self.seq.wrapping_add(1);
            if alpha <= 0.0 {
                continue;
            }
            let img = tip.level(variant, dia);
            // Map the tip's longest side onto the dab diameter.
            let m = img.max_side() as f64;
            let ext = (img.width as f64).hypot(img.height as f64) / m;
            let rx = (r * stretch).max(0.5);
            let ry = (r * ky).max(0.5);
            let ex_x = (rx * ext).min(w as f64 / 2.0 - 1.0);
            let ex_y = ry * ext;
            let y0 = (cy - ex_y - 1.0).floor().max(0.0) as i64;
            let y1 = (cy + ex_y + 1.0).ceil().min(h as f64) as i64;
            let x0 = (cx - ex_x - 1.0).floor() as i64;
            let x1 = (cx + ex_x + 1.0).ceil() as i64;
            if y1 <= y0 {
                continue;
            }
            let (sa, ca) = ang.sin_cos();
            let half_m = m * 0.5;
            let (tcx, tcy) = (img.width as f64 * 0.5, img.height as f64 * 0.5);
            let small = ((rx * ry) / 0.25).min(1.0) as f32;
            for y in y0..y1 {
                let ny = -(y as f64 + 0.5 - cy) / ry;
                let row = (y as u32 * w) as usize;
                for x in x0..x1 {
                    let ex = (x as f64 + 0.5 - cx) / rx;
                    // rotate into the tip frame, undo roundness
                    let mut a = ex * ca + ny * sa;
                    let mut b = (-ex * sa + ny * ca) / round;
                    if flip_x {
                        a = -a;
                    }
                    if flip_y {
                        b = -b;
                    }
                    let tx = tcx + a * half_m;
                    let ty = tcy - b * half_m;
                    if tx < -1.0 || ty < -1.0 || tx > img.width as f64 + 1.0 {
                        continue;
                    }
                    if ty > img.height as f64 + 1.0 {
                        continue;
                    }
                    let cov = img.sample(tx as f32, ty as f32);
                    if cov <= 0.0 {
                        continue;
                    }
                    let av = (cov * alpha * small).min(1.0);
                    let xi = x.rem_euclid(w as i64) as usize;
                    let v = (-ex * tdir.1 + ny * tdir.0) as f32;
                    self.put(row + xi, av, v);
                }
            }
            self.mark_box(x0, x1, y0, y1);
            self.any_dab = true;
        }
    }

    /// Paints the queued dabs of a wet stroke into the layer (mixing with the colours
    /// already there) and marks the changed tiles dirty. No-op for mask strokes.
    pub fn flush(&mut self, doc: &mut Document) {
        if self.pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        if !self.active || !self.wet || doc.width != self.w || doc.height != self.h {
            return;
        }
        let id = self.layer;
        let (w, tiles_x) = (self.w, self.tiles_x);
        let h = self.h;
        let s = self.settings;
        let main = self.rgb.map(|c| c as f32 / 255.0);
        let Some(layer) = doc.layer_mut(id) else {
            return;
        };
        let lock = layer.alpha_locked;
        let mut dirty_tiles: Vec<u32> = Vec::new();
        let mut flushed = std::collections::HashSet::new();
        for dab in pending {
            // capture undo tiles first
            for &(i, _, _) in &dab.px {
                let (x, y) = (i % w, i / w);
                let ti = (y / TILE) * tiles_x + x / TILE;
                if !self.touched[ti as usize] {
                    self.touched[ti as usize] = true;
                    let (tx, ty) = (x / TILE * TILE, y / TILE * TILE);
                    let rect = IRect::new(tx, ty, TILE.min(w - tx), TILE.min(h - ty));
                    self.wet_tiles.push(Tile::capture(&layer.pixels, w, rect));
                }
                if flushed.insert(ti) {
                    dirty_tiles.push(ti);
                }
            }
            let (q, qa) = self.reservoir;
            let pc = s.curve(dab.pressure);
            let bl = if s.pressure_blend {
                1.0 + (s.blending - 1.0) * pc
            } else {
                s.blending
            };
            let dab_alpha = (s.opacity * qa).clamp(0.0, 1.0);
            let irr = s.irregularity;
            // what is under the dab (before painting it)
            let (mut sw, mut sa, mut srgb) = (0f32, 0f32, [0f32; 3]);
            for &(i, cov, _) in &dab.px {
                let p = &layer.pixels[i as usize * 4..i as usize * 4 + 4];
                let ca = p[3] as f32 / 255.0;
                sw += cov;
                sa += cov * ca;
                for c in 0..3 {
                    srgb[c] += cov * ca * p[c] as f32 / 255.0;
                }
            }
            for &(i, cov, v) in &dab.px {
                let mut a = cov * dab_alpha;
                let mut blp = bl;
                if irr > 0.0 {
                    // bristle streaks (constant along the stroke) leave dry gaps,
                    // per-pixel grain makes the mix uneven
                    let streak = value_noise1(v * 7.0 + 31.0, self.mix_seed);
                    a *= 1.0 - irr + irr * crate::paint::smoothstep(0.2, 0.75, streak);
                    let g = hash01(self.mix_seed ^ (i as u64).wrapping_mul(0x9E37_79B9));
                    blp =
                        (blp + irr * (g - 0.5) * 0.9 + irr * (0.5 - streak) * 0.5).clamp(0.0, 1.0);
                }
                let p = &mut layer.pixels[i as usize * 4..i as usize * 4 + 4];
                let ca = p[3] as f32 / 255.0;
                let c = [p[0], p[1], p[2]].map(|v| v as f32 / 255.0);
                // the paint is tinted by what it touches
                let paint = mix_color(q, c, blp * ca, s.pigment);
                // dilution: the paint thins out over transparent areas
                a *= 1.0 - s.dilution * (1.0 - ca);
                let a = a.clamp(0.0, 1.0);
                if a <= 0.0 {
                    continue;
                }
                if lock {
                    if ca <= 0.0 {
                        continue;
                    }
                    let o = mix_color(c, paint, a, s.pigment);
                    for k in 0..3 {
                        p[k] = (o[k] * 255.0 + 0.5) as u8;
                    }
                    continue;
                }
                let oa = a + ca * (1.0 - a);
                let o = if ca <= 0.0 {
                    paint
                } else {
                    mix_color(c, paint, a / oa, s.pigment)
                };
                for k in 0..3 {
                    p[k] = (o[k].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
                p[3] = (oa * 255.0 + 0.5) as u8;
            }
            // the brush picks up the colour it went over
            if sw > 0.0 {
                let pa = sa / sw;
                let prgb = if sa > 0.0 { srgb.map(|v| v / sa) } else { q };
                let dirty = mix_color(q, prgb, bl * pa, s.pigment);
                let dirty_a = qa + (pa - qa) * s.dilution;
                let nq = mix_color(main, dirty, s.persistence, false);
                let na = 1.0 + (dirty_a - 1.0) * s.persistence;
                self.reservoir = (nq, na.clamp(0.05, 1.0));
            }
        }
        for ti in dirty_tiles {
            let tx = ti % tiles_x * TILE;
            let ty = ti / tiles_x * TILE;
            doc.mark_dirty(id, IRect::new(tx, ty, TILE.min(w - tx), TILE.min(h - ty)));
        }
    }

    /// Aborts the stroke, discarding the mask (and restoring the layer for wet strokes).
    pub fn cancel(&mut self, doc: &mut Document) {
        if !self.active {
            return;
        }
        if self.wet {
            self.pending.clear();
            let tiles = std::mem::take(&mut self.wet_tiles);
            let w = self.w;
            if doc.width == self.w && doc.height == self.h {
                if let Some(layer) = doc.layer_mut(self.layer) {
                    let mut rects = Vec::new();
                    for mut t in tiles {
                        t.swap_into(&mut layer.pixels, w);
                        rects.push(t.rect);
                    }
                    for r in rects {
                        doc.mark_dirty(self.layer, r);
                    }
                }
            }
            self.touched.fill(false);
        }
        self.clear_touched();
        self.active = false;
    }

    fn clear_touched(&mut self) {
        let (w, h) = (self.w, self.h);
        for (ti, t) in self.touched.iter_mut().enumerate() {
            if !*t {
                continue;
            }
            *t = false;
            let tx = ti as u32 % self.tiles_x * TILE;
            let ty = ti as u32 / self.tiles_x * TILE;
            let tw = TILE.min(w - tx);
            let th = TILE.min(h - ty);
            for y in ty..ty + th {
                let i = (y * w + tx) as usize;
                self.mask[i..i + tw as usize].fill(0.0);
            }
            self.mask_dirty.add(IRect::new(tx, ty, tw, th));
        }
    }

    /// Commits the stroke into its layer. Returns the undo edit (or `None` if nothing changed).
    pub fn end(&mut self, doc: &mut Document) -> Option<Edit> {
        if !self.active {
            return None;
        }
        self.stamp_deferred();
        if self.wet {
            self.flush(doc);
            self.active = false;
            self.last_end = Some(self.raw_last);
            self.touched.fill(false);
            let tiles = std::mem::take(&mut self.wet_tiles);
            if tiles.is_empty() {
                return None;
            }
            return Some(Edit::Pixels {
                layer: self.layer,
                tiles,
            });
        }
        self.active = false;
        self.last_end = Some(self.raw_last);
        if !self.any_dab {
            self.clear_touched();
            return None;
        }
        let (w, h) = (self.w, self.h);
        if doc.width != w || doc.height != h {
            self.clear_touched();
            return None;
        }
        let id = self.layer;
        let eraser = self.tool.is_eraser();
        let opacity = self.settings.opacity;
        let [cr, cg, cb] = self.rgb.map(|c| c as f32 / 255.0);
        let mut tiles = Vec::new();
        let mut dirty = Vec::new();
        {
            let Some(layer) = doc.layer_mut(id) else {
                self.clear_touched();
                return None;
            };
            let lock = layer.alpha_locked;
            for ti in 0..self.touched.len() {
                if !self.touched[ti] {
                    continue;
                }
                let tx = ti as u32 % self.tiles_x * TILE;
                let ty = ti as u32 / self.tiles_x * TILE;
                let tw = TILE.min(w - tx);
                let th = TILE.min(h - ty);
                let rect = IRect::new(tx, ty, tw, th);
                tiles.push(Tile::capture(&layer.pixels, w, rect));
                for y in ty..ty + th {
                    for x in tx..tx + tw {
                        let mi = (y * w + x) as usize;
                        let m = self.mask[mi];
                        if m <= 0.0 {
                            continue;
                        }
                        self.mask[mi] = 0.0;
                        let a = (m * opacity).min(1.0);
                        let p = &mut layer.pixels[mi * 4..mi * 4 + 4];
                        apply_paint(p, a, [cr, cg, cb], eraser, lock);
                    }
                }
                dirty.push(rect);
            }
            self.touched.fill(false);
        }
        for r in &dirty {
            doc.mark_dirty(id, *r);
            self.mask_dirty.add(*r);
        }
        Some(Edit::Pixels { layer: id, tiles })
    }
}

/// Applies a paint (or erase) contribution of alpha `a` to one straight-RGBA8 pixel.
/// Must match the preview logic in `composite.wgsl`.
#[inline]
pub fn apply_paint(p: &mut [u8], a: f32, rgb: [f32; 3], eraser: bool, alpha_lock: bool) {
    let da = p[3] as f32 / 255.0;
    if eraser {
        if alpha_lock {
            return;
        }
        let na = da * (1.0 - a);
        p[3] = (na * 255.0 + 0.5) as u8;
        return;
    }
    if alpha_lock {
        for i in 0..3 {
            let c = p[i] as f32 / 255.0;
            p[i] = ((c + (rgb[i] - c) * a) * 255.0 + 0.5) as u8;
        }
        return;
    }
    let oa = a + da * (1.0 - a);
    if oa <= 0.0 {
        return;
    }
    for i in 0..3 {
        let c = p[i] as f32 / 255.0;
        let o = (rgb[i] * a + c * da * (1.0 - a)) / oa;
        p[i] = (o * 255.0 + 0.5) as u8;
    }
    p[3] = (oa * 255.0 + 0.5) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Background;
    use crate::math::{dir_from_lonlat, px_to_dir};
    use std::f64::consts::PI;

    fn doc() -> Document {
        Document::new_blank(2048, 1024, Background::White, "bg", "l1")
    }
    fn bbox(d: &Document) -> (u32, u32, u32, u32, usize) {
        let l = d.active_layer().unwrap();
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (u32::MAX, u32::MAX, 0, 0, 0);
        for y in 0..d.height {
            for x in 0..d.width {
                if l.pixels[((y * d.width + x) * 4 + 3) as usize] > 8 {
                    n += 1;
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    y0 = y0.min(y);
                    y1 = y1.max(y);
                }
            }
        }
        (x0, y0, x1, y1, n)
    }
    fn hard(size: f32, polar: bool) -> BrushSettings {
        BrushSettings {
            size,
            hardness: 1.0,
            smoothing: 0.0,
            pressure_size: false,
            polar_correction: polar,
            ..Default::default()
        }
    }

    #[test]
    fn dab_round_at_equator_and_stretched_at_60() {
        for (lat, polar, ratio) in [(0.0, true, 1.0), (60.0f64, true, 2.0), (60.0, false, 1.0)] {
            let mut d = doc();
            let mut s = StrokeEngine::new();
            assert!(s.begin(
                &d,
                ToolKind::Brush,
                hard(60.0, polar),
                [255, 0, 0],
                dir_from_lonlat(PI, lat.to_radians()),
                1.0
            ));
            s.end(&mut d).unwrap();
            let (x0, y0, x1, y1, _) = bbox(&d);
            let r = (x1 - x0 + 1) as f64 / (y1 - y0 + 1) as f64;
            assert!(
                (r - ratio).abs() < 0.12,
                "lat {lat} polar {polar}: ratio {r}"
            );
        }
    }

    #[test]
    fn stroke_across_seam_has_no_spurious_line() {
        let mut d = doc();
        let mut s = StrokeEngine::new();
        let a = px_to_dir(2000.0, 512.0, 2048, 1024);
        let b = px_to_dir(48.0, 512.0, 2048, 1024);
        s.begin(&d, ToolKind::Brush, hard(20.0, true), [0, 0, 255], a, 1.0);
        s.add_point(b, 1.0, true);
        s.end(&mut d).unwrap();
        let l = d.active_layer().unwrap();
        let mid: usize = (900..1100)
            .map(|x| l.pixels[((512 * 2048 + x) * 4 + 3) as usize] as usize)
            .sum();
        assert_eq!(mid, 0, "stroke must go the short way across the seam");
        assert!(l.pixels[((512 * 2048) * 4 + 3) as usize] > 200);
        assert!(l.pixels[((512 * 2048 + 2047) * 4 + 3) as usize] > 200);
    }

    #[test]
    fn undo_tiles_restore_exactly() {
        let mut d = doc();
        let before = d.active_layer().unwrap().pixels.clone();
        let mut s = StrokeEngine::new();
        s.begin(
            &d,
            ToolKind::Brush,
            BrushSettings::default(),
            [10, 200, 30],
            dir_from_lonlat(PI, 0.2),
            0.7,
        );
        s.add_point(dir_from_lonlat(PI + 0.5, -0.1), 0.9, false);
        let mut e = s.end(&mut d).unwrap();
        assert_ne!(d.active_layer().unwrap().pixels, before);
        e.swap(&mut d);
        assert_eq!(d.active_layer().unwrap().pixels, before);
        assert!(
            s.mask().iter().all(|m| *m == 0.0),
            "mask cleared after commit"
        );
    }

    #[test]
    fn pressure_changes_width() {
        let mut d = doc();
        let mut s = StrokeEngine::new();
        let mut st = hard(80.0, true);
        st.pressure_size = true;
        s.begin(
            &d,
            ToolKind::Brush,
            st,
            [0, 0, 0],
            px_to_dir(800.0, 512.0, 2048, 1024),
            0.1,
        );
        for i in 1..=40 {
            s.add_point(
                px_to_dir(800.0 + i as f64 * 10.0, 512.0, 2048, 1024),
                0.1 + 0.9 * i as f32 / 40.0,
                true,
            );
        }
        s.end(&mut d);
        let l = d.active_layer().unwrap();
        let col = |x: u32| {
            (0..1024)
                .filter(|y| l.pixels[((y * 2048 + x) * 4 + 3) as usize] > 128)
                .count()
        };
        assert!(col(1150) > col(830) * 2, "{} vs {}", col(1150), col(830));
    }

    #[test]
    fn eraser_and_alpha_lock() {
        let mut p = [100, 100, 100, 255];
        apply_paint(&mut p, 1.0, [1.0, 0.0, 0.0], true, false);
        assert_eq!(p[3], 0);
        let mut p = [0, 0, 0, 0];
        apply_paint(&mut p, 1.0, [1.0, 0.0, 0.0], false, true);
        assert_eq!(p[3], 0, "alpha lock keeps transparent pixels transparent");
        let mut p = [0, 0, 0, 0];
        apply_paint(&mut p, 0.5, [1.0, 0.0, 0.0], false, false);
        assert_eq!(p, [255, 0, 0, 128]);
    }

    #[test]
    fn hidden_layer_refuses_stroke() {
        let mut d = doc();
        let id = d.active;
        d.layer_mut(id).unwrap().visible = false;
        let mut s = StrokeEngine::new();
        assert!(!s.begin(
            &d,
            ToolKind::Brush,
            BrushSettings::default(),
            [0, 0, 0],
            dir_from_lonlat(1.0, 0.0),
            1.0
        ));
    }

    #[test]
    fn pole_stroke_capped() {
        let mut d = doc();
        let mut s = StrokeEngine::new();
        s.begin(
            &d,
            ToolKind::Brush,
            hard(58.0, true),
            [0, 0, 0],
            dir_from_lonlat(PI, 88f64.to_radians()),
            1.0,
        );
        s.end(&mut d);
        let (x0, _, x1, _, _) = bbox(&d);
        assert!(x1 - x0 <= (12.3 * 58.0) as u32);
    }

    // ---- custom (sampled) tips -------------------------------------------------

    fn bar_tip(dynamics: crate::tip::TipDynamics) -> std::sync::Arc<PreparedTip> {
        // 64×8 horizontal bar
        PreparedTip::new(
            &[crate::tip::TipImage::new(64, 8, vec![255; 512])],
            &dynamics,
        )
    }
    fn custom_dab(tip: std::sync::Arc<PreparedTip>, lon: f64, lat: f64, polar: bool) -> Document {
        let mut d = doc();
        let mut s = StrokeEngine::new();
        s.set_tip(Some(tip));
        let mut b = hard(64.0, polar);
        b.spacing = 0.25;
        assert!(s.begin(
            &d,
            ToolKind::Custom,
            b,
            [255, 0, 0],
            dir_from_lonlat(lon, lat),
            1.0
        ));
        s.end(&mut d).unwrap();
        d
    }

    #[test]
    fn custom_tip_shape_and_angle() {
        let d = custom_dab(bar_tip(Default::default()), PI, 0.0, true);
        let (x0, y0, x1, y1, _) = bbox(&d);
        let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
        assert!((62..=67).contains(&bw), "bar width {bw}");
        assert!((7..=11).contains(&bh), "bar height {bh}");
        // rotated by 90° the bar stands upright
        let dy = crate::tip::TipDynamics {
            angle: 90.0,
            ..Default::default()
        };
        let d = custom_dab(bar_tip(dy), PI, 0.0, true);
        let (x0, y0, x1, y1, _) = bbox(&d);
        assert!((x1 - x0 + 1) < 12 && (y1 - y0 + 1) > 60);
        // roundness squashes the tip's height
        let dy = crate::tip::TipDynamics {
            roundness: 0.5,
            ..Default::default()
        };
        let sq = PreparedTip::new(&[crate::tip::TipImage::new(32, 32, vec![255; 1024])], &dy);
        let d = custom_dab(sq, PI, 0.0, true);
        let (x0, y0, x1, y1, _) = bbox(&d);
        let r = (x1 - x0 + 1) as f64 / (y1 - y0 + 1) as f64;
        assert!((r - 2.0).abs() < 0.2, "roundness ratio {r}");
    }

    #[test]
    fn custom_tip_polar_correction() {
        let eq = custom_dab(bar_tip(Default::default()), PI, 0.0, true);
        let hi = custom_dab(bar_tip(Default::default()), PI, 60f64.to_radians(), true);
        let w_eq = {
            let b = bbox(&eq);
            b.2 - b.0 + 1
        };
        let (x0, y0, x1, y1, _) = bbox(&hi);
        let w_hi = x1 - x0 + 1;
        let r = w_hi as f64 / w_eq as f64;
        assert!((r - 2.0).abs() < 0.1, "1/cos(60°) stretch, got {r}");
        assert!((y1 - y0 + 1) < 12, "height unchanged near the pole");
    }

    #[test]
    fn custom_tip_wraps_across_the_seam() {
        let d = custom_dab(bar_tip(Default::default()), 0.0, 0.0, true);
        let l = d.active_layer().unwrap();
        let row = 512 * 2048;
        assert!(l.pixels[(row * 4 + 3) as usize] > 200);
        assert!(l.pixels[((row + 2047) * 4 + 3) as usize] > 200);
        assert!(l.pixels[((row + 20) * 4 + 3) as usize] > 200);
        assert!(l.pixels[((row + 2030) * 4 + 3) as usize] > 200);
        assert_eq!(l.pixels[((row + 1024) * 4 + 3) as usize], 0);
    }

    #[test]
    fn custom_tip_jitter_is_deterministic_and_scatter_spreads() {
        let dy = crate::tip::TipDynamics {
            scatter: 3.0,
            size_jitter: 0.5,
            angle_jitter: 1.0,
            count: 3,
            ..Default::default()
        };
        let run = |seed: u64| {
            let mut d = doc();
            let mut s = StrokeEngine::new();
            s.seed = seed;
            s.set_tip(Some(PreparedTip::new(
                &[crate::tip::TipImage::ellipse(16, 1.0, 1.0)],
                &dy,
            )));
            let b = BrushSettings {
                spacing: 0.5,
                ..hard(16.0, true)
            };
            let a = px_to_dir(800.0, 512.0, 2048, 1024);
            s.begin(&d, ToolKind::Custom, b, [0, 0, 0], a, 1.0);
            s.add_point(px_to_dir(1200.0, 512.0, 2048, 1024), 1.0, true);
            s.end(&mut d).unwrap();
            d.active_layer().unwrap().pixels.clone()
        };
        let a = run(42);
        assert_eq!(a, run(42), "same seed → same stroke");
        assert_ne!(a, run(43));
        // scattering moves dabs away from the stroke line (y = 512) by up to 3×16/2 px
        let mut far = 0;
        for y in (512 - 30)..(512 + 30) {
            if (y - 512i32).abs() < 10 {
                continue;
            }
            for x in 800..1200 {
                if a[((y * 2048 + x) * 4 + 3) as usize] > 0 {
                    far += 1;
                }
            }
        }
        assert!(far > 50, "scatter must spread paint off the line ({far})");
    }

    #[test]
    fn custom_tip_spacing_controls_dab_count() {
        // A tiny dot tip; with spacing 2.0 (200 %) dabs are separated by gaps.
        let dot = PreparedTip::new(
            &[crate::tip::TipImage::ellipse(8, 1.0, 1.0)],
            &Default::default(),
        );
        let mut d = doc();
        let mut s = StrokeEngine::new();
        s.set_tip(Some(dot));
        let b = BrushSettings {
            spacing: 2.0,
            ..hard(8.0, true)
        };
        s.begin(
            &d,
            ToolKind::Custom,
            b,
            [0, 0, 0],
            px_to_dir(1000.0, 512.0, 2048, 1024),
            1.0,
        );
        s.add_point(px_to_dir(1160.0, 512.0, 2048, 1024), 1.0, true);
        s.end(&mut d).unwrap();
        let l = d.active_layer().unwrap();
        let mut runs = 0;
        let mut inside = false;
        for x in 990..1175 {
            let on = l.pixels[((512 * 2048 + x) * 4 + 3) as usize] > 100;
            if on && !inside {
                runs += 1;
            }
            inside = on;
        }
        // 160 px / (8 px × 2.0) = 10 gaps → 11 dabs
        assert!((10..=12).contains(&runs), "dabs: {runs}");
    }

    // ---- 1.3: wet mixing, spray particles, tip shapes, back-compat ----------------

    fn fill_active(d: &mut Document, x0: u32, x1: u32, rgba: [u8; 4]) {
        let w = d.width;
        let h = d.height;
        let id = d.active;
        let l = d.layer_mut(id).unwrap();
        for y in 0..h {
            for x in x0..x1 {
                let i = ((y * w + x) * 4) as usize;
                l.pixels[i..i + 4].copy_from_slice(&rgba);
            }
        }
    }

    fn oil() -> BrushSettings {
        BrushSettings {
            smoothing: 0.0,
            pressure_size: false,
            ..BrushSettings::preset(ToolKind::Brush)
        }
    }

    fn paint_line(
        d: &mut Document,
        s: &mut StrokeEngine,
        tool: ToolKind,
        b: BrushSettings,
        rgb: [u8; 3],
        from: (f64, f64),
        to: (f64, f64),
    ) -> Option<Edit> {
        let (w, h) = (d.width, d.height);
        s.begin(d, tool, b, rgb, px_to_dir(from.0, from.1, w, h), 1.0);
        for i in 1..=20 {
            let t = i as f64 / 20.0;
            let x = from.0 + (to.0 - from.0) * t;
            let y = from.1 + (to.1 - from.1) * t;
            s.add_point(px_to_dir(x, y, w, h), 1.0, i == 20);
            s.flush(d);
        }
        s.end(d)
    }

    #[test]
    fn settings_from_1_2_are_neutral_and_paint_like_before() {
        // A brush entry exactly as Panoraminal 1.2 saved it.
        let json = r#"{"size":40.0,"opacity":1.0,"flow":1.0,"hardness":0.6,"spacing":0.12,
            "smoothing":0.2,"pressure_size":true,"pressure_opacity":false,"min_size":0.08,
            "polar_correction":true}"#;
        let b: BrushSettings = serde_json::from_str(json).unwrap();
        assert_eq!(b, BrushSettings::classic(ToolKind::Brush));
        assert!(!b.is_wet(ToolKind::Brush));
        assert_eq!(b.tip_shape, TipShape::Round);
        assert_eq!(b.texture, PaperTexture::None);
        assert_eq!(b.particles, 0.0);
        // round trip keeps the new fields
        let o = oil();
        let back: BrushSettings =
            serde_json::from_str(&serde_json::to_string(&o).unwrap()).unwrap();
        assert_eq!(back, o);
    }

    #[test]
    fn default_styles() {
        // the brush now defaults to oil; the other tools keep their 1.2 presets
        assert!(BrushSettings::preset(ToolKind::Brush).is_wet(ToolKind::Brush));
        for t in [
            ToolKind::Pen,
            ToolKind::Airbrush,
            ToolKind::Eraser,
            ToolKind::Custom,
        ] {
            assert_eq!(BrushSettings::preset(t), BrushSettings::classic(t), "{t:?}");
        }
        for t in ToolKind::ALL {
            let st = builtin_styles(t);
            assert!(!st.is_empty());
            let mut ids: Vec<_> = st.iter().map(|s| s.id).collect();
            ids.dedup();
            assert_eq!(ids.len(), st.len());
            for s in st {
                let mut c = s.settings;
                c.clamp();
                assert_eq!(c, s.settings, "style {} must be within range", s.id);
            }
        }
        // the eraser never mixes
        let mut e = oil();
        e.blending = 1.0;
        assert!(!e.is_wet(ToolKind::Eraser));
    }

    #[test]
    fn pressure_curve() {
        let mut b = BrushSettings::default();
        assert_eq!(b.curve(0.5), 0.5);
        b.pressure_feel = 1.0;
        assert!(b.curve(0.5) > 0.75);
        b.pressure_feel = -1.0;
        assert!(b.curve(0.5) < 0.1);
        assert_eq!(b.curve(1.0), 1.0);
        assert_eq!(b.curve(0.0), 0.0);
    }

    #[test]
    fn oil_yellow_over_blue_mixes_unevenly() {
        let mut d = doc();
        fill_active(&mut d, 0, 2048, [30, 60, 220, 255]);
        let mut s = StrokeEngine::new();
        let e = paint_line(
            &mut d,
            &mut s,
            ToolKind::Brush,
            oil(),
            [255, 220, 0],
            (800.0, 512.0),
            (1100.0, 512.0),
        );
        assert!(e.is_some());
        let l = d.active_layer().unwrap();
        let (mut mixed, mut yellow, mut n) = (0, 0, 0);
        let mut greens = Vec::new();
        for y in 500..524u32 {
            for x in 900..1050u32 {
                let i = ((y * 2048 + x) * 4) as usize;
                let p = &l.pixels[i..i + 4];
                assert_eq!(p[3], 255, "opaque stays opaque");
                n += 1;
                let dy = (p[0] as i32 - 255).abs() + (p[1] as i32 - 220).abs() + p[2] as i32;
                let db =
                    (p[0] as i32 - 30).abs() + (p[1] as i32 - 60).abs() + (p[2] as i32 - 220).abs();
                if dy < 40 {
                    yellow += 1;
                } else if db > 40 {
                    mixed += 1;
                }
                greens.push(p[1] as f32);
            }
        }
        assert!(mixed * 3 > n, "most of the stroke is a mix ({mixed}/{n})");
        assert!(yellow < n, "the canvas colour must show through");
        // the mix is irregular, not a flat colour
        let mean = greens.iter().sum::<f32>() / greens.len() as f32;
        let var = greens.iter().map(|g| (g - mean) * (g - mean)).sum::<f32>() / greens.len() as f32;
        assert!(var.sqrt() > 4.0, "broken mix expected, std {}", var.sqrt());
        // a pigment mix of yellow and blue leans green rather than grey/purple
        let i = ((512 * 2048 + 1000) * 4) as usize;
        let p = &l.pixels[i..i + 3];
        assert!(p[1] as i32 >= p[0] as i32 - 10, "{p:?}");
    }

    #[test]
    fn oil_on_transparent_layer_lays_its_colour() {
        let mut d = doc();
        let mut s = StrokeEngine::new();
        let b = BrushSettings {
            irregularity: 0.0,
            texture: PaperTexture::None,
            ..oil()
        };
        paint_line(
            &mut d,
            &mut s,
            ToolKind::Brush,
            b,
            [200, 30, 40],
            (900.0, 512.0),
            (1100.0, 512.0),
        );
        let l = d.active_layer().unwrap();
        let i = ((512 * 2048 + 1000) * 4) as usize;
        let p = &l.pixels[i..i + 4];
        assert!(p[3] > 200, "{p:?}");
        assert!((p[0] as i32 - 200).abs() < 6 && p[1] < 40, "{p:?}");
    }

    #[test]
    fn wet_stroke_undo_and_cancel_restore_exactly() {
        let mut d = doc();
        fill_active(&mut d, 0, 1024, [200, 20, 20, 255]);
        fill_active(&mut d, 1024, 2048, [20, 20, 200, 255]);
        let before = d.active_layer().unwrap().pixels.clone();
        let mut s = StrokeEngine::new();
        // across the seam, through both colours
        let mut e = paint_line(
            &mut d,
            &mut s,
            ToolKind::Brush,
            oil(),
            [250, 230, 20],
            (1980.0, 300.0),
            (2100.0, 320.0),
        )
        .unwrap();
        let l = &d.active_layer().unwrap().pixels;
        let changed =
            |x: u32| l[((310 * 2048 + x) * 4) as usize] != before[((310 * 2048 + x) * 4) as usize];
        assert!(
            changed(2040) && changed(10),
            "paint on both sides of the seam"
        );
        assert!(!changed(1000), "no spurious line");
        e.swap(&mut d);
        assert_eq!(d.active_layer().unwrap().pixels, before);
        e.swap(&mut d);
        assert_ne!(d.active_layer().unwrap().pixels, before);
        e.swap(&mut d);
        // cancel restores too
        let w = d.width;
        s.begin(
            &d,
            ToolKind::Brush,
            oil(),
            [0, 255, 0],
            px_to_dir(500.0, 500.0, w, 1024),
            1.0,
        );
        s.add_point(px_to_dir(700.0, 520.0, w, 1024), 1.0, true);
        s.flush(&mut d);
        assert_ne!(d.active_layer().unwrap().pixels, before);
        s.cancel(&mut d);
        assert_eq!(d.active_layer().unwrap().pixels, before);
        assert!(!s.is_active());
    }

    #[test]
    fn wet_respects_alpha_lock() {
        let mut d = doc();
        fill_active(&mut d, 900, 1000, [20, 20, 200, 255]);
        let id = d.active;
        d.layer_mut(id).unwrap().alpha_locked = true;
        let mut s = StrokeEngine::new();
        paint_line(
            &mut d,
            &mut s,
            ToolKind::Brush,
            oil(),
            [255, 255, 0],
            (850.0, 512.0),
            (1050.0, 512.0),
        );
        let l = d.active_layer().unwrap();
        for x in [860u32, 1020] {
            assert_eq!(l.pixels[((512 * 2048 + x) * 4 + 3) as usize], 0);
        }
        let i = ((512 * 2048 + 950) * 4) as usize;
        assert_eq!(l.pixels[i + 3], 255);
        assert_ne!(&l.pixels[i..i + 3], &[20, 20, 200]);
    }

    fn spray() -> BrushSettings {
        BrushSettings {
            smoothing: 0.0,
            ..builtin_styles(ToolKind::Airbrush)
                .into_iter()
                .find(|s| s.id == "spray")
                .unwrap()
                .settings
        }
    }

    #[test]
    fn spray_particles_are_deterministic() {
        let run = |seed: u64| {
            let mut d = doc();
            let mut s = StrokeEngine::new();
            s.seed = seed;
            paint_line(
                &mut d,
                &mut s,
                ToolKind::Airbrush,
                spray(),
                [0, 0, 0],
                (800.0, 512.0),
                (1200.0, 512.0),
            );
            d.active_layer().unwrap().pixels.clone()
        };
        let a = run(5);
        assert_eq!(a, run(5), "same seed, same particles");
        assert_ne!(a, run(6), "another seed scatters differently");
    }

    #[test]
    fn spray_leaves_speckles_classic_airbrush_does_not() {
        let speckles = |b: BrushSettings| {
            let mut d = doc();
            let mut s = StrokeEngine::new();
            paint_line(
                &mut d,
                &mut s,
                ToolKind::Airbrush,
                b,
                [0, 0, 0],
                (800.0, 512.0),
                (1200.0, 512.0),
            );
            let l = d.active_layer().unwrap();
            // count isolated peaks along a row through the stroke
            let row: Vec<i32> = (900..1100)
                .map(|x| l.pixels[((520 * 2048 + x) * 4 + 3) as usize] as i32)
                .collect();
            let peaks = row
                .windows(3)
                .filter(|w| w[1] > w[0] + 6 && w[1] > w[2] + 6)
                .count();
            let paint: i32 = row.iter().sum();
            (peaks, paint)
        };
        let (p_classic, a_classic) = speckles(BrushSettings {
            smoothing: 0.0,
            ..BrushSettings::classic(ToolKind::Airbrush)
        });
        let (p_spray, a_spray) = speckles(spray());
        assert!(a_classic > 0 && a_spray > 0);
        assert!(p_classic <= 1, "smooth airbrush: {p_classic}");
        assert!(p_spray >= 5, "spray must leave particles: {p_spray}");
    }

    #[test]
    fn spray_wraps_across_the_seam() {
        let mut d = doc();
        let mut s = StrokeEngine::new();
        paint_line(
            &mut d,
            &mut s,
            ToolKind::Airbrush,
            spray(),
            [0, 0, 0],
            (2030.0, 512.0),
            (2066.0, 512.0),
        );
        let l = d.active_layer().unwrap();
        let side = |xs: std::ops::Range<u32>| -> u32 {
            let mut n = 0;
            for y in 440..584u32 {
                for x in xs.clone() {
                    n += (l.pixels[((y * 2048 + x) * 4 + 3) as usize] > 0) as u32;
                }
            }
            n
        };
        assert!(side(1950..2048) > 50 && side(0..100) > 50);
        assert_eq!(side(900..1100), 0);
    }

    #[test]
    fn tip_shapes_paint_and_flat_follows_the_stroke() {
        for shape in TipShape::ALL {
            let mut d = doc();
            let mut s = StrokeEngine::new();
            let b = BrushSettings {
                tip_shape: shape,
                follow_stroke: shape.follows_stroke_by_default(),
                ..hard(30.0, true)
            };
            paint_line(
                &mut d,
                &mut s,
                ToolKind::Pen,
                b,
                [0, 0, 0],
                (900.0, 512.0),
                (1100.0, 512.0),
            );
            let (_, _, _, _, n) = bbox(&d);
            assert!(n > 300, "{shape:?} painted {n}");
        }
        // a flat tip at 0° is thin across a stroke that follows it, wide otherwise
        let height = |follow: bool, vertical: bool| {
            let mut d = doc();
            let mut s = StrokeEngine::new();
            let b = BrushSettings {
                tip_shape: TipShape::Flat,
                follow_stroke: follow,
                ..hard(40.0, true)
            };
            let (from, to) = if vertical {
                ((1000.0, 400.0), (1000.0, 600.0))
            } else {
                ((900.0, 512.0), (1100.0, 512.0))
            };
            paint_line(&mut d, &mut s, ToolKind::Pen, b, [0, 0, 0], from, to);
            let (x0, y0, x1, y1, _) = bbox(&d);
            if vertical {
                x1 - x0
            } else {
                y1 - y0
            }
        };
        // horizontal stroke, tip along it: thin; vertical stroke without follow: thin
        // too (its width is the tip length); vertical stroke following: thin again.
        assert!(height(false, false) < 20, "{}", height(false, false));
        assert!(height(false, true) > 30, "{}", height(false, true));
        assert!(height(true, true) < 20, "{}", height(true, true));
    }

    #[test]
    fn texture_breaks_coverage() {
        let mut d = doc();
        let mut s = StrokeEngine::new();
        let b = BrushSettings {
            texture: PaperTexture::Grain,
            texture_strength: 1.0,
            flow: 1.0,
            ..hard(40.0, true)
        };
        paint_line(
            &mut d,
            &mut s,
            ToolKind::Pen,
            b,
            [0, 0, 0],
            (900.0, 512.0),
            (1100.0, 512.0),
        );
        let l = d.active_layer().unwrap();
        let mut holes = 0;
        for x in 950..1050u32 {
            holes += (l.pixels[((512 * 2048 + x) * 4 + 3) as usize] < 128) as u32;
        }
        assert!(
            holes > 5 && holes < 95,
            "grain must leave some holes: {holes}"
        );
    }
}
