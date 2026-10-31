# Changelog

All notable changes to Panoraminal are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [1.2.0] — 2026-10-31

### Added
- **Distort mode for placed images** — press `D` while an image is floating (or click *Distort* in the panel) and drag its **four corner pins** to fit it onto a wall, a screen, a floor or any flat surface in the panorama. The pins solve a **homography**, so the image gets a real perspective and looks like part of the scene.
  - **Persistent** projection: the homography lives in the projector plane — straight lines stay straight on the sphere, so a picture pinned to the corners of a wall follows the wall exactly.
  - **Wrap** projection: the homography is applied in the patch's local angular (tangent) coordinates (an approximation for large patches).
  - **Mesh warp** (`W`, or *Warp* in the panel): a 4×4 grid of points on top of the perspective for gently curved surfaces; *Reset mesh*.
  - `Shift` locks the drag to one axis; edge handles move two pins at once (`Shift`: perpendicular to the edge); dragging inside moves the image; *Pin coordinates* shows and edits the four pins numerically; *Reset distortion*.
  - Folded (concave/self-intersecting) and collapsed shapes are refused: the last valid shape is kept and a warning is shown.
  - Handles in the sphere and the flat view, across the seam; distortion is part of the placement's local undo/redo and is kept when moving, scaling, rotating, flipping or switching projection.
  - Live GPU preview with a per-pixel inverse mapping and mipmapped sampling; the merge bakes with anisotropic, mip-filtered sampling (bicubic when magnified).
- Help: distort mode and a new 360° tip; all new strings in the 8 languages.
- Command line: `--place-demo wall | wall-warp | wall-merged` (a picture pinned onto a synthetic room, used for the screenshots).

### Fixed
- The canvas could keep a stale strip of a floating image after its preview texture changed (e.g. undoing a distortion).
- A flat view set before the first frame is no longer reset by the initial fit.

## [1.1.0] — 2026-10-30

### Added
- **Custom brush tool** (`C`) with a **brush library** panel: thumbnails, live preview stroke in the current colour, rename, categories, favourites (shown first), search and delete. Nine procedural built-in brushes (soft round, chalk, calligraphy, splatter, stars, grass, clouds, hatching, leaves).
- **Brush import** (File → Import brushes…, `Ctrl+Shift+B`, or drop files on the window):
  - **Photoshop `.abr`** v1/v2 (computed and sampled tips, PackBits) and v6+ (`samp` tips and `desc` presets: diameter, angle, roundness, spacing, hardness, flips, shape dynamics, scattering, count, transfer, pen-pressure control).
  - **GIMP `.gbr`** (grayscale and colour) and **`.gih`** image pipes (every cell becomes a tip, cycled or random).
  - **PNG / JPEG / WebP** images (alpha or darkness used as the mask).
  - Unsupported Photoshop features (dual brush, texture, bristle/erodible tips, colour dynamics, wet edges, noise) are approximated or skipped and listed in a notice.
- Brush **dynamics**: spacing, angle, roundness, size/angle/roundness jitter with minimum diameter, scatter (one or both axes) and count, opacity/flow jitter, random flips, follow stroke direction, pen pressure. Stamps are placed with sphere-correct orientation, so they stay consistent across the seam and at the poles.
- The library is saved in the user data directory; brushes used in a project are **embedded in the `.pano` file** (Settings → Editor) and added to the library when the project is opened elsewhere.
- **Place images in the scene**: File → Insert image… (`Ctrl+Shift+I`), drag & drop onto a document, or paste (`Ctrl+V` / `Ctrl+Shift+V`, image data or copied file paths).
  - The image floats as a **non-destructive object** with on-canvas handles: move, uniform or free (`Shift`) scale, edge scale, rotate (`Shift` snaps to 15°), plus opacity, rotation, size and flip controls — in the sphere and the flat view.
  - **Wrap** projection (a spherical patch that keeps its angular size) and **Persistent** projection (a gnomonic projector anchored in space, undistorted from its direction; *Re-aim to view* moves it to the current view).
  - Live GPU preview through the compositor, correct across the seam and at the poles; full-quality, mip-filtered bake on **Merge** (`Enter`) or **Place as new layer**; `Esc` cancels; edits while floating have their own undo/redo, the merge is one undo step.
  - Dropping a 2:1 image on an empty document (or holding `Shift`) still opens it as a panorama.
- Help: custom brush tool, image placement and two new 360° tips. All new strings in the 8 languages.
- Command line: `--tool <id>` and `--place-demo <wrap|gnomonic>` (used for screenshots).

### Changed
- Slider values show at most one decimal.
- The release workflow publishes a single `SHA256SUMS.txt` covering every asset of the release (hand-uploaded and CI-built).

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
- Graphics backend probe at startup: if the preferred backend (e.g. DirectX 12) cannot create a device, Panoraminal falls back to Vulkan / OpenGL instead of exiting. `WGPU_BACKEND` still overrides it.
- Rotate heading (horizontal offset) and horizontal flip of the whole panorama.
- **Autosave & crash recovery**, unsaved-changes prompt on close / new / open.
- Brand look derived from the app icon: **dark, light and system themes**, gradients, neon glow and animations — all of which can be reduced or **turned off** (respects the system "reduce motion" setting).
- **Help** window (shortcuts, tools, 360° tips) and **About** window with links to the repository, website and donations (opened in the system browser, https only).
- **8 languages**: Português (Brasil), English, Español, Français, Deutsch, Italiano, 日本語 and 简体中文, auto-detected from the system and switchable at runtime. CJK glyphs are bundled.
- Persistent settings (language, theme, animations, glow, UI scale, autosave interval, undo memory, zoom speed, pen pressure, export defaults…).
- Native file dialogs, Windows executable icon, window icon, `.desktop` file and MIME type for `.pano`.
- Packages: Windows installer (NSIS) and portable zip; Linux AppImage, `.deb` and `.tar.gz`; macOS app bundle (built by CI).
- CI (tests, clippy, rustfmt) and tagged release workflows on GitHub Actions.

[1.2.0]: https://github.com/cleitinhodev/panoraminal/releases/tag/v1.2.0
[1.1.0]: https://github.com/cleitinhodev/panoraminal/releases/tag/v1.1.0
[1.0.0]: https://github.com/cleitinhodev/panoraminal/releases/tag/v1.0.0
