# Contributing to Panoraminal

Thanks for wanting to help! Bug reports, translations, ideas and pull requests are all welcome.

## Building

You need a recent stable [Rust toolchain](https://rustup.rs/).

```bash
git clone https://github.com/cleitinhodev/panoraminal
cd panoraminal
cargo run --release            # start the app
cargo run --release -- --demo  # start with the demo panorama
```

On Linux you also need the GTK 3 and xkbcommon development packages for the native file dialogs and keyboard handling:

```bash
sudo apt install libgtk-3-dev libxkbcommon-dev
```

## Before opening a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs the same three commands on Linux, Windows and macOS.

## Project layout

| Crate | What it does |
|---|---|
| `crates/panoraminal-core` | Document, layers, blend modes, brush engine, flood fill, history, sphere math. No GPU or UI code. |
| `crates/panoraminal-render` | wgpu compositor and sphere / flat display shaders (WGSL). |
| `crates/panoraminal-io` | `.pano` projects, legacy Paint360 import, PNG/JPEG encoding, GPano XMP. |
| `crates/panoraminal-i18n` | Translation catalogs (`locales/*.json`) and lookup. |
| `crates/panoraminal` | The application: egui UI, themes, shortcuts, settings, dialogs. |

## Translations

Every language lives in `crates/panoraminal-i18n/locales/<code>.json` and must contain **exactly the same keys** as `en.json`, with the same `{placeholders}`. `cargo test -p panoraminal-i18n` checks this.

If you add or change text that uses Japanese or Chinese characters, regenerate the bundled font subset:

```bash
pip install fonttools
python3 scripts/subset-cjk-font.py /path/to/NotoSansCJK-Regular.ttc
```

## Screenshots

The app can render itself headlessly, which is how the README screenshots are made:

```bash
panoraminal --demo --no-splash --theme dark --window help --screenshot shot.png
```

## Code style

- Keep the core crate free of UI/GPU dependencies so it stays easy to test.
- Every user-visible string goes through `tr("key")`.
- Respect the user's animation and glow settings (`app.anim`, `pal.glow`) in any new effect.

By contributing you agree that your contributions are licensed under the MIT License.
