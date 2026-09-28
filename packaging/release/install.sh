#!/usr/bin/env bash
# eXorchy installer for Arch / Omarchy.
#
# Downloads the prebuilt pacman package from a GitHub release, checks its
# SHA-256 and installs it with pacman (which pulls GTK, libadwaita,
# GStreamer, ... from the Arch repos and can remove it cleanly later).
#
#   curl -fsSL https://github.com/renanmt/exorchy/releases/latest/download/install.sh | bash
#
# Options (after `bash -s --` when piped):
#   --version vX.Y.Z      install that release instead of the latest
#   --download-only [DIR] download and verify only (default: current dir)
#   --uninstall           remove eXorchy (your games and settings stay)
#
# Running it again updates to the latest release.

set -euo pipefail

REPO="renanmt/exorchy"
ASSET="exorchy-x86_64.pkg.tar.zst"

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m==> WARNING:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31m==> ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# pacman and sudo prompt on the terminal: when this script arrives through a
# pipe, stdin is the script itself, which must never answer pacman. Without a
# terminal to ask on, stop instead of guessing the answers.
tty_in() {
  if { : </dev/tty; } 2>/dev/null; then
    "$@" </dev/tty
  elif [[ -t 0 ]]; then
    "$@"
  else
    die "No terminal to ask for your password and pacman's confirmation: run this in a terminal."
  fi
}

usage() {
  sed -n '2,17p' "${BASH_SOURCE[0]:-/dev/null}" 2>/dev/null | sed 's/^# \{0,1\}//' ||
    echo "Usage: install.sh [--version vX.Y.Z] [--download-only [DIR]] [--uninstall]"
}

uninstall() {
  if ! pacman -Qq exorchy >/dev/null 2>&1; then
    say "eXorchy is not installed."
    return
  fi
  say "Removing eXorchy..."
  tty_in sudo pacman -R exorchy
  say "Removed. Your settings and library are kept in ~/.local/share/exorchy"
  say "and ~/.local/state/exorchy, and downloaded games stay in your game folder."
}

download() { # <base-url> <dir>
  local base=$1 dir=$2
  say "Downloading $ASSET from ${base}"
  curl -fL --progress-bar -o "$dir/$ASSET" "$base/$ASSET" ||
    die "Download failed. Is there a release with $ASSET attached?"
  curl -fsSL -o "$dir/$ASSET.sha256" "$base/$ASSET.sha256" ||
    die "Could not download the checksum file."
  say "Checking the SHA-256 checksum..."
  (cd "$dir" && sha256sum --quiet -c "$ASSET.sha256") ||
    die "Checksum mismatch: the download is damaged or was tampered with. Nothing was installed."
}

main() {
  local version="" mode="install" dest="."
  while (($#)); do
    case $1 in
      --version) version=${2:?--version needs a tag like v0.2.0}; shift 2 ;;
      --download-only)
        mode="download"; shift
        if (($#)) && [[ $1 != --* ]]; then dest=$1; shift; fi ;;
      --uninstall) mode="uninstall"; shift ;;
      -h|--help) usage; return ;;
      *) die "Unknown option: $1 (try --help)" ;;
    esac
  done

  command -v pacman >/dev/null || die "pacman not found: eXorchy's package is for Arch Linux / Omarchy."
  [[ $mode == uninstall ]] && { uninstall; return; }
  [[ $(uname -m) == x86_64 ]] || die "Only x86_64 is built for now (this machine is $(uname -m))."
  command -v curl >/dev/null || die "curl is needed: sudo pacman -S curl"
  command -v sha256sum >/dev/null || die "sha256sum is needed (coreutils)."

  # EXORCHY_RELEASE_URL points at another copy of the release files (testing).
  local base
  if [[ -n ${EXORCHY_RELEASE_URL:-} ]]; then
    base=${EXORCHY_RELEASE_URL%/}
  elif [[ -n $version ]]; then
    [[ $version == v* ]] || version="v$version"
    base="https://github.com/$REPO/releases/download/$version"
  else
    base="https://github.com/$REPO/releases/latest/download"
  fi

  if [[ $mode == download ]]; then
    mkdir -p "$dest"
    download "$base" "$dest"
    say "Verified: $dest/$ASSET"
    say "Install it with: sudo pacman -U $dest/$ASSET"
    return
  fi

  # Global: the EXIT trap runs after main has returned.
  TMP_DIR=$(mktemp -d)
  trap 'rm -rf -- "${TMP_DIR:?}"' EXIT
  download "$base" "$TMP_DIR"

  say "Installing with pacman (it asks for your password and confirmation)..."
  tty_in sudo pacman -U --needed "$TMP_DIR/$ASSET"

  # A user-level install (packaging/install-dev.sh) comes first in PATH and
  # in the launcher and would hide the package.
  if [[ -e $HOME/.local/bin/exorchy || -e $HOME/.local/share/applications/org.exorchy.eXorchy.desktop ]]; then
    warn "An older user-level eXorchy is in ~/.local and hides this one. Remove it with:"
    warn "  rm -f ~/.local/bin/exorchy ~/.local/share/applications/org.exorchy.eXorchy.desktop; rm -rf ~/.local/lib/exorchy"
  fi
  if pgrep -x exorchy >/dev/null 2>&1; then
    say "eXorchy is running: restart it to use the new version."
  fi
  say "Done. Start eXorchy from the app launcher (Super+Space) or run: exorchy"
  say "Run this script again to update, or with --uninstall to remove it."
}

main "$@"
