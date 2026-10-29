#!/usr/bin/env bash
# Builds Linux release packages: tar.gz, .deb and AppImage.
# Usage: packaging/linux/build-packages.sh [outdir]
# Needs: cargo, dpkg-deb, and appimagetool (APPIMAGETOOL env or on PATH).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${1:-$ROOT/dist}"
mkdir -p "$OUT"
VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
ARCH=x86_64
BIN="$ROOT/target/release/panoraminal"
ICONS="$ROOT/crates/panoraminal/assets/icons"
DESKTOP="$ROOT/packaging/linux/io.github.cleitinhodev.Panoraminal.desktop"
[ -x "$BIN" ] || (cd "$ROOT" && cargo build --release --locked)
strip "$BIN" 2>/dev/null || true
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

install_tree() { # $1 = prefix root
  local P="$1"
  install -Dm755 "$BIN" "$P/usr/bin/panoraminal"
  install -Dm644 "$DESKTOP" "$P/usr/share/applications/io.github.cleitinhodev.Panoraminal.desktop"
  install -Dm644 "$ROOT/packaging/linux/panoraminal-mime.xml" "$P/usr/share/mime/packages/panoraminal.xml"
  for s in 32 128 256; do
    install -Dm644 "$ICONS/panoraminal_icon_$s.png" "$P/usr/share/icons/hicolor/${s}x${s}/apps/panoraminal.png"
  done
  install -Dm644 "$ROOT/LICENSE" "$P/usr/share/doc/panoraminal/LICENSE"
  install -Dm644 "$ROOT/crates/panoraminal/assets/fonts/OFL.txt" "$P/usr/share/doc/panoraminal/OFL-NotoSansCJK.txt"
}

# tar.gz
T="$WORK/panoraminal-$VERSION-linux-$ARCH"
mkdir -p "$T"
cp "$BIN" "$T/panoraminal"
cp "$ROOT/LICENSE" "$ROOT/README.md" "$DESKTOP" "$T/"
cp "$ICONS/panoraminal_icon_256.png" "$T/panoraminal.png"
tar -C "$WORK" -czf "$OUT/panoraminal-$VERSION-linux-$ARCH.tar.gz" "$(basename "$T")"

# .deb
D="$WORK/deb"
install_tree "$D"
mkdir -p "$D/DEBIAN"
SIZE=$(du -sk "$D/usr" | cut -f1)
cat > "$D/DEBIAN/control" <<CTRL
Package: panoraminal
Version: $VERSION
Section: graphics
Priority: optional
Architecture: amd64
Maintainer: cleitinhodev / bugzinho <https://www.bugzinho.com/>
Installed-Size: $SIZE
Depends: libc6 (>= 2.31), libgtk-3-0, libxkbcommon0, libvulkan1 | libgl1
Homepage: https://github.com/cleitinhodev/panoraminal
Description: Paint 360° equirectangular panoramas directly on the sphere
 Panoraminal is a GPU accelerated painting program for 360° panoramas,
 with layers, blend modes, pen pressure, a 360°-aware flood fill and
 JPEG export with GPano metadata for 360° viewers.
CTRL
dpkg-deb --root-owner-group --build "$D" "$OUT/panoraminal_${VERSION}_amd64.deb" >/dev/null

# AppImage
A="$WORK/Panoraminal.AppDir"
install_tree "$A"
cp "$DESKTOP" "$A/panoraminal.desktop"
cp "$ICONS/panoraminal_icon_256.png" "$A/panoraminal.png"
cp "$ICONS/panoraminal_icon_256.png" "$A/.DirIcon"
cat > "$A/AppRun" <<'RUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/panoraminal" "$@"
RUN
chmod +x "$A/AppRun"
TOOL="${APPIMAGETOOL:-appimagetool}"
RT=()
[ -n "${APPIMAGE_RUNTIME:-}" ] && RT=(--runtime-file "$APPIMAGE_RUNTIME")
ARCH=$ARCH "$TOOL" --appimage-extract-and-run "${RT[@]}" -n "$A" "$OUT/Panoraminal-$VERSION-$ARCH.AppImage" >/dev/null 2>&1 \
  || ARCH=$ARCH "$TOOL" "${RT[@]}" -n "$A" "$OUT/Panoraminal-$VERSION-$ARCH.AppImage"
ls -la "$OUT"
