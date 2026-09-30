# eXorchy - Handover for future sessions

Read this first, then `docs/ARCHITECTURE.md`. `docs/DECISIONS.md` explains why
things are the way they are; `docs/COLLECTIONS.md` explains how each collection
is wired and how enabling works; `docs/PORTING.md` holds the UI conventions;
`docs/RELEASING.md` is how a version ships; `docs/ROADMAP.md` is what to build
next and `docs/UPSTREAM-ISSUES.md` maps every Exodium issue to us. Keep all of
them current: a future agent starts from them.

## State of the project (2026-09-30, v0.4.0, branch `main`)

eXorchy is a native GTK4 / libadwaita app in Rust over the backend derived
from Exodium (MIT). The Tauri/SolidJS UI was deleted at parity (`ce3d70a9`).
The name is "eXorchy" (capital X) in every user-facing string; binary,
package, directories and identifiers stay `exorchy`; the GApplication id is
`org.exorchy.eXorchy`.

- `crates/core` (`exorchy_core`): Exodium's backend, pruned to Linux,
  restructured around `launchers/` + `emulators.rs`, XDG dirs, the Omarchy
  theme bridge, DOSBox Staging as an emulator pack, the enabled-collections
  filter (`db::queries::enabled_sql`), and no GUI dependency: `host.rs`
  replaces Tauri's `AppHandle` / `State` / events.
- `crates/app` (bin `exorchy`): the tokio ↔ GTK bridge (`app.rs`), the
  Omarchy token layer (`theme.rs`, live), the library (virtualised grid +
  list, shelves, genre tree, playlists, jump bar), the detail panel, the
  Reading Room and PDF reader, preview video and theme music, Settings,
  Transfers, onboarding, per-game settings, launch notes, the self-updater
  banner.
- Catalogue `metadata/exorchy.db.gz`, `CATALOG_VERSION` 17: 11,831 games
  (eXoDOS 7,666 / GLP 651 / SLP 642 / PLP 238 / Win3x 1,138 / Win9x 664 /
  ScummVM 832) and 2,195 Reading Room issues.
- Distribution: a GitHub release per `vX.Y.Z` tag carries a CI-built pacman
  package and `install.sh` (`docs/RELEASING.md`). The app updates itself from
  those releases (`commands/app_update.rs`). `packaging/aur/PKGBUILD` waits for
  AUR registrations to reopen.
- Credits: README tribute to eXoDOS + thank-you to Thomas Vollstädt,
  `ACKNOWLEDGEMENTS.md`, `LICENSE`, Settings → About.

## Machine / toolchain facts

- Omarchy (Arch, Hyprland). GTK 4.22, libadwaita 1.9, poppler-glib,
  GStreamer core + base. `gst-plugins-good` / `gst-libav` are NOT installed
  on the dev machine (preview videos and music need them; the players hide
  when GStreamer cannot decode). `fuse3` is installed, `fuse2` is not, which
  matters for the AppImage emulator packs (`docs/ROADMAP.md` step 1).
- Rust via rustup in `~/.cargo`, not on PATH by default in fish:
  `export PATH="$HOME/.cargo/bin:$PATH"` before `cargo`. No node, no pnpm.
- `dosbox-staging` is not in the Arch repos (AUR: `dosbox-staging`,
  `dosbox-staging-bin`); the app downloads the official tarball as a pack.
- `grim` hangs on this compositor: use `EXORCHY_SNAPSHOT` for screenshots.
- Whole-screen flicker on pointer movement while eXorchy is open (2026-09-28)
  was Omarchy forcing HDR on the monitor, not the app: it stopped once HDR
  was turned off. GTK does not use Hyprland's colour-management protocol
  here. Check HDR first if it comes back.

## Commands

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release -p exorchy                      # release binary: target/release/exorchy
cargo run -p exorchy                                  # run from the checkout (resources resolve to it)
cargo test --workspace                                # core + import_smoke + UI unit tests
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p exorchy-core --release --example generate_db && gzip -kf metadata/exorchy.db
                                                      # rebuild the catalogue; raise db::CATALOG_VERSION FIRST
packaging/install-dev.sh                              # user-level install (~/.local)
(cd packaging && makepkg -si)                         # system-wide package from the checkout
packaging/release/changelog-section.sh X.Y.Z          # preview a release's notes
RUST_LOG=exorchy_core=debug,librqbit=info cargo run -p exorchy   # verbose
EXORCHY_SNAPSHOT=/tmp/shot.png:6000 cargo run -p exorchy         # render the window to a PNG and quit
EXORCHY_SNAPSHOT_GAME=9676 EXORCHY_SNAPSHOT=... EXORCHY_DUMP_TREE=1 ...  # + detail panel, + widget tree
EXORCHY_PRETEND_VERSION=0.2.0 cargo run -p exorchy    # update check compares against that version
```

More snapshot switches (`_SIZE`, `_TAB`, `_SETTINGS`, `_READING`, `_SEQUENCE`,
…) are listed in `docs/PORTING.md`, together with the isolated profile for
checks: `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CONFIG_HOME`,
`XDG_CACHE_HOME` pointed at a scratch dir. The real profile is
`~/.local/share/exorchy`; do not modify it from a session.

The self-updater's Update button only appears for the pacman-installed
`/usr/bin/exorchy`; `EXORCHY_PRETEND_VERSION` shows the banner and the flow
against a real release. `EXORCHY_RELEASE_URL` points `install.sh` at another
copy of the release files for testing.

Logs: `~/.local/state/exorchy/logs/exorchy.log`, `dosbox-<id>.log`.
Database: `~/.local/share/exorchy/exorchy.db`. Delete both dirs for a clean
slate (or Settings → About → Factory reset).

## Conventions (inherited from Exodium, still binding)

- Every backend command function in `crates/core/src/commands` that touches
  DB, filesystem or network is `async fn` and runs on tokio through
  `app::spawn`; the GTK thread never blocks on it.
- Never hold `DbState`'s mutex across an `.await`; blocking tasks open a side
  connection by path (`paths::app_data_dir().join(DB_FILE_NAME)`).
- Filesystem passes go through `block_in_place` / `spawn_blocking` (network
  shares).
- Backend events are named strings with JSON payloads (`AppHandle::emit`);
  the UI subscribes with `app::on_event` and deserialises what it needs.
- `raise_fd_limit` must run before the first torrent add (it does, in `bootstrap()`).
- Keep the `[patch.crates-io]` librqbit pin (root `Cargo.toml`) until upstream merges.
- UI: widgets are touched on the GTK thread only; never hold a `RefCell`
  borrow across a callback; `glib::clone!(#[weak] ...)` into closures; the
  detail panel keys every refresh on ids, never on the game object.
- One CSS sheet per feature module (`crates/app/src/styles/`), registered
  in `theme.rs::STYLES`; no colour literals anywhere but the `--om-*`
  fallbacks in `style.css`.
- A new feature = a new `ui/<module>.rs` + one hook call, not edits spread
  across modules. Dialogs are libadwaita dialogs over the window; no second
  toplevel (Hyprland would tile it).
- Comments state the contract and the invariant, not the history; history
  goes to `docs/DECISIONS.md`.
- Emulator-specific code stays in its launcher module (`launchers/dosbox.rs`,
  `commands/win9x.rs`, `commands/scummvm.rs`); nothing keys on a collection id
  string except `commands/collections.rs` and the enabled-set filter.

## Where to look for what

| Task | Start at |
|---|---|
| Launch fails / DOSBox not found | `emulators.rs`, `launchers/dosbox.rs::prepare`, `dosbox-<id>.log` |
| Emulator silent | `launchers/mod.rs::prefer_pulse_audio_backend` (SDL → PipeWire's pulse socket) |
| Conf or bat path not found | `launchers/dosbox.rs` (case-insensitive mount targets and bat host paths) |
| Download stuck | `crates/app/src/ui/downloads.rs` (status strings, thresholds), `install.rs::get_download_progress`, DECISIONS invariants |
| Wrong game merged with another | `db/queries.rs` `primary_row_condition` / `family_expr`, `collection_base_id` |
| Catalogue upgrade lost or kept something | `db/mod.rs::refresh_catalog` (matches on `application_path`, then title + language) |
| Theme not applied | `omarchy.rs` (watcher log line "Watching ..."), `crates/app/src/theme.rs` (log line "Theme applied"), `style.css` fallbacks |
| Grid slow or not virtualising | `ui/library.rs` (GridView must be the ScrolledWindow's direct child), `ui/covers.rs` (card-size textures) |
| Backend call from the UI | `docs/PORTING.md` "Calling the backend", `crates/app/src/app.rs` |
| Adding a pack / emulator | `manifest.json`, `commands/content_packs.rs` (`unwrapped_source`, xz support) |
| Adding or debugging a collection | `docs/COLLECTIONS.md`, `commands/win9x.rs`, `commands/scummvm.rs` |
| Uninstall lost saves? | `user_data.rs` pristine index; `!save/<shortcode>` |
| App self-update | `commands/app_update.rs`, `ui/updates.rs`, `docs/RELEASING.md` |
| What the app looks like | `EXORCHY_SNAPSHOT` (see Commands), then the Read tool on the PNG |

## Known gaps / TODO

The planned work is in `docs/ROADMAP.md`. Beyond it:

- The release build runs the core tests but not clippy, so a lint can reach
  `main` unnoticed; run `cargo clippy` before tagging.
- `manifest.json` points the poster packs and the DOSBox-X / 86Box / ScummVM
  packs at Exodium's GitHub releases, and `manifest_url` is empty (no HTTP
  manifest refresh; packs change only with an app release). Roadmap step 2.
- E2E tests (Exodium used tauri-driver in a VM lab) were not ported; the UI
  has unit tests for pure logic only.
- `HANDOVER.md` itself was truncated to its last section by `ce3d70a9` and
  rebuilt on 2026-09-30 from `ef237346` plus the tree. If something the old
  version said seems missing, `git show ef237346:docs/HANDOVER.md`.

## Verification status (2026-09-30)

| Check | Result |
|---|---|
| `cargo test --workspace` | core 250 passed (4 ignored), import_smoke 2 passed, app 60 passed (1 ignored visual harness) |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |

Earlier checks, from the end of the native conversion (2026-09-27), not
re-run since:

| Check | Result |
|---|---|
| Responsive layout (Broadway/X11 snapshots at 520×600, 640×700, 960×768, 1280×800 via `EXORCHY_SNAPSHOT_SIZE`) | shelf and filter rows wrap; below 760 sp the toolbar stacks (wordmark + tabs, then search + badge + gear); detail panel overlays below 1100 sp and sits beside the grid above; grid goes down to one column |
| Snapshots (scratch offline profile, Broadway backend) | setup, Browse grid, list view, My Library shelves, detail panel (SimCity 2000 / Warcraft II with the ECE note and 44 press articles), Reading Room grid, Settings → Collections all render with the Omarchy `retro-82` palette |
| Settings pages (test harness, X11) | all eight pages rendered |
| Reading Room reader + PDF viewer (X11) | offline fetch panel and a 6-page PDF at fit-width rendered |
| `bash -n` PKGBUILD / install-dev.sh, `desktop-file-validate` | ok |

Never verified by a session (needs the user's real profile and network):
- the real torrent session: a download from Browse, cancel, the extras phase, resume after restart;
- a DOSBox Staging / Win9x / ScummVM launch from the native app, `game-exited` handling;
- preview video / theme music playback (the dev machine lacks `gst-plugins-good` / `gst-libav`);
- the online-only dialogs (seeding consent, welcome modal, Win9x network prompt), content-pack installs;
- `packaging/install-dev.sh` end to end, `omarchy theme set` while the window is open.

The CI release build (`makepkg` in a clean container) has run for each
published release since 0.2.0; see `docs/RELEASING.md`.
