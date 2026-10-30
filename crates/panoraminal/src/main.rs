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

    let mut options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        centered: true,
        persist_window: false,
        ..Default::default()
    };
    if let Some(b) = probe_backends() {
        if let egui_wgpu::WgpuSetup::CreateNew(ref mut c) = options.wgpu_options.wgpu_setup {
            c.instance_descriptor.backends = b;
        }
    }
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(app::PanoApp::new(cc, cli)))),
    )
}

/// Finds a graphics backend that can actually create a device.
///
/// wgpu picks the "best" backend first (DX12 on Windows), but some drivers,
/// remote sessions and compatibility layers expose an adapter whose device
/// creation then fails, which would make the app exit at startup. We try the
/// backends one by one and hand eframe the first one that works. An explicit
/// `WGPU_BACKEND` environment variable always wins.
fn probe_backends() -> Option<egui_wgpu::wgpu::Backends> {
    use egui_wgpu::wgpu;
    if std::env::var_os("WGPU_BACKEND").is_some() {
        return None;
    }
    let candidates: &[wgpu::Backends] = if cfg!(windows) {
        &[
            wgpu::Backends::DX12,
            wgpu::Backends::VULKAN,
            wgpu::Backends::GL,
        ]
    } else if cfg!(target_os = "macos") {
        &[wgpu::Backends::METAL]
    } else {
        &[wgpu::Backends::VULKAN, wgpu::Backends::GL]
    };
    if candidates.len() < 2 {
        return None;
    }
    for &b in candidates {
        let mut defaults = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
        defaults.instance_descriptor.backends = b;
        let instance = wgpu::Instance::new(defaults.instance_descriptor);
        let ok = pollster::block_on(async {
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: defaults.power_preference,
                    ..Default::default()
                })
                .await
                .ok()?;
            let dd = (defaults.device_descriptor)(&adapter);
            adapter.request_device(&dd).await.ok().map(|_| ())
        })
        .is_some();
        if ok {
            log::info!("using graphics backend {b:?}");
            return Some(b);
        }
        log::warn!("graphics backend {b:?} unavailable, trying the next one");
    }
    None
}
