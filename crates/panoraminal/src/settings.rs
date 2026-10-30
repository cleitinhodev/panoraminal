//! User settings, persisted as JSON in the platform config directory.

use crate::theme::Glow;
use panoraminal_core::document::Background;
use panoraminal_core::fill::FillOptions;
use panoraminal_core::place::Projection;
use panoraminal_core::{BrushSettings, ToolKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeChoice {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AnimChoice {
    /// Follow the OS "reduce motion" preference.
    #[default]
    Auto,
    On,
    Off,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    /// Language code (`None` = follow the system).
    pub language: Option<String>,
    pub animations: AnimChoice,
    pub glow: Glow,
    pub ui_scale: f32,
    pub autosave_minutes: u32,
    pub recent: Vec<PathBuf>,
    pub last_dir: Option<PathBuf>,
    pub show_grid: bool,
    pub jpeg_quality: u8,
    pub jpeg_gpano: bool,
    pub new_width: u32,
    pub new_height: u32,
    pub new_background: Background,
    pub brushes: BTreeMap<String, BrushSettings>,
    pub fill: FillOptions,
    pub swatches: Vec<[u8; 3]>,
    pub primary: [u8; 3],
    pub secondary: [u8; 3],
    pub pen_pressure: bool,
    pub undo_memory_mb: u32,
    pub show_splash: bool,
    pub wheel_zoom_speed: f32,
    pub invert_drag: bool,
    pub confirm_on_close: bool,
    /// Selected custom brush id.
    pub custom_brush: Option<String>,
    /// Embed the custom brushes used in a document when saving `.pano` files.
    pub embed_brushes: bool,
    /// Last projection used when placing an image.
    pub place_projection: Projection,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            language: None,
            animations: AnimChoice::Auto,
            glow: Glow::Full,
            ui_scale: 1.0,
            autosave_minutes: 2,
            recent: Vec::new(),
            last_dir: None,
            show_grid: false,
            jpeg_quality: 92,
            jpeg_gpano: true,
            new_width: 4096,
            new_height: 2048,
            new_background: Background::Sky,
            brushes: BTreeMap::new(),
            fill: FillOptions::default(),
            swatches: Vec::new(),
            primary: [0x1b, 0x99, 0xf9],
            secondary: [0xff, 0xff, 0xff],
            pen_pressure: true,
            undo_memory_mb: 1024,
            show_splash: true,
            wheel_zoom_speed: 1.0,
            invert_drag: false,
            confirm_on_close: true,
            custom_brush: None,
            embed_brushes: true,
            place_projection: Projection::Gnomonic,
        }
    }
}

pub fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("com", "bugzinho", "Panoraminal")
}

pub fn settings_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PANORAMINAL_CONFIG_DIR") {
        return Some(PathBuf::from(p).join("settings.json"));
    }
    project_dirs().map(|d| d.config_dir().join("settings.json"))
}

pub fn data_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PANORAMINAL_CONFIG_DIR") {
        return Some(PathBuf::from(p));
    }
    project_dirs().map(|d| d.data_dir().to_path_buf())
}

pub fn recovery_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("recovery").join("autosave.pano"))
}

impl Settings {
    pub fn load() -> Self {
        let Some(p) = settings_path() else {
            return Self::default();
        };
        match std::fs::read(&p) {
            Ok(b) => serde_json::from_slice::<Settings>(&b)
                .map(|s| s.sanitized())
                .unwrap_or_else(|e| {
                    log::warn!("ignoring invalid settings file {}: {e}", p.display());
                    Self::default()
                }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let Some(p) = settings_path() else { return };
        match serde_json::to_vec_pretty(self) {
            Ok(b) => {
                if let Err(e) = panoraminal_io::write_atomic(&p, &b) {
                    log::warn!("could not save settings: {e}");
                }
            }
            Err(e) => log::warn!("could not serialize settings: {e}"),
        }
    }

    fn sanitized(mut self) -> Self {
        self.ui_scale = if self.ui_scale.is_finite() {
            self.ui_scale.clamp(0.75, 2.0)
        } else {
            1.0
        };
        self.jpeg_quality = self.jpeg_quality.clamp(10, 100);
        self.new_width = self.new_width.clamp(256, panoraminal_core::MAX_DIM);
        self.new_height = self.new_height.clamp(128, panoraminal_core::MAX_DIM);
        self.autosave_minutes = self.autosave_minutes.min(60);
        self.undo_memory_mb = self.undo_memory_mb.clamp(128, 16384);
        self.wheel_zoom_speed = if self.wheel_zoom_speed.is_finite() {
            self.wheel_zoom_speed.clamp(0.2, 4.0)
        } else {
            1.0
        };
        self.recent.truncate(10);
        self.swatches.truncate(24);
        for b in self.brushes.values_mut() {
            b.clamp();
        }
        self
    }

    pub fn brush(&self, tool: ToolKind) -> BrushSettings {
        self.brushes
            .get(tool.id())
            .copied()
            .unwrap_or_else(|| BrushSettings::preset(tool))
    }

    pub fn push_recent(&mut self, p: &Path) {
        let p = p.to_path_buf();
        self.recent.retain(|x| x != &p);
        self.recent.insert(0, p);
        self.recent.truncate(10);
    }

    pub fn push_swatch(&mut self, c: [u8; 3]) {
        self.swatches.retain(|x| *x != c);
        self.swatches.insert(0, c);
        self.swatches.truncate(16);
    }
}

/// Best-effort detection of the OS "reduce motion" accessibility preference.
pub fn os_prefers_reduced_motion() -> bool {
    #[cfg(target_os = "linux")]
    {
        let out = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "enable-animations"])
            .output();
        if let Ok(o) = out {
            if String::from_utf8_lossy(&o.stdout).trim() == "false" {
                return true;
            }
        }
        false
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("defaults")
            .args(["read", "com.apple.universalaccess", "reduceMotion"])
            .output();
        matches!(out, Ok(o) if String::from_utf8_lossy(&o.stdout).trim() == "1")
    }
    #[cfg(target_os = "windows")]
    {
        windows_animations_disabled()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        false
    }
}

#[cfg(target_os = "windows")]
fn windows_animations_disabled() -> bool {
    // SPI_GETCLIENTAREAANIMATION = 0x1042
    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(
            action: u32,
            param: u32,
            pv: *mut core::ffi::c_void,
            win_ini: u32,
        ) -> i32;
    }
    let mut enabled: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes a BOOL into the provided pointer.
    let ok = unsafe { SystemParametersInfoW(0x1042, 0, &mut enabled as *mut i32 as *mut _, 0) };
    ok != 0 && enabled == 0
}
