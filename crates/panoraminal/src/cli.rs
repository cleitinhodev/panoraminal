//! Command line options.

use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct Cli {
    /// File to open at startup.
    pub file: Option<PathBuf>,
    /// Save a screenshot of the window to this path and quit (testing / docs).
    pub screenshot: Option<PathBuf>,
    pub theme: Option<String>,
    pub lang: Option<String>,
    /// Open a window at startup: help, about, settings, new, export.
    pub window: Option<String>,
    pub demo: bool,
    pub frames: u32,
    pub window_size: Option<[f32; 2]>,
    pub view: Option<String>,
    pub no_splash: bool,
    pub help_tab: Option<usize>,
    /// Initial tool id (pen, brush, airbrush, custom, eraser).
    pub tool: Option<String>,
    /// Float a generated demo image: wrap or gnomonic.
    pub place_demo: Option<String>,
}

const USAGE: &str = "Panoraminal — paint 360° panoramas

USAGE:
    panoraminal [OPTIONS] [FILE]

FILE may be a .pano project, a legacy .p360.json project or an image (PNG, JPEG, WebP, BMP, TIFF).

OPTIONS:
    --theme <dark|light|system>   Override the theme for this session
    --lang <code>                 Override the language (pt-BR, en, es, fr, de, it, ja, zh-CN)
    --demo                        Start with a procedurally generated demo panorama
    --view <sphere|flat>          Initial view mode
    --no-splash                   Skip the startup animation
    --screenshot <file.png>       Render, save a screenshot and exit (for tests/docs)
    --window <help|about|settings|new|export>  Open a window at startup
    --frames <n>                  Frames to render before the screenshot (default 30)
    --size <WxH>                  Initial window size in points
    --tool <id>                   Initial tool (pen, brush, airbrush, custom, eraser)
    --place-demo <wrap|gnomonic|wall|wall-warp|wall-merged>
                                  Float a generated picture for placing (docs/tests);
                                  wall* = corner-pinned onto a synthetic room
    -h, --help                    Show this help
    -V, --version                 Show the version";

impl Cli {
    pub fn parse(args: impl Iterator<Item = String>) -> Result<Cli, String> {
        let mut c = Cli {
            frames: 30,
            ..Default::default()
        };
        let mut it = args.peekable();
        while let Some(a) = it.next() {
            let mut val = |name: &str| {
                it.next()
                    .ok_or_else(|| format!("missing value for {name}\n\n{USAGE}"))
            };
            match a.as_str() {
                "-h" | "--help" => {
                    return Err(USAGE.replacen(
                        "Panoraminal —",
                        &format!("Panoraminal {} —", crate::VERSION),
                        1,
                    ))
                }
                "-V" | "--version" => return Err(format!("Panoraminal {}", crate::VERSION)),
                "--theme" => c.theme = Some(val("--theme")?),
                "--lang" => c.lang = Some(val("--lang")?),
                "--window" => c.window = Some(val("--window")?),
                "--view" => c.view = Some(val("--view")?),
                "--screenshot" => c.screenshot = Some(PathBuf::from(val("--screenshot")?)),
                "--frames" => {
                    c.frames = val("--frames")?
                        .parse()
                        .map_err(|_| "invalid --frames".to_string())?
                }
                "--help-tab" => c.help_tab = val("--help-tab")?.parse().ok(),
                "--size" => {
                    let v = val("--size")?;
                    let (w, h) = v.split_once('x').ok_or("invalid --size, expected WxH")?;
                    c.window_size = Some([
                        w.parse().map_err(|_| "invalid --size")?,
                        h.parse().map_err(|_| "invalid --size")?,
                    ]);
                }
                "--tool" => c.tool = Some(val("--tool")?),
                "--place-demo" => c.place_demo = Some(val("--place-demo")?),
                "--demo" => c.demo = true,
                "--no-splash" => c.no_splash = true,
                s if s.starts_with('-') && s.len() > 1 && !std::path::Path::new(s).exists() => {
                    return Err(format!("unknown option {s}\n\n{USAGE}"))
                }
                _ => c.file = Some(PathBuf::from(a)),
            }
        }
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses() {
        let c = Cli::parse(
            ["--theme", "dark", "--size", "800x600", "x.pano"]
                .iter()
                .map(|s| s.to_string()),
        )
        .unwrap();
        assert_eq!(c.theme.as_deref(), Some("dark"));
        assert_eq!(c.window_size, Some([800.0, 600.0]));
        assert_eq!(c.file, Some(PathBuf::from("x.pano")));
        assert!(Cli::parse(["--bogus"].iter().map(|s| s.to_string())).is_err());
    }
}
