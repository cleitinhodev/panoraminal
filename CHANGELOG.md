# Changelog

All notable changes to Panoraminal are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [1.0.0] — 2026-10-29

First release of **Panoraminal**, the successor of the Paint360 prototype, rewritten from scratch as a native application.

### Added
- Native engine in Rust: GPU rendering with **wgpu** (Vulkan / Metal / DirectX 12 / OpenGL) and an **egui** interface.
- **Sphere view** (paint inside the 360° panorama with a perspective camera) and **flat view** (equirectangular with wrap-around preview), toggle with `V`.
- Brush engine that paints in sphere space: pen, brush, airbrush and eraser with size, opacity, flow, hardness, spacing, smoothing, polar correction and **pen pressure** (size and/or opacity).
- **Eyedropper** (also `Alt`+click) and a 360°-aware **flood fill** that wraps around the seam.
- Straight lines with `Shift`+click (drawn as great circles on the sphere).
- **Layers** (up to 64): add, duplicate, delete, merge down, reorder by drag & drop, rename, visibility, opacity, alpha lock and 9 blend modes, with live thumbnails.
- Undo / redo with a memory budget, coalesced slider changes and per-action toasts.
- **`.pano` project format** (ZIP + PNG layers + JSON manifest); legacy Paint360 `.p360.json` projects are imported.
- Import PNG / JPEG / WebP / BMP / TIFF as a document or as a new layer (drag & drop supported).
- Export **PNG** and **360° JPEG with GPano XMP metadata** (recognised by Facebook, Google Photos and VR viewers).
- Rotate heading (horizontal offset) and horizontal flip of the whole panorama.
- **Autosave & crash recovery**, unsaved-changes prompt on close / new / open.
- Brand look derived from the app icon: **dark, light and system themes**, gradients, neon glow and animations — all of which can be reduced or **turned off** (respects the system "reduce motion" setting).
- **Help** window (shortcuts, tools, 360° tips) and **About** window with links to the repository, website and donations (opened in the system browser, https only).
- **8 languages**: Português (Brasil), English, Español, Français, Deutsch, Italiano, 日本語 and 简体中文, auto-detected from the system and switchable at runtime. CJK glyphs are bundled.
- Persistent settings (language, theme, animations, glow, UI scale, autosave interval, undo memory, zoom speed, pen pressure, export defaults…).
- Native file dialogs, Windows executable icon, window icon, `.desktop` file and MIME type for `.pano`.
- Packages: Windows installer (NSIS) and portable zip; Linux AppImage, `.deb` and `.tar.gz`; macOS app bundle (built by CI).
- CI (tests, clippy, rustfmt) and tagged release workflows on GitHub Actions.

[1.0.0]: https://github.com/cleitinhodev/panoraminal/releases/tag/v1.0.0
