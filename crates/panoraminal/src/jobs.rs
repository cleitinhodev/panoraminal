//! Background file jobs (save, export, open, import, autosave) so the UI never
//! freezes on large panoramas.

use panoraminal_io::{ExportFormat, ImportedImage, ProjectSnapshot};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

pub enum JobResult {
    Saved {
        path: PathBuf,
        state: u64,
        then: Option<crate::app::Pending>,
    },
    Exported {
        path: PathBuf,
    },
    OpenedProject {
        path: PathBuf,
        snap: ProjectSnapshot,
    },
    OpenedImage {
        path: PathBuf,
        img: ImportedImage,
    },
    ImportedLayer {
        name: String,
        img: ImportedImage,
    },
    Autosaved {
        state: u64,
    },
    Recovered {
        snap: ProjectSnapshot,
    },
    Failed {
        what: &'static str,
        path: Option<PathBuf>,
        err: String,
    },
}

pub struct Jobs {
    tx: Sender<JobResult>,
    rx: Receiver<JobResult>,
    pub running: usize,
    pub autosave_running: bool,
}

impl Jobs {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            running: 0,
            autosave_running: false,
        }
    }

    fn spawn(
        &mut self,
        ctx: &eframe::egui::Context,
        f: impl FnOnce() -> JobResult + Send + 'static,
    ) {
        self.running += 1;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("panoraminal-job".into())
            .spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(
                    |_| JobResult::Failed {
                        what: "job",
                        path: None,
                        err: "internal error (panic)".into(),
                    },
                );
                let _ = tx.send(r);
                ctx.request_repaint();
            })
            .expect("spawn job thread");
    }

    pub fn poll(&mut self) -> Vec<JobResult> {
        let mut v = Vec::new();
        while let Ok(r) = self.rx.try_recv() {
            self.running = self.running.saturating_sub(1);
            if matches!(r, JobResult::Autosaved { .. })
                || matches!(
                    r,
                    JobResult::Failed {
                        what: "autosave",
                        ..
                    }
                )
            {
                self.autosave_running = false;
            }
            v.push(r);
        }
        v
    }

    pub fn save(
        &mut self,
        ctx: &eframe::egui::Context,
        snap: ProjectSnapshot,
        path: PathBuf,
        state: u64,
        then: Option<crate::app::Pending>,
    ) {
        self.spawn(ctx, move || {
            match panoraminal_io::save_project(&snap, &path) {
                Ok(()) => JobResult::Saved { path, state, then },
                Err(e) => JobResult::Failed {
                    what: "save",
                    path: Some(path),
                    err: e.to_string(),
                },
            }
        });
    }

    pub fn autosave(
        &mut self,
        ctx: &eframe::egui::Context,
        snap: ProjectSnapshot,
        path: PathBuf,
        state: u64,
    ) {
        self.autosave_running = true;
        self.spawn(ctx, move || {
            match panoraminal_io::save_project(&snap, &path) {
                Ok(()) => JobResult::Autosaved { state },
                Err(e) => JobResult::Failed {
                    what: "autosave",
                    path: Some(path),
                    err: e.to_string(),
                },
            }
        });
    }

    pub fn export(
        &mut self,
        ctx: &eframe::egui::Context,
        snap: ProjectSnapshot,
        fmt: ExportFormat,
        path: PathBuf,
    ) {
        self.spawn(ctx, move || {
            let (w, h) = (snap.width, snap.height);
            let bg = match fmt {
                ExportFormat::Png => None,
                ExportFormat::Jpeg { .. } => Some([255, 255, 255]),
            };
            let flat = snap.into_document().flatten(bg);
            match panoraminal_io::encode_flat(&flat, w, h, fmt)
                .and_then(|b| panoraminal_io::write_atomic(&path, &b))
            {
                Ok(()) => JobResult::Exported { path },
                Err(e) => JobResult::Failed {
                    what: "export",
                    path: Some(path),
                    err: e.to_string(),
                },
            }
        });
    }

    pub fn open(&mut self, ctx: &eframe::egui::Context, path: PathBuf, max_dim: u32) {
        self.spawn(ctx, move || {
            if is_project(&path) {
                match panoraminal_io::load_project(&path) {
                    Ok(snap) if snap.width <= max_dim && snap.height <= max_dim => {
                        JobResult::OpenedProject { path, snap }
                    }
                    Ok(_) => JobResult::Failed {
                        what: "open",
                        path: Some(path),
                        err: "too large for this GPU".into(),
                    },
                    Err(e) => JobResult::Failed {
                        what: "open",
                        path: Some(path),
                        err: e.to_string(),
                    },
                }
            } else {
                match panoraminal_io::import_image(&path, max_dim) {
                    Ok(img) => JobResult::OpenedImage { path, img },
                    Err(e) => JobResult::Failed {
                        what: "open",
                        path: Some(path),
                        err: e.to_string(),
                    },
                }
            }
        });
    }

    pub fn recover(&mut self, ctx: &eframe::egui::Context, path: PathBuf) {
        self.spawn(ctx, move || match panoraminal_io::load_project(&path) {
            Ok(snap) => JobResult::Recovered { snap },
            Err(e) => JobResult::Failed {
                what: "recover",
                path: Some(path),
                err: e.to_string(),
            },
        });
    }

    pub fn import_layer(&mut self, ctx: &eframe::egui::Context, path: PathBuf, w: u32, h: u32) {
        self.spawn(ctx, move || {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().chars().take(60).collect())
                .unwrap_or_else(|| "image".into());
            match std::fs::read(&path)
                .map_err(panoraminal_io::IoError::from)
                .and_then(|b| panoraminal_io::import_image_bytes(&b, panoraminal_core::MAX_DIM))
            {
                Ok(img) => {
                    // Fit the image to the document size.
                    let img = if (img.width, img.height) != (w, h) {
                        let src = image::RgbaImage::from_raw(img.width, img.height, img.pixels)
                            .expect("valid buffer");
                        let r = image::imageops::resize(
                            &src,
                            w,
                            h,
                            image::imageops::FilterType::Lanczos3,
                        );
                        ImportedImage {
                            width: w,
                            height: h,
                            pixels: r.into_raw(),
                            downscaled: true,
                            had_gpano: img.had_gpano,
                        }
                    } else {
                        img
                    };
                    JobResult::ImportedLayer { name, img }
                }
                Err(e) => JobResult::Failed {
                    what: "import",
                    path: Some(path),
                    err: e.to_string(),
                },
            }
        });
    }
}

pub fn is_project(p: &std::path::Path) -> bool {
    let n = p
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    n.ends_with(".pano") || n.ends_with(".p360.json") || n.ends_with(".json")
}

pub fn is_supported(p: &std::path::Path) -> bool {
    let n = p
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    is_project(p)
        || [".png", ".jpg", ".jpeg", ".webp", ".bmp", ".tif", ".tiff"]
            .iter()
            .any(|e| n.ends_with(e))
}
