#!/usr/bin/env bash
# Build eXorchy from this checkout and install it for the current user
# (no root): binary under ~/.local/bin, resources under ~/.local/lib/exorchy
# (exorchy_core::resource_dir() looks there after /usr/lib/exorchy), desktop
# entry and icons under ~/.local/share. `packaging/PKGBUILD` is the
# system-wide (pacman) route.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"

cd "$ROOT"
cargo build --release -p exorchy

BIN="$HOME/.local/bin"
LIB="$HOME/.local/lib/exorchy"
APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor"
mkdir -p "$BIN" "$LIB" "$APPS"

install -Dm755 target/release/exorchy "$BIN/exorchy"

rm -rf "$LIB/metadata" "$LIB/torrents" "$LIB/previews"
cp -r metadata "$LIB/metadata"
rm -f "$LIB/metadata/"*.db "$LIB/metadata/"*.db-shm "$LIB/metadata/"*.db-wal
cp -r torrents "$LIB/torrents"
cp -r crates/core/resources/previews "$LIB/previews"
install -Dm644 manifest.json "$LIB/manifest.json"

# The desktop file is named after the GApplication id so the launcher matches
# the running window; drop the entry an older install left behind.
rm -f "$APPS/exorchy.desktop"
install -Dm644 packaging/org.exorchy.eXorchy.desktop "$APPS/org.exorchy.eXorchy.desktop"
for s in 32 128 256 512; do
  install -Dm644 "packaging/icons/${s}x${s}.png" "$ICONS/${s}x${s}/apps/exorchy.png"
done
# The icon is raster only now; a scalable one left by an older install would win.
rm -f "$ICONS/scalable/apps/exorchy.svg"
update-desktop-database "$APPS" 2>/dev/null || true
gtk4-update-icon-cache -q "$ICONS" 2>/dev/null || gtk-update-icon-cache -q "$ICONS" 2>/dev/null || true

echo "Installed: $BIN/exorchy (resources in $LIB)"
echo "Launch with 'exorchy' or from the Omarchy launcher (Super+Space)."
