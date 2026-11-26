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
    /// Generation of the brush styles the saved brushes were made with:
    /// 0 = Panoraminal 1.0–1.2, 1 = 1.3 (oil brush). Missing in old files.
    #[serde(default)]
    pub brush_styles_rev: u32,
    /// Styles saved by the user.
    pub user_styles: Vec<UserStyle>,
    /// Options of the line tools (1.4; missing in older files → defaults).
    pub lines: crate::linetools::LineSettings,
}

/// A brush style saved by the user for one tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserStyle {
    /// Tool id (`pen`, `brush`, `airbrush`, `eraser`, `custom`).
    pub tool: String,
    pub name: String,
    pub settings: BrushSettings,
}

/// Current generation of the brush styles (see [`Settings::brush_styles_rev`]).
pub const BRUSH_STYLES_REV: u32 = 1;

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
            brush_styles_rev: BRUSH_STYLES_REV,
            user_styles: Vec::new(),
            lines: Default::default(),
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

    pub(crate) fn sanitized(mut self) -> Self {
        self.ui_scale = if self.ui_scale.is_finite() {
            self.ui_scale.clamp(0.75, 2.0)
        } else {
            1.0
        };
        self.jpeg_quality = self.jpeg_quality.clamp(10, 100);
        self.lines.clamp();
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
        if self.brush_styles_rev < 1 {
            // 1.3: the brush becomes an oil brush. Keep the user's size, opacity,
            // smoothing and pressure options; the classic brush is still a style.
            if let Some(b) = self.brushes.get_mut(ToolKind::Brush.id()) {
                *b = b.with_style_of(&BrushSettings::preset(ToolKind::Brush));
            }
        }
        self.brush_styles_rev = self.brush_styles_rev.max(BRUSH_STYLES_REV);
        self.user_styles
            .retain(|u| ToolKind::ALL.iter().any(|t| t.id() == u.tool));
        self.user_styles.truncate(200);
        for u in &mut self.user_styles {
            u.settings.clamp();
            u.name = u.name.chars().take(40).collect();
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

#[cfg(test)]
mod style_tests {
    use super::*;

    #[test]
    fn settings_from_1_2_upgrade_the_brush_only() {
        let json = r#"{"theme":"Dark","brushes":{
            "brush":{"size":77.0,"opacity":0.5,"flow":1.0,"hardness":0.6,"spacing":0.12,
                     "smoothing":0.4,"pressure_size":true,"pressure_opacity":false,
                     "min_size":0.08,"polar_correction":true},
            "airbrush":{"size":140.0,"opacity":1.0,"flow":0.08,"hardness":0.0,"spacing":0.06,
                     "smoothing":0.2,"pressure_size":false,"pressure_opacity":true,
                     "min_size":0.08,"polar_correction":true}}}"#;
        let s: Settings = serde_json::from_str::<Settings>(json).unwrap().sanitized();
        assert_eq!(s.brush_styles_rev, BRUSH_STYLES_REV);
        assert_eq!(s.lines, crate::linetools::LineSettings::default());
        let b = s.brush(ToolKind::Brush);
        assert!(b.is_wet(ToolKind::Brush), "the brush becomes an oil brush");
        assert_eq!(
            (b.size, b.opacity, b.smoothing),
            (77.0, 0.5, 0.4),
            "user values kept"
        );
        assert_eq!(
            s.brush(ToolKind::Airbrush),
            BrushSettings::classic(ToolKind::Airbrush),
            "the airbrush is unchanged"
        );
        // once upgraded, the brush is never touched again
        let again: Settings = serde_json::from_str::<Settings>(&serde_json::to_string(&s).unwrap())
            .unwrap()
            .sanitized();
        assert_eq!(again.brush(ToolKind::Brush), b);
        let mut classic = s.clone();
        classic
            .brushes
            .insert("brush".into(), BrushSettings::classic(ToolKind::Brush));
        let c: Settings =
            serde_json::from_str::<Settings>(&serde_json::to_string(&classic).unwrap())
                .unwrap()
                .sanitized();
        assert_eq!(
            c.brush(ToolKind::Brush),
            BrushSettings::classic(ToolKind::Brush)
        );
    }

    #[test]
    fn user_styles_round_trip() {
        let mut s = Settings::default();
        s.user_styles.push(UserStyle {
            tool: "airbrush".into(),
            name: "My spray".into(),
            settings: BrushSettings {
                particles: 0.7,
                ..BrushSettings::classic(ToolKind::Airbrush)
            },
        });
        s.user_styles.push(UserStyle {
            tool: "bogus".into(),
            name: "x".into(),
            settings: BrushSettings::default(),
        });
        let back = serde_json::from_str::<Settings>(&serde_json::to_string(&s).unwrap())
            .unwrap()
            .sanitized();
        assert_eq!(back.user_styles.len(), 1);
        assert_eq!(back.user_styles[0], s.user_styles[0]);
    }
}
