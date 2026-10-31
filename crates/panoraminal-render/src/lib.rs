//! # panoraminal-render
//!
//! GPU side of Panoraminal, built on `wgpu` and integrated into egui through
//! `egui_wgpu` paint callbacks.
//!
//! * Every layer lives in its own `Rgba8Unorm` texture; only dirty rectangles are
//!   uploaded after an edit.
//! * Layers are composited on the GPU (one pass per visible layer, ping-ponging
//!   between two `Rgba16Float` targets, scissored to the dirty rectangles). The live
//!   brush stroke is previewed in the compositor from an `R32Float` coverage mask, so
//!   painting never re-uploads whole layers.
//! * The result is displayed either as an exact 360° view (per-pixel ray casting, no
//!   sphere mesh, seamless wrap) or as a flat equirectangular map.

use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu;
use panoraminal_core::place::{Placement, Projection};
use panoraminal_core::{BlendMode, Camera, Document, FlatView, IRect, LayerId, StrokeEngine};
use std::collections::HashMap;
use std::sync::Arc;

const MAX_PASSES: usize = panoraminal_core::MAX_LAYERS + 1;
const UNIFORM_STRIDE: u64 = 512;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CompositeU {
    opacity: f32,
    blend: u32,
    flags: u32,
    stroke_opacity: f32,
    color: [f32; 4],
    p_mode: [f32; 4],
    p_f: [f32; 4],
    p_r: [f32; 4],
    p_u: [f32; 4],
    p_c: [f32; 4],
    /// Inverse corner-pin homography rows; `p_h0[3]` = warp flags (1 pin, 2 mesh).
    p_h0: [f32; 4],
    p_h1: [f32; 4],
    p_h2: [f32; 4],
    /// Texture width, height (texels), highest mip level, unused.
    p_t: [f32; 4],
    /// Mesh control points, two per vec4.
    p_mesh: [[f32; 4]; 8],
}

/// A floating image to preview on top of one layer (non-destructive).
#[derive(Clone, Debug)]
pub struct PlacePreview {
    pub layer: LayerId,
    pub placement: Placement,
    /// Premultiplied RGBA8 pixels.
    pub rgba: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    /// Changes whenever `rgba` changes (triggers a re-upload).
    pub rev: u64,
    /// Extra mip levels after `rgba` (level k is `max(1, size >> k)`), used when the
    /// image is distorted.
    pub mips: Arc<Vec<Vec<u8>>>,
}

struct PlaceState {
    tex: Tex,
    rev: u64,
    levels: u32,
    tw: u32,
    th: u32,
    last: Option<(LayerId, Placement)>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct DisplayU {
    fwd: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    vp: [f32; 4],
    flat: [f32; 4],
    check_a: [f32; 4],
    check_b: [f32; 4],
    bg: [f32; 4],
    grid: [f32; 4],
}

/// How the canvas is displayed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Sphere,
    Flat,
}

/// Visual parameters of the display pass (colours are sRGB 0..1).
#[derive(Clone, Copy, Debug)]
pub struct DisplayStyle {
    pub checker_a: [f32; 3],
    pub checker_b: [f32; 3],
    pub checker_cell: f32,
    pub background: [f32; 3],
    pub wrap_dim: f32,
    pub grid: bool,
    pub grid_color: [f32; 3],
    pub grid_alpha: f32,
}

impl Default for DisplayStyle {
    fn default() -> Self {
        Self {
            checker_a: [0.80, 0.83, 0.88],
            checker_b: [0.93, 0.95, 0.98],
            checker_cell: 10.0,
            background: [0.03, 0.05, 0.11],
            wrap_dim: 0.45,
            grid: false,
            grid_color: [0.37, 0.82, 0.99],
            grid_alpha: 0.6,
        }
    }
}

struct Tex {
    tex: wgpu::Texture,
    view: wgpu::TextureView,
}

fn make_tex(
    device: &wgpu::Device,
    label: &str,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> Tex {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    Tex { tex, view }
}

#[derive(PartialEq, Clone)]
struct ChainKey {
    layers: Vec<(LayerId, u32, BlendMode, bool)>,
    stroke: Option<(LayerId, [u8; 3], u32, bool, bool)>,
    place: Option<LayerId>,
}

/// GPU state for one document. Stored in egui-wgpu's `CallbackResources`.
pub struct GpuCanvas {
    w: u32,
    h: u32,
    layers: HashMap<LayerId, Tex>,
    mask: Tex,
    comp: [Tex; 2],
    final_idx: usize,
    key: Option<ChainKey>,
    comp_pipeline: wgpu::RenderPipeline,
    comp_bgl: wgpu::BindGroupLayout,
    comp_ubuf: wgpu::Buffer,
    disp_pipeline: wgpu::RenderPipeline,
    disp_bgl: wgpu::BindGroupLayout,
    disp_ubuf: wgpu::Buffer,
    disp_bg: [wgpu::BindGroup; 2],
    sampler: wgpu::Sampler,
    place_sampler: wgpu::Sampler,
    place: PlaceState,
    place_u: Option<(LayerId, Placement)>,
    srgb_target: bool,
    /// Statistics of the last [`GpuCanvas::sync`] (for the debug overlay).
    pub stats: SyncStats,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SyncStats {
    pub uploaded_px: u64,
    pub composited_px: u64,
    pub passes: u32,
}

const COMP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl GpuCanvas {
    /// Largest texture side supported by the device.
    pub fn max_texture_side(device: &wgpu::Device) -> u32 {
        device.limits().max_texture_dimension_2d
    }

    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat, w: u32, h: u32) -> Self {
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let comp_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pano.comp.bgl"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                tex_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<CompositeU>() as u64
                        ),
                    },
                    count: None,
                },
            ],
        });
        let comp_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pano.composite"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/composite.wgsl").into()),
        });
        let comp_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pano.comp.layout"),
            bind_group_layouts: &[Some(&comp_bgl)],
            immediate_size: 0,
        });
        let comp_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pano.comp"),
            layout: Some(&comp_layout),
            vertex: wgpu::VertexState {
                module: &comp_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &comp_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(COMP_FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let comp_ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pano.comp.ubuf"),
            size: UNIFORM_STRIDE * MAX_PASSES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let disp_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pano.disp.bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let disp_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pano.display"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/display.wgsl").into()),
        });
        let disp_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pano.disp.layout"),
            bind_group_layouts: &[Some(&disp_bgl)],
            immediate_size: 0,
        });
        let disp_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pano.disp"),
            layout: Some(&disp_layout),
            vertex: wgpu::VertexState {
                module: &disp_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &disp_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(target_format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let disp_ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pano.disp.ubuf"),
            size: std::mem::size_of::<DisplayU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pano.sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let place_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pano.place.sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let place = PlaceState {
            tex: make_tex(
                device,
                "pano.place.dummy",
                1,
                1,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            ),
            rev: u64::MAX,
            levels: 1,
            tw: 1,
            th: 1,
            last: None,
        };
        let (mask, comp) = Self::make_targets(device, w, h);
        let disp_bg = Self::make_disp_bgs(device, &disp_bgl, &comp, &sampler, &disp_ubuf);
        Self {
            w,
            h,
            layers: HashMap::new(),
            mask,
            comp,
            final_idx: 0,
            key: None,
            comp_pipeline,
            comp_bgl,
            comp_ubuf,
            disp_pipeline,
            disp_bgl,
            disp_ubuf,
            disp_bg,
            sampler,
            place_sampler,
            place,
            place_u: None,
            srgb_target: target_format.is_srgb(),
            stats: SyncStats::default(),
        }
    }

    fn make_targets(device: &wgpu::Device, w: u32, h: u32) -> (Tex, [Tex; 2]) {
        let mask = make_tex(
            device,
            "pano.mask",
            w,
            h,
            wgpu::TextureFormat::R32Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let u = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT;
        let comp = [
            make_tex(device, "pano.comp0", w, h, COMP_FORMAT, u),
            make_tex(device, "pano.comp1", w, h, COMP_FORMAT, u),
        ];
        (mask, comp)
    }

    fn make_disp_bgs(
        device: &wgpu::Device,
        bgl: &wgpu::BindGroupLayout,
        comp: &[Tex; 2],
        sampler: &wgpu::Sampler,
        ubuf: &wgpu::Buffer,
    ) -> [wgpu::BindGroup; 2] {
        let mk = |t: &Tex| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pano.disp.bg"),
                layout: bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&t.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: ubuf.as_entire_binding(),
                    },
                ],
            })
        };
        [mk(&comp[0]), mk(&comp[1])]
    }

    pub fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    /// Drops all GPU state of the current document and resizes for a new one.
    pub fn reset(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        self.layers.clear();
        self.key = None;
        if (w, h) != (self.w, self.h) {
            self.w = w;
            self.h = h;
            let (mask, comp) = Self::make_targets(device, w, h);
            self.mask = mask;
            self.comp = comp;
            self.disp_bg = Self::make_disp_bgs(
                device,
                &self.disp_bgl,
                &self.comp,
                &self.sampler,
                &self.disp_ubuf,
            );
        }
    }

    fn upload_layer(queue: &wgpu::Queue, t: &Tex, px: &[u8], w: u32, r: IRect) {
        if r.is_empty() {
            return;
        }
        let off = ((r.y * w + r.x) * 4) as usize;
        let end = (((r.bottom() - 1) * w + r.right()) * 4) as usize;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &t.tex,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: r.x,
                    y: r.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &px[off..end],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: r.w,
                height: r.h,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Brings the GPU in sync with the document and the live stroke, then recomposites
    /// the changed regions. Call once per frame before painting.
    pub fn sync(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        doc: &mut Document,
        stroke: &mut StrokeEngine,
        place: Option<&PlacePreview>,
    ) {
        self.stats = SyncStats::default();
        if (doc.width, doc.height) != (self.w, self.h) {
            self.reset(device, doc.width, doc.height);
        }
        let (w, h) = (self.w, self.h);
        let full = IRect::full(w, h);
        let mut regions: Vec<IRect> = Vec::new();
        let mut full_comp = false;

        // Layer textures: drop stale, create new.
        self.layers
            .retain(|id, _| doc.layers.iter().any(|l| l.id == *id));
        for l in &doc.layers {
            if !self.layers.contains_key(&l.id) {
                let t = make_tex(
                    device,
                    "pano.layer",
                    w,
                    h,
                    wgpu::TextureFormat::Rgba8Unorm,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                );
                Self::upload_layer(queue, &t, &l.pixels, w, full);
                self.stats.uploaded_px += full.area();
                self.layers.insert(l.id, t);
                full_comp = true;
            }
        }
        // Dirty uploads.
        for (id, mut region) in doc.take_dirty() {
            let (Some(t), Some(l)) = (self.layers.get(&id), doc.layer(id)) else {
                continue;
            };
            let rects = region.take().unwrap_or_else(|| vec![full]);
            for r in rects {
                let r = r.intersect(&full);
                Self::upload_layer(queue, t, &l.pixels, w, r);
                self.stats.uploaded_px += r.area();
                regions.push(r);
            }
        }
        // Stroke mask.
        if let Some(rects) = {
            let r = &mut stroke.mask_dirty;
            if r.is_clean() {
                Some(Vec::new())
            } else {
                r.take()
            }
        }
        .or_else(|| Some(vec![full]))
        {
            let (mw, mh) = stroke.mask_size();
            if (mw, mh) == (w, h) {
                let bytes: &[u8] = bytemuck::cast_slice(stroke.mask());
                for r in rects {
                    let r = r.intersect(&full);
                    if r.is_empty() {
                        continue;
                    }
                    let off = ((r.y * w + r.x) * 4) as usize;
                    let end = (((r.bottom() - 1) * w + r.right()) * 4) as usize;
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &self.mask.tex,
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: r.x,
                                y: r.y,
                                z: 0,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        &bytes[off..end],
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(w * 4),
                            rows_per_image: None,
                        },
                        wgpu::Extent3d {
                            width: r.w,
                            height: r.h,
                            depth_or_array_layers: 1,
                        },
                    );
                    regions.push(r);
                }
            }
        }

        // Floating placed image: upload when new, recomposite old ∪ new bounds on change.
        let place = place.filter(|p| doc.layer(p.layer).is_some() && p.width > 0 && p.height > 0);
        let mut tex_changed = false;
        if let Some(p) = place {
            if p.rev != self.place.rev {
                let levels = 1 + p.mips.len() as u32;
                let tex = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("pano.place"),
                    size: wgpu::Extent3d {
                        width: p.width,
                        height: p.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: levels,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
                let t = Tex { tex, view };
                for (k, px) in std::iter::once(&*p.rgba).chain(p.mips.iter()).enumerate() {
                    let lw = (p.width >> k).max(1);
                    let lh = (p.height >> k).max(1);
                    if px.len() != (lw * lh * 4) as usize {
                        continue;
                    }
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &t.tex,
                            mip_level: k as u32,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        px,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(lw * 4),
                            rows_per_image: None,
                        },
                        wgpu::Extent3d {
                            width: lw,
                            height: lh,
                            depth_or_array_layers: 1,
                        },
                    );
                }
                self.place.levels = levels;
                self.place.tw = p.width;
                self.place.th = p.height;
                self.place.tex = t;
                self.place.rev = p.rev;
                // New pixels: refresh the area (the old bounds stay in `last`
                // so a shape change in the same frame is cleaned up too).
                tex_changed = true;
            }
        }
        let now = place.map(|p| (p.layer, p.placement));
        if tex_changed || now != self.place.last {
            for (_, pl) in self.place.last.iter().chain(now.iter()) {
                regions.extend(pl.pixel_bounds(w, h));
            }
            self.place.last = now;
        }
        self.place_u = now;

        // Did the layer chain (order, visibility, props, stroke parameters) change?
        let key = ChainKey {
            layers: doc
                .layers
                .iter()
                .map(|l| (l.id, l.opacity.to_bits(), l.blend, l.visible))
                .collect(),
            stroke: stroke.is_active().then(|| {
                let lock = doc
                    .layer(stroke.layer())
                    .map(|l| l.alpha_locked)
                    .unwrap_or(false);
                (
                    stroke.layer(),
                    stroke.color(),
                    stroke.opacity().to_bits(),
                    stroke.tool().is_eraser(),
                    lock,
                )
            }),
            place: now.map(|(l, _)| l),
        };
        if self.key.as_ref() != Some(&key) {
            // A stroke starting/ending only affects its own area, which the mask
            // regions already cover; anything else needs a full recomposite.
            let same_layers = self
                .key
                .as_ref()
                .map(|k| k.layers == key.layers)
                .unwrap_or(false);
            if !same_layers {
                full_comp = true;
            }
            self.key = Some(key.clone());
        }
        if full_comp {
            regions = vec![full];
        }
        regions.retain(|r| !r.is_empty());
        if regions.is_empty() {
            return;
        }
        if regions.len() > 8 {
            let b = regions.iter().skip(1).fold(regions[0], |a, r| a.union(r));
            regions = vec![b];
        }
        self.composite(device, queue, doc, &key, &regions);
    }

    fn composite(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        doc: &Document,
        key: &ChainKey,
        regions: &[IRect],
    ) {
        let visible: Vec<_> = doc.layers.iter().filter(|l| l.visible).collect();
        let mut uniforms = vec![0u8; UNIFORM_STRIDE as usize * MAX_PASSES];
        let mut bind_groups = Vec::new();
        let n = visible.len().max(1);
        let new_final = (n - 1) % 2;
        if new_final != self.final_idx && !regions.contains(&IRect::full(self.w, self.h)) {
            // Parity changed: the other target holds stale data outside the regions.
            return self.composite(device, queue, doc, key, &[IRect::full(self.w, self.h)]);
        }
        for (i, l) in visible.iter().enumerate() {
            let mut flags = if i == 0 { 1 } else { 0 };
            let mut u = CompositeU {
                opacity: l.opacity,
                blend: l.blend.shader_index(),
                flags: 0,
                stroke_opacity: 0.0,
                ..Zeroable::zeroed()
            };
            if let Some((sid, rgb, op, eraser, lock)) = key.stroke {
                if sid == l.id {
                    flags |= 2;
                    if eraser {
                        flags |= 4;
                    }
                    if lock {
                        flags |= 8;
                    }
                    u.stroke_opacity = f32::from_bits(op);
                    u.color = [
                        rgb[0] as f32 / 255.0,
                        rgb[1] as f32 / 255.0,
                        rgb[2] as f32 / 255.0,
                        1.0,
                    ];
                }
            }
            if let Some((pid, pl)) = self.place_u {
                if pid == l.id {
                    flags |= 16;
                    if l.alpha_locked {
                        flags |= 8;
                    }
                    let fr = pl.frame();
                    let v = |a: panoraminal_core::Vec3, w: f64| {
                        [a.x as f32, a.y as f32, a.z as f32, w as f32]
                    };
                    u.p_mode = [
                        if fr.projection == Projection::Gnomonic {
                            1.0
                        } else {
                            0.0
                        },
                        fr.cos as f32,
                        fr.sin as f32,
                        pl.opacity,
                    ];
                    u.p_f = v(fr.f, fr.sx);
                    u.p_r = v(fr.r, fr.sy);
                    let flips = (pl.flip_x as u32) | ((pl.flip_y as u32) << 1);
                    u.p_u = v(fr.u, flips as f64);
                    u.p_c = [fr.c.0 as f32, fr.c.1 as f32, self.w as f32, self.h as f32];
                    let wp = panoraminal_core::place::Warper::new(&pl.warp);
                    let mut wflags = 0u32;
                    if !wp.identity {
                        if !pl.warp.perspective_is_identity() {
                            wflags |= 1;
                        }
                        if pl.warp.has_mesh() {
                            wflags |= 2;
                        }
                    }
                    let hi = wp.hinv.0;
                    u.p_h0 = [hi[0] as f32, hi[1] as f32, hi[2] as f32, wflags as f32];
                    u.p_h1 = [hi[3] as f32, hi[4] as f32, hi[5] as f32, 0.0];
                    u.p_h2 = [hi[6] as f32, hi[7] as f32, hi[8] as f32, 0.0];
                    u.p_t = [
                        self.place.tw as f32,
                        self.place.th as f32,
                        (self.place.levels.max(1) - 1) as f32,
                        0.0,
                    ];
                    for (k, m) in pl.warp.mesh.chunks(2).enumerate() {
                        u.p_mesh[k] = [
                            m[0][0] as f32,
                            m[0][1] as f32,
                            m[1][0] as f32,
                            m[1][1] as f32,
                        ];
                    }
                }
            }
            u.flags = flags;
            let off = i * UNIFORM_STRIDE as usize;
            uniforms[off..off + std::mem::size_of::<CompositeU>()]
                .copy_from_slice(bytemuck::bytes_of(&u));
            let prev = &self.comp[(i + 1) % 2];
            let Some(lt) = self.layers.get(&l.id) else {
                continue;
            };
            bind_groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pano.comp.bg"),
                layout: &self.comp_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&prev.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&lt.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&self.mask.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&self.place.tex.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::Sampler(&self.place_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.comp_ubuf,
                            offset: 0,
                            size: wgpu::BufferSize::new(std::mem::size_of::<CompositeU>() as u64),
                        }),
                    },
                ],
            }));
        }
        queue.write_buffer(&self.comp_ubuf, 0, &uniforms);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pano.composite"),
        });
        if visible.is_empty() {
            let _ = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pano.clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.comp[0].view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        let area: u64 = regions.iter().map(|r| r.area()).sum();
        for (i, bg) in bind_groups.iter().enumerate() {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pano.layer.pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.comp[i % 2].view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.comp_pipeline);
            rp.set_bind_group(0, bg, &[(i as u64 * UNIFORM_STRIDE) as u32]);
            for r in regions {
                rp.set_scissor_rect(r.x, r.y, r.w, r.h);
                rp.draw(0..3, 0..1);
            }
            self.stats.passes += 1;
            self.stats.composited_px += area;
        }
        queue.submit(Some(enc.finish()));
        self.final_idx = new_final;
    }

    fn write_display(&self, queue: &wgpu::Queue, p: &DisplayParams) {
        let (f, r, u) = p.camera.basis();
        let ty = (p.camera.fov.to_radians() * 0.5).tan();
        let tx = ty * (p.viewport_px[2] / p.viewport_px[3].max(1.0)) as f64;
        let s = &p.style;
        let d = DisplayU {
            fwd: [f.x as f32, f.y as f32, f.z as f32, ty as f32],
            right: [r.x as f32, r.y as f32, r.z as f32, tx as f32],
            up: [
                u.x as f32,
                u.y as f32,
                u.z as f32,
                if p.mode == ViewMode::Flat { 1.0 } else { 0.0 },
            ],
            vp: p.viewport_px,
            flat: [
                p.flat.cx as f32,
                p.flat.cy as f32,
                p.flat.zoom as f32,
                s.checker_cell * p.pixels_per_point,
            ],
            check_a: [
                s.checker_a[0],
                s.checker_a[1],
                s.checker_a[2],
                if s.grid { 1.0 } else { 0.0 },
            ],
            check_b: [
                s.checker_b[0],
                s.checker_b[1],
                s.checker_b[2],
                if self.srgb_target { 1.0 } else { 0.0 },
            ],
            bg: [
                s.background[0],
                s.background[1],
                s.background[2],
                s.wrap_dim,
            ],
            grid: [
                s.grid_color[0],
                s.grid_color[1],
                s.grid_color[2],
                s.grid_alpha,
            ],
        };
        queue.write_buffer(&self.disp_ubuf, 0, bytemuck::bytes_of(&d));
    }
}

/// Everything the display pass needs for one frame.
#[derive(Clone, Copy, Debug)]
pub struct DisplayParams {
    pub camera: Camera,
    pub flat: FlatView,
    pub mode: ViewMode,
    /// Viewport in physical pixels: x, y, w, h.
    pub viewport_px: [f32; 4],
    pub pixels_per_point: f32,
    pub style: DisplayStyle,
}

/// egui paint callback drawing the canvas.
pub struct CanvasCallback(pub DisplayParams);

impl egui_wgpu::CallbackTrait for CanvasCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _sd: &egui_wgpu::ScreenDescriptor,
        _enc: &mut wgpu::CommandEncoder,
        res: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(c) = res.get::<GpuCanvas>() {
            c.write_display(queue, &self.0);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        rp: &mut wgpu::RenderPass<'static>,
        res: &egui_wgpu::CallbackResources,
    ) {
        if let Some(c) = res.get::<GpuCanvas>() {
            rp.set_pipeline(&c.disp_pipeline);
            rp.set_bind_group(0, &c.disp_bg[c.final_idx], &[]);
            rp.draw(0..3, 0..1);
        }
    }
}

pub use egui_wgpu;
