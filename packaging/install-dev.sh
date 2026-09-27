#!/usr/bin/env bash
# Build Exorchy from this checkout and install it for the current user
# (no root): binary + resources under ~/.local, desktop entry and icons under
# ~/.local/share. `packaging/PKGBUILD` is the system-wide (pacman) route.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"

cd "$ROOT"
pnpm install --frozen-lockfile
pnpm build
(cd src-tauri && cargo build --release)

BIN="$HOME/.local/bin"
LIB="$HOME/.local/lib/exorchy"
mkdir -p "$BIN" "$LIB" "$HOME/.local/share/applications" "$HOME/.local/share/icons/hicolor/scalable/apps"
install -Dm755 src-tauri/target/release/exorchy "$BIN/exorchy"
rm -rf "$LIB/metadata" "$LIB/torrents" "$LIB/previews"
cp -r metadata "$LIB/metadata"
cp -r torrents "$LIB/torrents"
cp -r src-tauri/resources/previews "$LIB/previews"
install -Dm644 manifest.json "$LIB/manifest.json"
install -Dm644 packaging/exorchy.desktop "$HOME/.local/share/applications/exorchy.desktop"
for s in 32 128 256 512; do
  install -Dm644 "src-tauri/icons/${s}x${s}.png" "$HOME/.local/share/icons/hicolor/${s}x${s}/apps/exorchy.png"
done
install -Dm644 src/assets/exorchy.svg "$HOME/.local/share/icons/hicolor/scalable/apps/exorchy.svg"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
echo "Installed: $BIN/exorchy (resources in $LIB)"
echo "Launch with 'exorchy' or from the Omarchy launcher (Super+Space)."
