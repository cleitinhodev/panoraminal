//! Panoraminal — paint 360° equirectangular panoramas directly on the sphere.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod cli;
mod demo;
mod fonts;
mod icons;
mod jobs;
mod settings;
mod shortcuts;
mod theme;
mod ui;

use eframe::egui;

pub const APP_NAME: &str = "Panoraminal";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO_URL: &str = "https://github.com/cleitinhodev/panoraminal";
pub const SITE_URL: &str = "https://www.bugzinho.com/";
pub const DONATE_URL: &str = "https://www.bugzinho.com/donate";

fn main() -> eframe::Result {
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("warn,wgpu_core=error,wgpu_hal=error,naga=error"),
    )
    .init();
    let cli = match cli::Cli::parse(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(if msg.starts_with("Panoraminal") { 0 } else { 2 });
        }
    };

    let favicon = icons::favicon_icon_data();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_app_id("panoraminal")
        .with_inner_size(cli.window_size.unwrap_or([1440.0, 900.0]))
        .with_min_inner_size([900.0, 600.0])
        .with_drag_and_drop(true)
        .with_icon(favicon);
    if cli.screenshot.is_some() {
        viewport = viewport.with_resizable(false);
    }

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        centered: true,
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(app::PanoApp::new(cc, cli)))),
    )
}
