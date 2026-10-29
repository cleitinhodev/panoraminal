//! Application state and the per-frame orchestration.

use crate::cli::Cli;
use crate::icons::Icons;
use crate::jobs::{JobResult, Jobs};
use crate::settings::{self, AnimChoice, Settings, ThemeChoice};
use crate::shortcuts::{Action, TABLE};
use crate::theme::{self, Palette};
use eframe::egui::{self, Pos2, Rect};
use panoraminal_core::document::Background;
use panoraminal_core::history::ops;
use panoraminal_core::{
    BrushSettings, Camera, Document, FlatView, History, LayerId, StrokeEngine, ToolKind, Vec3,
};
use panoraminal_i18n::{tr, trf, Lang};
use panoraminal_io::{ExportFormat, ProjectSnapshot};
use panoraminal_render::{GpuCanvas, ViewMode};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Paint(ToolKind),
    Eyedropper,
    Fill,
}

impl Tool {
    pub const ALL: [Tool; 6] = [
        Tool::Paint(ToolKind::Pen),
        Tool::Paint(ToolKind::Brush),
        Tool::Paint(ToolKind::Airbrush),
        Tool::Paint(ToolKind::Eraser),
        Tool::Eyedropper,
        Tool::Fill,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Tool::Paint(ToolKind::Pen) => "tool.pen",
            Tool::Paint(ToolKind::Brush) => "tool.brush",
            Tool::Paint(ToolKind::Airbrush) => "tool.airbrush",
            Tool::Paint(ToolKind::Eraser) => "tool.eraser",
            Tool::Eyedropper => "tool.eyedropper",
            Tool::Fill => "tool.fill",
        }
    }
    pub fn glyph(self) -> crate::icons::Glyph {
        use crate::icons::Glyph;
        match self {
            Tool::Paint(ToolKind::Pen) => Glyph::Pen,
            Tool::Paint(ToolKind::Brush) => Glyph::Brush,
            Tool::Paint(ToolKind::Airbrush) => Glyph::Airbrush,
            Tool::Paint(ToolKind::Eraser) => Glyph::Eraser,
            Tool::Eyedropper => Glyph::Eyedropper,
            Tool::Fill => Glyph::Bucket,
        }
    }
    pub fn action(self) -> Action {
        match self {
            Tool::Paint(ToolKind::Pen) => Action::ToolPen,
            Tool::Paint(ToolKind::Brush) => Action::ToolBrush,
            Tool::Paint(ToolKind::Airbrush) => Action::ToolAirbrush,
            Tool::Paint(ToolKind::Eraser) => Action::ToolEraser,
            Tool::Eyedropper => Action::ToolEyedropper,
            Tool::Fill => Action::ToolFill,
        }
    }
}

/// Something to do once unsaved changes have been dealt with.
#[derive(Clone, Debug)]
pub enum Pending {
    New,
    Open(Option<PathBuf>),
    Quit,
}

#[derive(Clone, Debug)]
pub struct NewDialog {
    pub width: u32,
    pub height: u32,
    pub background: Background,
    pub lock_ratio: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Ok,
    Error,
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub born: f64,
}

#[derive(Default)]
pub struct Dialogs {
    pub new: Option<NewDialog>,
    pub export_jpg: bool,
    pub settings: bool,
    pub help: bool,
    pub help_tab: usize,
    pub about: bool,
    pub unsaved: Option<Pending>,
    pub recovery: Option<(PathBuf, String)>,
    pub offset: Option<f32>,
    pub rename: Option<(LayerId, String)>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    None,
    Paint,
    Pick,
    Rotate,
}

pub struct ScreenshotState {
    pub path: PathBuf,
    pub frames_left: u32,
    pub requested: bool,
    pub done: Arc<AtomicBool>,
    pub failed: Arc<AtomicBool>,
}

pub struct PanoApp {
    pub s: Settings,
    pub doc: Document,
    pub hist: History,
    pub stroke: StrokeEngine,
    pub tool: Tool,
    pub brushes: HashMap<ToolKind, BrushSettings>,
    pub primary: [u8; 3],
    pub secondary: [u8; 3],
    pub cam: Camera,
    pub flat: FlatView,
    pub mode: ViewMode,
    pub file: Option<PathBuf>,
    pub ui_hidden: bool,
    pub dlg: Dialogs,
    pub jobs: Jobs,
    pub toasts: Vec<Toast>,
    pub pal: Palette,
    pub anim: bool,
    pub icons: Icons,
    pub thumbs: HashMap<LayerId, egui::TextureHandle>,
    pub stale_thumbs: HashSet<LayerId>,
    pub last_thumb_update: f64,
    pub gpu_ok: bool,
    pub gpu_info: String,
    pub max_dim: u32,
    pub cli: Cli,
    pub start_time: Option<f64>,
    pub last_autosave_check: f64,
    pub autosaved_state: u64,
    pub allow_close: bool,
    pub pressure: Option<f32>,
    pub drag: Drag,
    pub last_pointer: Option<Pos2>,
    pub canvas_rect: Rect,
    pub cursor_dir: Option<Vec3>,
    pub props_drag: Option<(LayerId, panoraminal_core::document::LayerProps)>,
    pub screenshot: Option<ScreenshotState>,
    pub applied_style: Option<(Palette, theme::Glow, bool)>,
    pub applied_lang: Option<Lang>,
    pub applied_scale: f32,
    pub title: String,
    pub splash_until: f64,
    pub dragged_layer: Option<usize>,
    pub hex_edit: String,
    pub busy_label: Option<&'static str>,
    pub view_anim: Option<(Camera, Camera, f64)>,
}

fn max_dim_for(device_max: u32) -> u32 {
    device_max.min(panoraminal_core::MAX_DIM)
}

impl PanoApp {
    pub fn new(cc: &eframe::CreationContext<'_>, cli: Cli) -> Self {
        let ctx = &cc.egui_ctx;
        let s = if cli.screenshot.is_some() {
            Settings::default()
        } else {
            Settings::load()
        };
        let lang = cli
            .lang
            .as_deref()
            .and_then(Lang::from_tag)
            .or_else(|| s.language.as_deref().and_then(Lang::from_tag))
            .unwrap_or_else(Lang::detect);
        panoraminal_i18n::set_lang(lang);
        crate::fonts::install(ctx, lang.needs_cjk());

        let mut gpu_ok = false;
        let mut gpu_info = String::from("—");
        let mut max_dim = panoraminal_core::MAX_DIM;
        let (w, h) = (s.new_width, s.new_height);
        if let Some(rs) = cc.wgpu_render_state.as_ref() {
            let info = rs.adapter.get_info();
            gpu_info = format!("{} ({:?})", info.name, info.backend);
            max_dim = max_dim_for(GpuCanvas::max_texture_side(&rs.device));
            let canvas =
                GpuCanvas::new(&rs.device, rs.target_format, w.min(max_dim), h.min(max_dim));
            rs.renderer.write().callback_resources.insert(canvas);
            gpu_ok = true;
        } else {
            log::error!("wgpu render state unavailable");
        }

        let mut brushes = HashMap::new();
        for t in ToolKind::ALL {
            brushes.insert(t, s.brush(t));
        }
        let doc = if cli.demo {
            crate::demo::build(4096.min(max_dim), 2048.min(max_dim / 2))
        } else {
            Document::new_blank(
                w.min(max_dim),
                h.min(max_dim),
                s.new_background,
                tr("layer.background"),
                &trf("layer.default_name", &[("n", &1)]),
            )
        };
        let mut hist = History::new();
        hist.max_bytes = s.undo_memory_mb as usize * 1024 * 1024;
        let now = ctx.input(|i| i.time);
        let anim = Self::anim_enabled(&s);
        let mut app = Self {
            primary: s.primary,
            secondary: s.secondary,
            brushes,
            doc,
            hist,
            stroke: StrokeEngine::new(),
            tool: Tool::Paint(ToolKind::Brush),
            cam: Camera::default(),
            flat: FlatView::default(),
            mode: if cli.view.as_deref() == Some("flat") {
                ViewMode::Flat
            } else {
                ViewMode::Sphere
            },
            file: None,
            ui_hidden: false,
            dlg: Dialogs::default(),
            jobs: Jobs::new(),
            toasts: Vec::new(),
            pal: Palette::dark(),
            anim,
            icons: Icons::load(ctx),
            thumbs: HashMap::new(),
            stale_thumbs: HashSet::new(),
            last_thumb_update: 0.0,
            gpu_ok,
            gpu_info,
            max_dim,
            start_time: None,
            last_autosave_check: now,
            autosaved_state: 0,
            allow_close: false,
            pressure: None,
            drag: Drag::None,
            last_pointer: None,
            canvas_rect: Rect::NOTHING,
            cursor_dir: None,
            props_drag: None,
            screenshot: cli.screenshot.clone().map(|path| ScreenshotState {
                path,
                frames_left: cli.frames.max(2),
                requested: false,
                done: Arc::new(AtomicBool::new(false)),
                failed: Arc::new(AtomicBool::new(false)),
            }),
            applied_style: None,
            applied_lang: None,
            applied_scale: 0.0,
            title: String::new(),
            splash_until: 0.0,
            dragged_layer: None,
            hex_edit: String::new(),
            busy_label: None,
            view_anim: None,
            s,
            cli,
        };
        if app.cli.demo {
            app.cam = Camera {
                lon: std::f64::consts::PI * 0.82,
                lat: 0.12,
                fov: 82.0,
            };
        }
        app.stale_thumbs = app.doc.layers.iter().map(|l| l.id).collect();
        app.hex_edit = hex(app.primary);
        match app.cli.window.as_deref() {
            Some("help") => {
                app.dlg.help = true;
                app.dlg.help_tab = app.cli.help_tab.unwrap_or(0);
            }
            Some("about") => app.dlg.about = true,
            Some("settings") => app.dlg.settings = true,
            Some("new") => app.open_new_dialog(),
            Some("export") => app.dlg.export_jpg = true,
            _ => {}
        }
        if let Some(f) = app.cli.file.clone() {
            app.open_path(ctx, f);
        } else if app.screenshot.is_none() {
            app.check_recovery();
        }
        app
    }

    pub fn anim_enabled(s: &Settings) -> bool {
        match s.animations {
            AnimChoice::On => true,
            AnimChoice::Off => false,
            AnimChoice::Auto => !settings::os_prefers_reduced_motion(),
        }
    }

    // ------------------------------------------------------------------ helpers

    pub fn toast(&mut self, ctx: &egui::Context, kind: ToastKind, text: impl Into<String>) {
        let born = ctx.input(|i| i.time);
        self.toasts.push(Toast {
            text: text.into(),
            kind,
            born,
        });
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    pub fn brush(&self) -> BrushSettings {
        let kind = match self.tool {
            Tool::Paint(k) => k,
            _ => ToolKind::Brush,
        };
        self.brushes
            .get(&kind)
            .copied()
            .unwrap_or_else(|| BrushSettings::preset(kind))
    }

    pub fn brush_mut(&mut self) -> Option<&mut BrushSettings> {
        match self.tool {
            Tool::Paint(k) => self.brushes.get_mut(&k),
            _ => None,
        }
    }

    pub fn doc_name(&self) -> String {
        self.file
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| tr("doc.untitled").to_string())
    }

    pub fn is_modified(&self) -> bool {
        self.hist.is_modified()
    }

    pub fn set_primary(&mut self, c: [u8; 3]) {
        if self.primary != c {
            self.primary = c;
            self.hex_edit = hex(c);
        }
    }

    pub fn commit_color(&mut self) {
        self.s.push_swatch(self.primary);
    }

    fn replace_document(&mut self, doc: Document, file: Option<PathBuf>) {
        self.stroke.reset();
        self.drag = Drag::None;
        self.doc = doc;
        self.hist.clear();
        self.file = file;
        self.thumbs.clear();
        self.stale_thumbs = self.doc.layers.iter().map(|l| l.id).collect();
        for l in self.doc.layers.iter().map(|l| l.id).collect::<Vec<_>>() {
            self.doc.mark_layer_full(l);
        }
        self.doc.touch_structure();
        self.autosaved_state = self.hist.state_id();
        self.props_drag = None;
        self.dlg.rename = None;
    }

    pub fn snapshot(&self) -> ProjectSnapshot {
        ProjectSnapshot::from_document(&self.doc)
    }

    // ------------------------------------------------------------------ file actions

    /// Runs `p` now, or asks about unsaved changes first.
    pub fn request(&mut self, ctx: &egui::Context, p: Pending) {
        if self.is_modified() {
            self.dlg.unsaved = Some(p);
        } else {
            self.run_pending(ctx, p);
        }
    }

    pub fn run_pending(&mut self, ctx: &egui::Context, p: Pending) {
        match p {
            Pending::New => self.open_new_dialog(),
            Pending::Open(None) => self.open_dialog(ctx),
            Pending::Open(Some(path)) => self.open_path(ctx, path),
            Pending::Quit => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    pub fn open_new_dialog(&mut self) {
        self.dlg.new = Some(NewDialog {
            width: self.s.new_width.min(self.max_dim),
            height: self.s.new_height.min(self.max_dim),
            background: self.s.new_background,
            lock_ratio: true,
        });
    }

    pub fn create_new(&mut self, d: &NewDialog) {
        let w = d.width.clamp(256, self.max_dim);
        let h = d.height.clamp(128, self.max_dim);
        self.s.new_width = w;
        self.s.new_height = h;
        self.s.new_background = d.background;
        let doc = Document::new_blank(
            w,
            h,
            d.background,
            tr("layer.background"),
            &trf("layer.default_name", &[("n", &1)]),
        );
        self.replace_document(doc, None);
        self.cam = Camera::default();
        self.flat = FlatView::fit(
            self.canvas_rect.width() as f64,
            self.canvas_rect.height() as f64,
        );
    }

    fn start_dir(&self) -> Option<PathBuf> {
        self.file
            .as_ref()
            .and_then(|f| f.parent().map(Path::to_path_buf))
            .or_else(|| self.s.last_dir.clone())
            .filter(|d| d.is_dir())
    }

    pub fn open_dialog(&mut self, ctx: &egui::Context) {
        let mut dlg = rfd::FileDialog::new()
            .set_title(tr("dialog.open.title"))
            .add_filter(
                tr("filter.supported"),
                &[
                    "pano", "json", "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff",
                ],
            )
            .add_filter(tr("filter.project"), &["pano", "json"])
            .add_filter(
                tr("filter.images"),
                &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"],
            );
        if let Some(d) = self.start_dir() {
            dlg = dlg.set_directory(d);
        }
        if let Some(p) = dlg.pick_file() {
            self.open_path(ctx, p);
        }
    }

    pub fn open_path(&mut self, ctx: &egui::Context, p: PathBuf) {
        if !crate::jobs::is_supported(&p) {
            self.toast(
                ctx,
                ToastKind::Error,
                trf("toast.unsupported", &[("file", &p.display())]),
            );
            return;
        }
        self.busy_label = Some("busy.opening");
        self.jobs.open(ctx, p, self.max_dim);
    }

    pub fn save(&mut self, ctx: &egui::Context, then: Option<Pending>) {
        match self
            .file
            .clone()
            .filter(|p| p.extension().map(|e| e == "pano").unwrap_or(false))
        {
            Some(p) => self.save_to(ctx, p, then),
            None => self.save_as(ctx, then),
        }
    }

    pub fn save_as(&mut self, ctx: &egui::Context, then: Option<Pending>) {
        let stem = self
            .file
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| {
                s.to_string_lossy()
                    .trim_end_matches(".json")
                    .trim_end_matches(".p360")
                    .trim_end_matches(".pano")
                    .to_string()
            })
            .unwrap_or_else(|| tr("doc.untitled").to_string());
        let mut dlg = rfd::FileDialog::new()
            .set_title(tr("dialog.save.title"))
            .add_filter(tr("filter.project"), &["pano"])
            .set_file_name(format!("{stem}.pano"));
        if let Some(d) = self.start_dir() {
            dlg = dlg.set_directory(d);
        }
        if let Some(mut p) = dlg.save_file() {
            if p.extension().is_none_or(|e| e != "pano") {
                p.set_extension("pano");
            }
            self.save_to(ctx, p, then);
        }
    }

    fn save_to(&mut self, ctx: &egui::Context, p: PathBuf, then: Option<Pending>) {
        self.finish_stroke();
        self.busy_label = Some("busy.saving");
        self.jobs
            .save(ctx, self.snapshot(), p, self.hist.state_id(), then);
    }

    pub fn export(&mut self, ctx: &egui::Context, jpg: bool) {
        self.finish_stroke();
        let stem = self
            .file
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().trim_end_matches(".p360").to_string())
            .unwrap_or_else(|| "panorama".into());
        let (ext, filter) = if jpg {
            ("jpg", tr("filter.jpg"))
        } else {
            ("png", tr("filter.png"))
        };
        let mut dlg = rfd::FileDialog::new()
            .set_title(tr("dialog.export.title"))
            .add_filter(filter, &[ext])
            .set_file_name(format!("{stem}.{ext}"));
        if let Some(d) = self.start_dir() {
            dlg = dlg.set_directory(d);
        }
        if let Some(mut p) = dlg.save_file() {
            if p.extension().is_none_or(|e| {
                !e.eq_ignore_ascii_case(ext) && !(jpg && e.eq_ignore_ascii_case("jpeg"))
            }) {
                p.set_extension(ext);
            }
            let fmt = if jpg {
                ExportFormat::Jpeg {
                    quality: self.s.jpeg_quality,
                    gpano: self.s.jpeg_gpano,
                }
            } else {
                ExportFormat::Png
            };
            self.busy_label = Some("busy.exporting");
            self.jobs.export(ctx, self.snapshot(), fmt, p);
        }
    }

    pub fn import_layer_dialog(&mut self, ctx: &egui::Context) {
        let mut dlg = rfd::FileDialog::new()
            .set_title(tr("dialog.import.title"))
            .add_filter(
                tr("filter.images"),
                &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"],
            );
        if let Some(d) = self.start_dir() {
            dlg = dlg.set_directory(d);
        }
        if let Some(p) = dlg.pick_file() {
            self.busy_label = Some("busy.importing");
            self.jobs
                .import_layer(ctx, p, self.doc.width, self.doc.height);
        }
    }

    fn check_recovery(&mut self) {
        if let Some(p) = settings::recovery_path() {
            if let Ok(meta) = std::fs::metadata(&p) {
                let when = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let txt = if when < 3600 {
                    trf("time.minutes_ago", &[("n", &(when / 60).max(1))])
                } else {
                    trf("time.hours_ago", &[("n", &(when / 3600))])
                };
                self.dlg.recovery = Some((p, txt));
            }
        }
    }

    pub fn discard_recovery(&mut self) {
        if let Some(p) = settings::recovery_path() {
            let _ = std::fs::remove_file(p);
        }
    }

    fn handle_jobs(&mut self, ctx: &egui::Context) {
        for r in self.jobs.poll() {
            if self.jobs.running == 0 {
                self.busy_label = None;
            }
            match r {
                JobResult::Saved { path, state, then } => {
                    self.hist.mark_saved_state(state);
                    self.s.last_dir = path.parent().map(Path::to_path_buf);
                    self.s.push_recent(&path);
                    self.file = Some(path.clone());
                    self.toast(
                        ctx,
                        ToastKind::Ok,
                        trf("toast.saved", &[("file", &file_name(&path))]),
                    );
                    if !self.is_modified() {
                        self.discard_recovery();
                    }
                    if let Some(p) = then {
                        self.run_pending(ctx, p);
                    }
                }
                JobResult::Exported { path } => {
                    self.s.last_dir = path.parent().map(Path::to_path_buf);
                    self.toast(
                        ctx,
                        ToastKind::Ok,
                        trf("toast.exported", &[("file", &file_name(&path))]),
                    );
                }
                JobResult::OpenedProject { path, snap } => {
                    let legacy = !path.extension().is_some_and(|e| e == "pano");
                    let doc = snap.into_document();
                    self.replace_document(doc, Some(path.clone()));
                    self.cam = Camera::default();
                    self.s.last_dir = path.parent().map(Path::to_path_buf);
                    self.s.push_recent(&path);
                    if legacy {
                        self.toast(ctx, ToastKind::Info, tr("toast.legacy_imported"));
                    } else {
                        self.toast(
                            ctx,
                            ToastKind::Ok,
                            trf("toast.opened", &[("file", &file_name(&path))]),
                        );
                    }
                }
                JobResult::OpenedImage { path, img } => {
                    let mut doc = Document::empty(img.width, img.height);
                    let mut l = doc.new_layer(&file_name(&path));
                    l.pixels = img.pixels;
                    doc.active = l.id;
                    doc.layers.push(l);
                    let top = doc.new_layer(&trf("layer.default_name", &[("n", &1)]));
                    doc.active = top.id;
                    doc.layers.push(top);
                    self.replace_document(doc, Some(path.clone()));
                    self.hist.mark_unsaved();
                    self.cam = Camera::default();
                    self.s.last_dir = path.parent().map(Path::to_path_buf);
                    self.s.push_recent(&path);
                    let ratio = img.width as f32 / img.height as f32;
                    if (ratio - 2.0).abs() > 0.02 {
                        self.toast(ctx, ToastKind::Info, tr("toast.not_2_1"));
                    } else if img.downscaled {
                        self.toast(
                            ctx,
                            ToastKind::Info,
                            trf("toast.downscaled", &[("w", &img.width), ("h", &img.height)]),
                        );
                    } else {
                        self.toast(
                            ctx,
                            ToastKind::Ok,
                            trf("toast.opened", &[("file", &file_name(&path))]),
                        );
                    }
                }
                JobResult::ImportedLayer { name, img } => {
                    if (img.width, img.height) == (self.doc.width, self.doc.height) {
                        if let Some(id) = ops::add_layer(&mut self.doc, &mut self.hist, &name) {
                            // Replace the new empty layer's pixels as part of the same history step.
                            if let Some(l) = self.doc.layer_mut(id) {
                                l.pixels = img.pixels;
                            }
                            self.doc.mark_layer_full(id);
                            self.stale_thumbs.insert(id);
                            self.toast(
                                ctx,
                                ToastKind::Ok,
                                trf("toast.imported", &[("name", &name)]),
                            );
                        } else {
                            self.toast(ctx, ToastKind::Error, tr("toast.max_layers"));
                        }
                    }
                }
                JobResult::Autosaved { state } => {
                    self.autosaved_state = state;
                }
                JobResult::Recovered { snap } => {
                    let doc = snap.into_document();
                    self.replace_document(doc, None);
                    self.hist.mark_unsaved();
                    self.toast(ctx, ToastKind::Ok, tr("toast.recovered"));
                }
                JobResult::Failed { what, path, err } => {
                    log::error!("{what} failed for {:?}: {err}", path);
                    if what != "autosave" {
                        let f = path.as_ref().map(|p| file_name(p)).unwrap_or_default();
                        let key = match what {
                            "save" => "error.save",
                            "export" => "error.export",
                            "open" => "error.open",
                            "import" => "error.import",
                            "recover" => "error.recover",
                            _ => "error.generic",
                        };
                        self.dlg.error = Some(format!("{}\n\n{err}", trf(key, &[("file", &f)])));
                    }
                }
            }
        }
    }

    fn autosave(&mut self, ctx: &egui::Context, now: f64) {
        if self.s.autosave_minutes == 0 || self.screenshot.is_some() || self.jobs.autosave_running {
            return;
        }
        if now - self.last_autosave_check < self.s.autosave_minutes as f64 * 60.0 {
            return;
        }
        self.last_autosave_check = now;
        if self.stroke.is_active()
            || !self.is_modified()
            || self.hist.state_id() == self.autosaved_state
        {
            return;
        }
        if let Some(p) = settings::recovery_path() {
            self.jobs
                .autosave(ctx, self.snapshot(), p, self.hist.state_id());
        }
    }

    // ------------------------------------------------------------------ editing

    pub fn finish_stroke(&mut self) {
        if self.stroke.is_active() {
            let before = self.doc.active;
            if let Some(e) = self.stroke.end(&mut self.doc) {
                let label = if self.stroke.tool().is_eraser() {
                    "hist.erase"
                } else {
                    "hist.stroke"
                };
                self.stale_thumbs.insert(self.stroke.layer());
                self.hist.push(e, label, before, before);
            }
        }
        self.drag = Drag::None;
    }

    pub fn undo(&mut self, ctx: &egui::Context) {
        self.finish_stroke();
        if let Some(l) = self.hist.undo_label() {
            self.hist.undo(&mut self.doc);
            self.mark_all_thumbs();
            self.toast(ctx, ToastKind::Info, trf("toast.undo", &[("what", &tr(l))]));
        }
    }

    pub fn redo(&mut self, ctx: &egui::Context) {
        self.finish_stroke();
        if let Some(l) = self.hist.redo_label() {
            self.hist.redo(&mut self.doc);
            self.mark_all_thumbs();
            self.toast(ctx, ToastKind::Info, trf("toast.redo", &[("what", &tr(l))]));
        }
    }

    pub fn mark_all_thumbs(&mut self) {
        self.stale_thumbs
            .extend(self.doc.layers.iter().map(|l| l.id));
    }

    pub fn new_layer(&mut self, ctx: &egui::Context) {
        self.finish_stroke();
        let n = self.doc.layers.len();
        let name = trf("layer.default_name", &[("n", &n)]);
        match ops::add_layer(&mut self.doc, &mut self.hist, &name) {
            Some(id) => {
                self.stale_thumbs.insert(id);
            }
            None => self.toast(ctx, ToastKind::Error, tr("toast.max_layers")),
        }
    }

    pub fn duplicate_layer(&mut self, ctx: &egui::Context) {
        self.finish_stroke();
        match ops::duplicate_layer(&mut self.doc, &mut self.hist, tr("layer.copy_suffix")) {
            Some(id) => {
                self.stale_thumbs.insert(id);
            }
            None => self.toast(ctx, ToastKind::Error, tr("toast.max_layers")),
        }
    }

    pub fn delete_layer(&mut self, ctx: &egui::Context) {
        self.finish_stroke();
        if !ops::delete_layer(&mut self.doc, &mut self.hist) {
            self.toast(ctx, ToastKind::Error, tr("toast.last_layer"));
        }
    }

    pub fn merge_down(&mut self, ctx: &egui::Context) {
        self.finish_stroke();
        if ops::merge_down(&mut self.doc, &mut self.hist) {
            self.mark_all_thumbs();
        } else {
            self.toast(ctx, ToastKind::Error, tr("toast.cannot_merge"));
        }
    }

    pub fn move_layer(&mut self, from: usize, to: usize) {
        self.finish_stroke();
        ops::move_layer(&mut self.doc, &mut self.hist, from, to);
    }

    pub fn clear_layer(&mut self) {
        self.finish_stroke();
        let id = self.doc.active;
        let len = self.doc.pixel_len();
        if let Some(l) = self.doc.layer_mut(id) {
            if l.is_empty() {
                return;
            }
            let old = std::mem::replace(&mut l.pixels, vec![0; len]);
            ops::replaced_pixels(&mut self.doc, &mut self.hist, id, old, "hist.clear");
            self.stale_thumbs.insert(id);
        }
    }

    pub fn fill_layer(&mut self) {
        self.finish_stroke();
        let id = self.doc.active;
        let c = self.primary;
        if let Some(l) = self.doc.layer_mut(id) {
            let mut px = l.pixels.clone();
            for p in px.chunks_exact_mut(4) {
                p.copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
            let old = std::mem::replace(&mut l.pixels, px);
            ops::replaced_pixels(&mut self.doc, &mut self.hist, id, old, "hist.fill_layer");
            self.stale_thumbs.insert(id);
        }
    }

    pub fn flip_horizontal(&mut self) {
        self.finish_stroke();
        let w = self.doc.width as usize;
        let mut edits = Vec::new();
        for l in self.doc.layers.iter_mut() {
            let old = l.pixels.clone();
            for row in l.pixels.chunks_exact_mut(w * 4) {
                let mut px: Vec<[u8; 4]> = row
                    .chunks_exact(4)
                    .map(|p| [p[0], p[1], p[2], p[3]])
                    .collect();
                px.reverse();
                for (d, s) in row.chunks_exact_mut(4).zip(px) {
                    d.copy_from_slice(&s);
                }
            }
            edits.push(panoraminal_core::Edit::LayerPixels {
                layer: l.id,
                pixels: old,
            });
        }
        let ids: Vec<_> = self.doc.layers.iter().map(|l| l.id).collect();
        for id in ids {
            self.doc.mark_layer_full(id);
        }
        let a = self.doc.active;
        self.hist
            .push(panoraminal_core::Edit::Group(edits), "hist.flip", a, a);
        self.mark_all_thumbs();
    }

    pub fn offset_heading(&mut self, degrees: f32) {
        self.finish_stroke();
        let dx = (degrees as f64 / 360.0 * self.doc.width as f64).round() as i64;
        ops::offset_all(&mut self.doc, &mut self.hist, dx);
        self.mark_all_thumbs();
    }

    pub fn set_tool(&mut self, t: Tool) {
        if self.tool != t {
            self.finish_stroke();
            self.tool = t;
        }
    }

    pub fn swap_colors(&mut self) {
        let p = self.primary;
        self.set_primary(self.secondary);
        self.secondary = p;
    }

    pub fn reset_view(&mut self, now: f64) {
        match self.mode {
            ViewMode::Sphere => {
                let target = Camera::default();
                if self.anim {
                    self.view_anim = Some((self.cam, target, now));
                } else {
                    self.cam = target;
                }
            }
            ViewMode::Flat => {
                self.flat = FlatView::fit(
                    self.canvas_rect.width() as f64,
                    self.canvas_rect.height() as f64,
                )
            }
        }
    }

    pub fn zoom(&mut self, factor: f64) {
        match self.mode {
            ViewMode::Sphere => self.cam.zoom(factor),
            ViewMode::Flat => {
                let r = self.canvas_rect;
                self.flat.zoom_at(
                    1.0 / factor,
                    r.width() as f64 * 0.5,
                    r.height() as f64 * 0.5,
                    r.width() as f64,
                    r.height() as f64,
                );
            }
        }
    }

    pub fn toggle_view(&mut self) {
        self.finish_stroke();
        self.mode = match self.mode {
            ViewMode::Sphere => {
                self.flat = FlatView::fit(
                    self.canvas_rect.width() as f64,
                    self.canvas_rect.height() as f64,
                );
                ViewMode::Flat
            }
            ViewMode::Flat => ViewMode::Sphere,
        };
    }

    pub fn run_action(&mut self, ctx: &egui::Context, a: Action) {
        let now = ctx.input(|i| i.time);
        match a {
            Action::ToolPen => self.set_tool(Tool::Paint(ToolKind::Pen)),
            Action::ToolBrush => self.set_tool(Tool::Paint(ToolKind::Brush)),
            Action::ToolAirbrush => self.set_tool(Tool::Paint(ToolKind::Airbrush)),
            Action::ToolEraser => self.set_tool(Tool::Paint(ToolKind::Eraser)),
            Action::ToolEyedropper => self.set_tool(Tool::Eyedropper),
            Action::ToolFill => self.set_tool(Tool::Fill),
            Action::SizeDown | Action::SizeUp => {
                if let Some(b) = self.brush_mut() {
                    let f = if a == Action::SizeUp {
                        1.0 / 0.85
                    } else {
                        0.85
                    };
                    b.size = (b.size * f).round().clamp(1.0, 1000.0);
                    if a == Action::SizeUp && b.size < 3.0 {
                        b.size += 1.0;
                    }
                }
            }
            Action::HardnessDown | Action::HardnessUp => {
                if let Some(b) = self.brush_mut() {
                    let d = if a == Action::HardnessUp { 0.1 } else { -0.1 };
                    b.hardness = ((b.hardness + d) * 10.0).round() / 10.0;
                    b.hardness = b.hardness.clamp(0.0, 1.0);
                }
            }
            Action::Undo => self.undo(ctx),
            Action::Redo => self.redo(ctx),
            Action::New => self.request(ctx, Pending::New),
            Action::Open => self.request(ctx, Pending::Open(None)),
            Action::Save => self.save(ctx, None),
            Action::SaveAs => self.save_as(ctx, None),
            Action::ExportPng => self.export(ctx, false),
            Action::ExportJpg => self.dlg.export_jpg = true,
            Action::ImportLayer => self.import_layer_dialog(ctx),
            Action::NewLayer => self.new_layer(ctx),
            Action::DuplicateLayer => self.duplicate_layer(ctx),
            Action::MergeDown => self.merge_down(ctx),
            Action::ClearLayer => self.clear_layer(),
            Action::ToggleView => self.toggle_view(),
            Action::ToggleGrid => self.s.show_grid = !self.s.show_grid,
            Action::ResetView => self.reset_view(now),
            Action::ZoomIn => self.zoom(1.0 / 1.2),
            Action::ZoomOut => self.zoom(1.2),
            Action::HideUi => self.ui_hidden = !self.ui_hidden,
            Action::Help => self.dlg.help = !self.dlg.help,
            Action::Fullscreen => {
                let fs = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fs));
            }
            Action::SwapColors => self.swap_colors(),
            Action::Settings => self.dlg.settings = true,
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    fn any_modal_open(&self) -> bool {
        self.dlg.new.is_some()
            || self.dlg.unsaved.is_some()
            || self.dlg.recovery.is_some()
            || self.dlg.error.is_some()
            || self.dlg.offset.is_some()
            || self.dlg.export_jpg
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.any_modal_open() {
            return;
        }
        let mut fired = Vec::new();
        ctx.input_mut(|i| {
            for sc in TABLE {
                for k in sc.keys {
                    if i.consume_shortcut(k) {
                        fired.push(sc.action);
                    }
                }
            }
        });
        // Undo/redo while a stroke is active should apply after the stroke ends.
        for a in fired {
            self.run_action(ctx, a);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.stroke.is_active() {
                self.stroke.cancel();
                self.drag = Drag::None;
            } else if self.dlg.help {
                self.dlg.help = false;
            } else if self.dlg.about {
                self.dlg.about = false;
            } else if self.dlg.settings {
                self.dlg.settings = false;
                self.s.save();
            } else if self.ui_hidden {
                self.ui_hidden = false;
            }
        }
    }

    fn apply_theme_and_lang(&mut self, ctx: &egui::Context) {
        let theme_choice = match self.cli.theme.as_deref() {
            Some("dark") => ThemeChoice::Dark,
            Some("light") => ThemeChoice::Light,
            _ => self.s.theme,
        };
        let dark = match theme_choice {
            ThemeChoice::Dark => true,
            ThemeChoice::Light => false,
            ThemeChoice::System => ctx
                .system_theme()
                .map(|t| t == egui::Theme::Dark)
                .unwrap_or(true),
        };
        self.pal = if dark {
            Palette::dark()
        } else {
            Palette::light()
        };
        self.anim = Self::anim_enabled_cached(&self.s);
        let key = (self.pal, self.s.glow, self.anim);
        if self.applied_style != Some(key) {
            ctx.set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            theme::apply_style(ctx, &self.pal, self.s.glow, self.anim);
            self.applied_style = Some(key);
        }
        let lang = self
            .cli
            .lang
            .as_deref()
            .and_then(Lang::from_tag)
            .or_else(|| self.s.language.as_deref().and_then(Lang::from_tag))
            .unwrap_or_else(Lang::detect_cached);
        if self.applied_lang != Some(lang) {
            if self.applied_lang.is_some() && lang.needs_cjk() {
                crate::fonts::install(ctx, true);
            }
            panoraminal_i18n::set_lang(lang);
            self.applied_lang = Some(lang);
        }
        if (self.applied_scale - self.s.ui_scale).abs() > 1e-3 {
            ctx.set_zoom_factor(self.s.ui_scale);
            self.applied_scale = self.s.ui_scale;
        }
    }

    fn anim_enabled_cached(s: &Settings) -> bool {
        use std::sync::OnceLock;
        static OS: OnceLock<bool> = OnceLock::new();
        match s.animations {
            AnimChoice::On => true,
            AnimChoice::Off => false,
            AnimChoice::Auto => !*OS.get_or_init(settings::os_prefers_reduced_motion),
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let t = format!(
            "{}{} — {}",
            if self.is_modified() { "• " } else { "" },
            self.doc_name(),
            crate::APP_NAME
        );
        if t != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(t.clone()));
            self.title = t;
        }
    }

    fn update_thumbs(&mut self, ctx: &egui::Context, now: f64) {
        if self.stale_thumbs.is_empty()
            || self.stroke.is_active()
            || now - self.last_thumb_update < 0.15
        {
            return;
        }
        self.last_thumb_update = now;
        let ids: Vec<_> = self.stale_thumbs.drain().collect();
        self.thumbs.retain(|id, _| self.doc.layer(*id).is_some());
        for id in ids {
            if self.doc.layer(id).is_none() {
                continue;
            }
            let (tw, th) = (96u32, 48u32);
            let px = self.doc.thumbnail(id, tw, th);
            let img = egui::ColorImage::from_rgba_unmultiplied([tw as usize, th as usize], &px);
            match self.thumbs.get_mut(&id) {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => {
                    let t = ctx.load_texture(
                        format!("thumb{}", id.0),
                        img,
                        egui::TextureOptions::LINEAR,
                    );
                    self.thumbs.insert(id, t);
                }
            }
        }
    }

    fn gpu_sync(&mut self, frame: &mut eframe::Frame) {
        // Thumbnails of layers that changed.
        for (id, _) in &self.doc.dirty {
            if !self.stroke.is_active() || *id != self.stroke.layer() {
                self.stale_thumbs.insert(*id);
            }
        }
        if let Some(rs) = frame.wgpu_render_state() {
            let mut r = rs.renderer.write();
            if let Some(c) = r.callback_resources.get_mut::<GpuCanvas>() {
                c.sync(&rs.device, &rs.queue, &mut self.doc, &mut self.stroke);
            }
        } else {
            self.doc.take_dirty();
        }
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.allow_close
                || !self.is_modified()
                || !self.s.confirm_on_close
                || self.screenshot.is_some()
            {
                self.on_exit();
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.dlg.unsaved = Some(Pending::Quit);
            }
        }
    }

    fn on_exit(&mut self) {
        if self.screenshot.is_some() {
            return;
        }
        self.s.primary = self.primary;
        self.s.secondary = self.secondary;
        for (k, b) in &self.brushes {
            self.s.brushes.insert(k.id().to_string(), *b);
        }
        self.s.save();
        if !self.is_modified() {
            self.discard_recovery();
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let files: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if let Some(first) = files.into_iter().next() {
            // Projects always open. Images open as a new document when Shift is
            // held or the current document has no real work in it.
            let trivial_doc = !self.is_modified()
                && self.doc.layers.len() <= 2
                && self.doc.layers.iter().skip(1).all(|l| l.is_empty());
            if crate::jobs::is_project(&first) || ctx.input(|i| i.modifiers.shift) || trivial_doc {
                self.request(ctx, Pending::Open(Some(first)));
            } else {
                // Dropping an image onto a document with work adds it as a layer.
                self.busy_label = Some("busy.importing");
                self.jobs
                    .import_layer(ctx, first, self.doc.width, self.doc.height);
            }
        }
    }

    fn screenshot_step(&mut self, ctx: &egui::Context) {
        let Some(ss) = self.screenshot.as_mut() else {
            return;
        };
        if ss.done.load(Ordering::Relaxed) {
            if ss.failed.load(Ordering::Relaxed) {
                eprintln!("screenshot failed");
                std::process::exit(1);
            }
            eprintln!("screenshot saved to {}", ss.path.display());
            self.screenshot = None;
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        ctx.request_repaint();
        if ss.frames_left > 0 {
            ss.frames_left -= 1;
            return;
        }
        if !ss.requested && self.jobs.running == 0 {
            ss.requested = true;
            let path = ss.path.clone();
            let done = ss.done.clone();
            let failed = ss.failed.clone();
            let c2 = ctx.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                egui::ScreenshotCallback::new(move |img| {
                    let [w, h] = img.size;
                    let mut raw = Vec::with_capacity(w * h * 4);
                    for p in &img.pixels {
                        raw.extend_from_slice(&p.to_array());
                    }
                    let ok = image::RgbaImage::from_raw(w as u32, h as u32, raw)
                        .map(|im| im.save(&path).is_ok())
                        .unwrap_or(false);
                    failed.store(!ok, Ordering::Relaxed);
                    done.store(true, Ordering::Relaxed);
                    c2.request_repaint();
                }),
            ));
        }
    }
}

pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    let v = match s.len() {
        3 => {
            let d: Vec<u8> = s
                .chars()
                .map(|c| c.to_digit(16).map(|x| x as u8 * 17))
                .collect::<Option<_>>()?;
            [d[0], d[1], d[2]]
        }
        6 => {
            let n = u32::from_str_radix(s, 16).ok()?;
            [(n >> 16) as u8, (n >> 8) as u8, n as u8]
        }
        _ => return None,
    };
    Some(v)
}

pub fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| p.display().to_string())
}

trait DetectCached {
    fn detect_cached() -> Lang;
}
impl DetectCached for Lang {
    fn detect_cached() -> Lang {
        use std::sync::OnceLock;
        static L: OnceLock<Lang> = OnceLock::new();
        *L.get_or_init(Lang::detect)
    }
}

impl eframe::App for PanoApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        if self.start_time.is_none() {
            self.start_time = Some(now);
            let splash =
                self.s.show_splash && !self.cli.no_splash && self.screenshot.is_none() && self.anim;
            self.splash_until = if splash { now + 1.9 } else { 0.0 };
        }
        self.apply_theme_and_lang(&ctx);
        self.handle_jobs(&ctx);
        self.handle_dropped_files(&ctx);
        self.handle_close(&ctx);
        self.handle_shortcuts(&ctx);
        if let Some((from, to, t0)) = self.view_anim {
            let t = ((now - t0) / 0.45).min(1.0);
            let e = 1.0 - (1.0 - t).powi(3);
            let mut dl = (to.lon - from.lon).rem_euclid(std::f64::consts::TAU);
            if dl > std::f64::consts::PI {
                dl -= std::f64::consts::TAU;
            }
            self.cam = Camera {
                lon: from.lon + dl * e,
                lat: from.lat + (to.lat - from.lat) * e,
                fov: from.fov + (to.fov - from.fov) * e,
            };
            self.cam.clamp();
            if t >= 1.0 {
                self.view_anim = None;
            } else {
                ctx.request_repaint();
            }
        }

        crate::ui::layout(self, ui, now);
        crate::ui::dialogs::show_all(self, &ctx);
        crate::ui::overlay(self, &ctx, now);

        self.update_thumbs(&ctx, now);
        self.gpu_sync(frame);
        self.autosave(&ctx, now);
        self.update_title(&ctx);
        if self.jobs.running > 0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if !self.stale_thumbs.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(160));
        }
        if self.s.autosave_minutes > 0 {
            ctx.request_repaint_after(std::time::Duration::from_secs(20));
        }
        self.screenshot_step(&ctx);
    }

    fn on_exit(&mut self) {
        PanoApp::on_exit(self);
    }
}
