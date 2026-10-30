//! App actions for custom brushes and image placement.

use crate::app::{PanoApp, Pending, ToastKind, Tool};
use crate::brushlib::BrushLib;
use crate::placing::Floating;
use eframe::egui;
use panoraminal_core::place::{self, PlaceImage, Placement, Projection};
use panoraminal_core::{ToolKind, Vec3};
use panoraminal_i18n::{tr, trf};
use panoraminal_io::brushes::{CustomBrush, BRUSH_EXTS};
use panoraminal_render::ViewMode;
use std::path::PathBuf;
use std::sync::Arc;

const IMAGE_EXTS: [&str; 7] = ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"];

impl PanoApp {
    // ------------------------------------------------------------ custom brushes

    /// Hands the selected custom brush tip to the stroke engine.
    pub fn sync_tip(&mut self) {
        if self.stroke.is_active() {
            return;
        }
        let tip = self.blib.prepared();
        let same = match (&tip, self.stroke.tip()) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.stroke.set_tip(tip);
        }
    }

    pub fn select_brush(&mut self, id: &str) {
        self.finish_stroke();
        let Some(b) = self.blib.lib.find(id) else {
            return;
        };
        let cur = self
            .brushes
            .get(&ToolKind::Custom)
            .copied()
            .unwrap_or_else(|| panoraminal_core::BrushSettings::preset(ToolKind::Custom));
        let s = BrushLib::settings_for(b, &cur);
        self.brushes.insert(ToolKind::Custom, s);
        self.blib.selected = Some(id.to_string());
        self.sync_tip();
    }

    pub fn import_brushes_dialog(&mut self, ctx: &egui::Context) {
        let mut dlg = rfd::FileDialog::new()
            .set_title(tr("brushlib.import_title"))
            .add_filter(tr("brushlib.filter"), BRUSH_EXTS);
        if let Some(d) = self.s.last_dir.clone().filter(|d| d.is_dir()) {
            dlg = dlg.set_directory(d);
        }
        if let Some(v) = dlg.pick_files() {
            if !v.is_empty() {
                self.busy_label = Some("busy.importing");
                self.jobs.import_brushes(ctx, v);
            }
        }
    }

    pub fn brushes_imported(
        &mut self,
        ctx: &egui::Context,
        brushes: Vec<CustomBrush>,
        failed: Vec<(PathBuf, String)>,
    ) {
        let total = brushes.len();
        let mut notices: Vec<&'static str> = Vec::new();
        for b in &brushes {
            for n in &b.notices {
                if !notices.contains(&n.key()) {
                    notices.push(n.key());
                }
            }
        }
        let n = self.blib.import(brushes);
        if n > 0 {
            if let Some(id) = self.blib.selected.clone() {
                self.select_brush(&id);
            }
            self.set_tool(Tool::Paint(ToolKind::Custom));
            self.toast(
                ctx,
                ToastKind::Ok,
                trf("brushlib.toast.imported", &[("n", &n)]),
            );
            if !notices.is_empty() {
                let list: Vec<String> = notices.iter().map(|k| tr(k).to_string()).collect();
                self.toast(
                    ctx,
                    ToastKind::Info,
                    trf("brushlib.toast.approx", &[("list", &list.join(", "))]),
                );
            }
        } else if total > 0 {
            self.toast(ctx, ToastKind::Info, tr("brushlib.toast.none_new"));
        }
        if !failed.is_empty() {
            let msg: Vec<String> = failed
                .iter()
                .map(|(p, e)| {
                    format!(
                        "{}\n{e}",
                        trf("error.import", &[("file", &crate::app::file_name(p))])
                    )
                })
                .collect();
            self.dlg.error = Some(msg.join("\n\n"));
        }
    }

    // ------------------------------------------------------------ placing images

    pub fn insert_image_dialog(&mut self, ctx: &egui::Context) {
        let mut dlg = rfd::FileDialog::new()
            .set_title(tr("place.dialog_title"))
            .add_filter(tr("filter.images"), &IMAGE_EXTS);
        if let Some(d) = self.s.last_dir.clone().filter(|d| d.is_dir()) {
            dlg = dlg.set_directory(d);
        }
        if let Some(p) = dlg.pick_file() {
            self.busy_label = Some("busy.importing");
            self.jobs.place_image(ctx, p);
        }
    }

    /// Opens a dropped / pasted file: projects open, brush files import, images are
    /// placed (or opened as a new document with Shift, or when the current document
    /// is still empty and the image is a 2:1 panorama).
    pub fn open_or_place(&mut self, ctx: &egui::Context, p: PathBuf, shift: bool) {
        if crate::jobs::is_brush_file(&p) {
            self.busy_label = Some("busy.importing");
            self.jobs.import_brushes(ctx, vec![p]);
            return;
        }
        if crate::jobs::is_project(&p) {
            self.request(ctx, Pending::Open(Some(p)));
            return;
        }
        if !crate::jobs::is_supported(&p) {
            self.toast(
                ctx,
                ToastKind::Error,
                trf("toast.unsupported", &[("file", &p.display())]),
            );
            return;
        }
        let trivial_doc = !self.is_modified()
            && self.doc.layers.len() <= 2
            && self.doc.layers.iter().skip(1).all(|l| l.is_empty());
        let panorama = image::ImageReader::open(&p)
            .ok()
            .and_then(|r| r.with_guessed_format().ok())
            .and_then(|r| r.into_dimensions().ok())
            .map(|(w, h)| (w as f64 / h.max(1) as f64 - 2.0).abs() < 0.03)
            .unwrap_or(false);
        if shift || (trivial_doc && panorama) {
            self.request(ctx, Pending::Open(Some(p)));
        } else {
            self.busy_label = Some("busy.importing");
            self.jobs.place_image(ctx, p);
        }
    }

    /// Ctrl+V: images from the clipboard, or pasted file paths / URIs.
    pub fn handle_paste(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            match ev {
                egui::Event::PasteImage(img) => {
                    let px: Vec<u8> = img
                        .pixels
                        .iter()
                        .flat_map(|c| c.to_srgba_unmultiplied())
                        .collect();
                    let [w, h] = img.size;
                    self.place_pixels(ctx, tr("place.pasted").to_string(), w as u32, h as u32, &px);
                }
                egui::Event::Paste(text) => {
                    let paths = parse_paths(&text);
                    let shift = ctx.input(|i| i.modifiers.shift);
                    if let Some(p) = paths.into_iter().next() {
                        self.open_or_place(ctx, p, shift);
                    }
                }
                _ => {}
            }
        }
    }

    /// Edit › Paste image (Ctrl+Shift+V): reads the clipboard image directly.
    pub fn paste_image_from_clipboard(&mut self, ctx: &egui::Context) {
        let got = arboard::Clipboard::new().and_then(|mut c| match c.get_image() {
            Ok(img) => Ok(Ok((
                img.width as u32,
                img.height as u32,
                img.bytes.into_owned(),
            ))),
            Err(_) => c.get_text().map(Err),
        });
        match got {
            Ok(Ok((w, h, px))) if px.len() == (w * h * 4) as usize => {
                self.place_pixels(ctx, tr("place.pasted").to_string(), w, h, &px);
            }
            Ok(Err(text)) => match parse_paths(&text).into_iter().next() {
                Some(p) => self.open_or_place(ctx, p, false),
                None => self.toast(ctx, ToastKind::Info, tr("place.toast.no_clipboard_image")),
            },
            _ => self.toast(ctx, ToastKind::Info, tr("place.toast.no_clipboard_image")),
        }
    }

    fn place_pixels(&mut self, ctx: &egui::Context, name: String, w: u32, h: u32, px: &[u8]) {
        if w == 0 || h == 0 {
            return;
        }
        let img = PlaceImage::from_rgba8(w, h, px);
        self.start_placing(ctx, name, img);
    }

    /// The direction at the centre of the canvas and the vertical field of view.
    fn view_center(&self) -> (Vec3, f64) {
        match self.mode {
            ViewMode::Sphere => (self.cam.basis().0, self.cam.fov.to_radians()),
            ViewMode::Flat => {
                let d = self
                    .screen_to_dir(self.canvas_rect.center())
                    .unwrap_or_else(|| self.cam.basis().0);
                (d, 60f64.to_radians())
            }
        }
    }

    /// The direction new projector placements look along.
    pub fn view_forward(&self) -> Vec3 {
        self.view_center().0
    }

    pub fn begin_floating(&mut self, name: String, img: PlaceImage) {
        self.finish_stroke();
        let (c, fov) = self.view_center();
        let aspect = img.width as f64 / img.height.max(1) as f64;
        let p = Placement::new(self.s.place_projection, c, c, fov, aspect, 0.6);
        self.floating = Some(Floating::new(img, name, p));
    }

    pub fn start_placing(&mut self, ctx: &egui::Context, name: String, img: PlaceImage) {
        self.begin_floating(name, img);
        self.toast(ctx, ToastKind::Info, tr("place.toast.hint"));
    }

    pub fn cancel_placement(&mut self) {
        if let Some(f) = self.floating.take() {
            self.s.place_projection = f.p.projection;
        }
        self.drag = crate::app::Drag::None;
    }

    /// Bakes the floating image into the active layer (or a new layer).
    pub fn commit_placement(&mut self, ctx: &egui::Context, new_layer: bool) {
        let Some(f) = self.floating.take() else {
            return;
        };
        self.drag = crate::app::Drag::None;
        if !new_layer {
            match self.doc.active_layer() {
                Some(l) if l.visible => {}
                Some(_) => {
                    self.toast(ctx, ToastKind::Error, tr("toast.layer_hidden"));
                    self.floating = Some(f);
                    return;
                }
                None => {
                    self.toast(ctx, ToastKind::Error, tr("toast.no_layer"));
                    self.floating = Some(f);
                    return;
                }
            }
        }
        let img = f.img.prefiltered_for(&f.p, self.doc.width);
        let name = f.name.clone();
        let layer_name = new_layer.then_some(name.as_str());
        match place::commit(&mut self.doc, &mut self.hist, &f.p, &img, layer_name) {
            Some(id) => {
                self.s.place_projection = f.p.projection;
                self.stale_thumbs.insert(id);
                self.toast(
                    ctx,
                    ToastKind::Ok,
                    trf("place.toast.merged", &[("name", &name)]),
                );
            }
            None => {
                self.toast(
                    ctx,
                    ToastKind::Error,
                    tr(if new_layer {
                        "toast.max_layers"
                    } else {
                        "place.toast.nothing"
                    }),
                );
                if new_layer {
                    self.floating = Some(f);
                }
            }
        }
    }

    pub fn set_place_projection(&mut self, proj: Projection) {
        let fwd = self.view_forward();
        if let Some(f) = self.floating.as_mut() {
            f.set_projection(proj, fwd);
            self.s.place_projection = proj;
        }
    }

    pub fn reaim_placement(&mut self) {
        let fwd = self.view_forward();
        if let Some(f) = self.floating.as_mut() {
            f.checkpoint();
            f.p.reaim(fwd);
        }
    }

    /// `--place-demo wrap|gnomonic`: floats a generated picture (docs screenshots).
    pub fn place_demo(&mut self, mode: &str) {
        let proj = if mode.eq_ignore_ascii_case("wrap") {
            Projection::Wrap
        } else {
            Projection::Gnomonic
        };
        self.s.place_projection = proj;
        let (w, h) = (960u32, 640u32);
        let px = demo_picture(w, h);
        self.begin_floating("demo".into(), PlaceImage::from_rgba8(w, h, &px));
        if let Some(f) = self.floating.as_mut() {
            f.p.rotation = 8f64.to_radians();
            if proj == Projection::Wrap {
                // Show how a wrap patch bends with the sphere: tilt it up towards the sky.
                f.p.half_w *= 1.6;
                f.p.half_h *= 1.6;
                let (_, _, u) = panoraminal_core::place::view_basis(f.p.center);
                f.p.center = (f.p.center + u * 0.45).normalize();
            }
            f.p.clamp();
        }
        if proj == Projection::Gnomonic && self.mode == ViewMode::Sphere {
            // Turn the camera so the projector's perspective is visible.
            self.cam.lon += 28f64.to_radians();
            self.cam.lat += 6f64.to_radians();
            self.cam.clamp();
        }
    }
}

/// File paths in pasted text (plain paths or `file://` URIs, one per line).
pub fn parse_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let l = l.trim_matches('"');
            let p = if let Some(rest) = l.strip_prefix("file://") {
                // file:///home/x or file://localhost/home/x
                let rest = rest.strip_prefix("localhost").unwrap_or(rest);
                let dec = percent_decode(rest);
                // Windows: file:///C:/x
                if dec.len() > 2 && dec.as_bytes()[0] == b'/' && dec.as_bytes()[2] == b':' {
                    PathBuf::from(&dec[1..])
                } else {
                    PathBuf::from(dec)
                }
            } else {
                PathBuf::from(l)
            };
            (p.is_file() && (crate::jobs::is_supported(&p) || crate::jobs::is_brush_file(&p)))
                .then_some(p)
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = h {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A colourful test picture: sunset gradient, sun, mountains, a frame and a grid so
/// the projection is easy to read in screenshots.
pub fn demo_picture(w: u32, h: u32) -> Vec<u8> {
    let mut px = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
            let mix = |a: [f32; 3], b: [f32; 3], t: f32| {
                [
                    a[0] + (b[0] - a[0]) * t,
                    a[1] + (b[1] - a[1]) * t,
                    a[2] + (b[2] - a[2]) * t,
                ]
            };
            let mut c = mix(
                [40.0, 30.0, 120.0],
                [255.0, 140.0, 90.0],
                (v / 0.7).min(1.0),
            );
            // Sun.
            let d = ((u - 0.66) * w as f32 / h as f32).hypot(v - 0.5);
            if d < 0.16 {
                c = mix(c, [255.0, 236.0, 150.0], (1.0 - d / 0.16).min(0.25) * 4.0);
            }
            // Mountains.
            let ridge = 0.62 + 0.08 * (u * 11.0).sin() + 0.05 * (u * 23.0 + 1.0).sin();
            if v > ridge {
                c = mix([46.0, 32.0, 80.0], [20.0, 16.0, 40.0], (v - ridge) * 3.0);
            }
            // Grid + frame.
            let gx = (u * 12.0).fract();
            let gy = (v * 8.0).fract();
            if gx < 0.012 || gy < 0.018 {
                c = mix(c, [255.0, 255.0, 255.0], 0.35);
            }
            let edge = u
                .min(1.0 - u)
                .min(v.min(1.0 - v) * h as f32 / w as f32 * 1.5);
            if edge < 0.012 {
                c = [255.0, 255.0, 255.0];
            }
            let i = ((y * w + x) * 4) as usize;
            px[i] = c[0] as u8;
            px[i + 1] = c[1] as u8;
            px[i + 2] = c[2] as u8;
            px[i + 3] = 255;
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pasted_paths_and_uris() {
        let dir = std::env::temp_dir().join(format!("pn-paste-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("my pic.png");
        std::fs::write(&f, b"x").unwrap();
        let uri = format!("file://{}", f.display().to_string().replace(' ', "%20"));
        let v = parse_paths(&format!("{uri}\n# comment\n/nonexistent.png\n"));
        assert_eq!(v, vec![f.clone()]);
        assert_eq!(parse_paths(&f.display().to_string()), vec![f.clone()]);
        assert!(parse_paths("hello world").is_empty());
        assert_eq!(percent_decode("a%20b%zz%2"), "a b%zz%2");
        std::fs::remove_dir_all(&dir).ok();
    }
}
