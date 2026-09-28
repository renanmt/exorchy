#!/usr/bin/env bash
# Build the pacman package from this checkout exactly as the AUR would:
# packaging/aur/PKGBUILD over a `git archive` tarball of HEAD (what GitHub
# serves for a tag). Output in <out-dir>:
#   exorchy-x86_64.pkg.tar.zst         the package (stable name, for install.sh)
#   exorchy-x86_64.pkg.tar.zst.sha256  its checksum, `sha256sum -c` format
#   install.sh                         the installer
# Used by .github/workflows/release.yml and for local testing. As root (a CI
# container) makepkg runs as a throwaway `builder` user. MAKEPKG_FLAGS adds
# flags, e.g. MAKEPKG_FLAGS=-d when Rust comes from rustup, not pacman.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$(mkdir -p "${1:-$root/dist}" && cd "${1:-$root/dist}" && pwd)"
ver="$(sed -n 's/^pkgver=//p' "$root/packaging/aur/PKGBUILD")"
asset="exorchy-x86_64.pkg.tar.zst"

work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT

git -C "$root" archive --format=tar.gz --prefix="exorchy-$ver/" -o "$work/exorchy-$ver.tar.gz" HEAD
sed "s|^source=.*|source=(\"exorchy-$ver.tar.gz\")|" "$root/packaging/aur/PKGBUILD" >"$work/PKGBUILD"

if [[ $EUID -eq 0 ]]; then
  id builder >/dev/null 2>&1 || useradd -m builder
  chown -R builder: "$work"
  su builder -c "cd '$work' && makepkg -f --noconfirm ${MAKEPKG_FLAGS:-}"
else
  (cd "$work" && makepkg -f --noconfirm ${MAKEPKG_FLAGS:-})
fi

built=("$work"/exorchy-"$ver"-*-x86_64.pkg.tar.zst)
[[ -f ${built[0]} ]] || { echo "no package was built" >&2; exit 1; }
cp -- "${built[0]}" "$out/$asset"
(cd "$out" && sha256sum "$asset" >"$asset.sha256")
install -m755 "$root/packaging/release/install.sh" "$out/install.sh"
echo "Built $(basename "${built[0]}") -> $out/$asset"
