# eXorchy - `manifest.json` still points the poster and emulator packs at Exodium's GitHub
  release asset (`posters-eXoDOS-v5.tar.gz`); build and host eXorchy's own
  with `scripts/gen_thumbnails.py` + `gen_previews.py` (same hash scheme, drop-in).
- No HTTP manifest refresh; packs change only with an app release.
- `LICENSE` still spells the project "Exorchy" in the copyright line.
- E2E tests (Exodium used tauri-driver in a VM lab) were not ported; the UI
  has unit tests for pure logic only.
- Whole-screen flicker on pointer movement while eXorchy is open (2026-09-28) was Omarchy
  forcing HDR on the monitor, not the app: it stopped once HDR was turned off. GTK does not use
  Hyprland's colour-management protocol here. Check HDR first if it comes back.

## Verification status at handover (2026-09-27, end of the native conversion)

| Check | Result |
|---|---|
| `cargo test --workspace` | core 244 passed (3 ignored) + import_smoke 2; app 56 passed (1 ignored visual harness) |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo build -p exorchy` | ok |
| Responsive layout (Broadway/X11 snapshots at 520×600, 640×700, 960×768, 1280×800 via `EXORCHY_SNAPSHOT_SIZE`) | shelf and filter rows wrap; below 760 sp the toolbar stacks (wordmark + tabs, then search + badge + gear); detail panel overlays below 1100 sp and sits beside the grid above; grid goes down to one column |
| Snapshots (scratch offline profile, Broadway backend) | setup, Browse grid, list view, My Library shelves, detail panel (SimCity 2000 / Warcraft II with the ECE note and 44 press articles), Reading Room grid, Settings → Collections all render with the Omarchy `retro-82` palette |
| Settings pages (test harness, X11) | all eight pages rendered by the settings port |
| Reading Room reader + PDF viewer (X11) | offline fetch panel and a 6-page PDF at fit-width rendered by the reading port |
| `bash -n` PKGBUILD / install-dev.sh, `desktop-file-validate` | ok |

NOT verified end to end (needs the user's real profile and network, which this
session did not touch):
- the real torrent session: a download from Browse, cancel, the extras phase, resume after restart;
- a DOSBox Staging / Win9x / ScummVM launch from the native app, `game-exited` handling;
- preview video / theme music playback (this machine lacks `gst-plugins-good` / `gst-libav`);
- the online-only dialogs (seeding consent, welcome modal, Win9x network prompt), content-pack installs;
- `makepkg` and `packaging/install-dev.sh` end to end, `omarchy theme set` while the window is open.

- `EXORCHY_PRETEND_VERSION=0.2.0` makes the update check compare against that
  version, so the banner and the update flow can be seen against a real
  release. The Update button itself only appears for the pacman-installed
  `/usr/bin/exorchy`.
