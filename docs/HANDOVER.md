# eXorchy - Handover for future sessions

Read this first, then `docs/ARCHITECTURE.md`. `docs/DECISIONS.md` explains why
things are the way they are; `docs/COLLECTIONS.md` explains how each collection
is wired and how enabling works; `docs/PORTING.md` is the brief for UI code
while the native port is in progress. Keep all of them current: a future agent
starts from them.

## State of the project (2026-09-27, v0.2.0, branch `native`)

eXorchy is being converted from a Tauri (WebKitGTK + SolidJS) app to a native
GTK4 / libadwaita app in Rust. The name is "eXorchy" (capital X) in every
user-facing string; binary, package, directories and identifiers stay
`exorchy`; the GApplication id is `org.exorchy.eXorchy`.

Done:
- Cargo workspace. `crates/core` = the backend as the `exorchy_core` library:
  Exodium's full backend (all seven collections, media previews and music with
  the localhost media server, the Media Pack reading room, Win9x DOSBox-X/86Box +
  VHDs, ScummVM pinned builds), pruned to Linux, restructured around
  `launchers/` + `emulators.rs`, XDG dirs, the Omarchy theme bridge, DOSBox
  Staging as an emulator pack, the enabled-collections filter
  (`db::queries::enabled_sql`), and no GUI dependency: `host.rs` replaces
  Tauri's `AppHandle` / `State` / events / `async_runtime`, `bootstrap()` /
  `shutdown()` / `resource_dir()` replace Tauri's setup. 243 unit tests + 2
  import-smoke tests pass.
- Catalogue `metadata/exorchy.db.gz`, `CATALOG_VERSION` 16: 11,831 games
  (eXoDOS 7,666 / GLP 651 / SLP 642 / PLP 238 / Win3x 1,138 / Win9x 664 /
  ScummVM 832), 2,195 reading-room issues.
- `crates/app` = the GTK4 app (bin `exorchy`): the tokio ↔ GTK bridge
  (`app.rs`), the Omarchy token layer (`theme.rs`, live), the undecorated
  window with splash, setup page, toasts and the web UI's startup order, the
  library (virtualised grid + list, collection shelf, genre tree, sort,
  playlists, jump bar, My Library shelves), the detail panel (variants, action
  bar, info, gallery, manual), covers scaled off-thread and cached, download
  trackers identical in behaviour to the web store, keyboard shortcuts, the
  `EXORCHY_SNAPSHOT` developer aid. Feature modules: see the checklist below.
- `legacy/webui` = the previous SolidJS UI, kept only as the porting
  reference until parity, then to be deleted.
- Credits: README tribute to eXoDOS + thank-you to Thomas Vollstädt,
  `ACKNOWLEDGEMENTS.md`, LICENSE, Settings → About.
- Packaging: `packaging/PKGBUILD` (gtk4 / libadwaita / poppler-glib /
  gstreamer stack, `check()` runs the core tests), `org.exorchy.eXorchy.desktop`,
  icons, `install-dev.sh`.

## Port checklist (modules being written in parallel, 2026-09-27)

Each row is a `crates/app/src/ui/<module>.rs` with its own
`crates/app/src/styles/<module>.css`; the hooks it plugs into already exist
(commit "Integration hooks for the feature modules"). Mark a row done when
the module replaces its stub, `cargo clippy -p exorchy -- -D warnings` is
clean and a snapshot shows it.

| Module | Web reference (`legacy/webui/src`) | Hook | Status |
|---|---|---|---|
| `settings.rs` - Settings dialog (General, Collections, Emulators, Appearance, Storage, Network, Packs, About) | `components/SettingsDialog.tsx`, `ContentPackSettings.tsx`, `StorageTab.tsx`, `stores/collections.ts`, `contentPacks.ts`, `network.ts` | `settings::open(parent, section)` from the toolbar button / Ctrl+, | in progress |
| `reading.rs` + `pdf.rs` - Reading Room tab, magazine/book/catalogue browser, PDF reader (poppler) | `components/ReadingRoom.tsx`, `IssueReader.tsx`, `PdfReader.tsx`, `DocumentViewer.tsx`, `ManualViewer.tsx`, `stores/reading.ts` | `reading::build(window)` → `library.set_reading_widget()` | in progress |
| `media.rs` - preview video + theme music (GStreamer), now-playing bar | `components/NowPlayingBar.tsx`, `MediaNotice.tsx`, `videoCanvas.ts`, `stores/videos.ts`, `music.ts`, `heroVideo.ts`, `mediaQueue.ts`, `playback.ts` | `media::install(window, library, bar_slot)`, fills `detail.media_slot` | in progress |
| `onboarding.rs` - seeding consent, welcome modal | `components/WelcomeModal.tsx`, `SeedingConsentDialog.tsx`, `stores/seeding.ts` | `onboarding::run(window)` after the library is up | in progress |
| `playlists.rs` - add to playlist, create / rename / delete | `components/PlaylistMenu.tsx`, `PlaylistNameDialog.tsx`, `stores/playlists.ts` | `playlists::pick_for_game(parent, game)` from the ⋯ menu | in progress |
| `game_settings.rs` - per-game shader / fullscreen / cycles / custom conf / ScummVM options | `components/GameSettingsDialog.tsx` | `game_settings::open(parent, game)` from the ⋯ menu | in progress |
| `launch_notes.rs` - the one panel note per game | `launchNotes.ts` + `launchNotes.test.ts`, `stores/notes.ts` | rendered by `detail.rs` | in progress |

When all rows are done: delete `legacy/webui`, drop the `legacy/webui/*`
lines from `.gitignore`, remove this table, and re-run the verification below.

## Machine / toolchain facts

- Omarchy 4.0.4 (Arch, Hyprland). GTK 4.22, libadwaita 1.9, poppler-glib,
  GStreamer core + base present; `gst-plugins-good` / `gst-libav` are NOT
  installed (preview videos and music need them; the players hide when
  GStreamer cannot decode).
- Rust via rustup in `~/.cargo` (not on PATH by default in fish):
  `export PATH="$HOME/.cargo/bin:$PATH"` before `cargo`.
- No node, no pnpm needed any more (the legacy web UI is not built).
- `dosbox-staging` is NOT in the Arch repos (AUR only: `dosbox-staging`,
  `dosbox-staging-bin`); the app downloads the official tarball instead.
- NVIDIA + Wayland: GTK4's GL renderer runs fine; the WebKit crash-on-close
  workaround of the Tauri build is gone with the webview.
- `grim` hangs on this compositor: use `EXORCHY_SNAPSHOT` for screenshots.

## Commands

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release -p exorchy                      # release binary: target/release/exorchy
cargo run -p exorchy                                  # run from the checkout (resources resolve to it)
cargo test -p exorchy-core                            # backend: 243 unit tests + import_smoke
cargo test -p exorchy                                 # UI unit tests
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p exorchy-core --release --example generate_db && gzip -kf metadata/exorchy.db
                                                      # rebuild the catalogue; raise db::CATALOG_VERSION FIRST
packaging/install-dev.sh                              # user-level install (~/.local)
(cd packaging && makepkg -si)                         # system-wide package
RUST_LOG=exorchy_core=debug,librqbit=info cargo run -p exorchy   # verbose
EXORCHY_SNAPSHOT=/tmp/shot.png:6000 cargo run -p exorchy         # render the window to a PNG and quit
EXORCHY_SNAPSHOT_GAME=9676 EXORCHY_SNAPSHOT=... EXORCHY_DUMP_TREE=1 ...  # + detail panel, + widget tree
```

An isolated profile for checks (offline copy of the catalogue, no torrent
session): set `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CONFIG_HOME`,
`XDG_CACHE_HOME` to a scratch dir as shown in `docs/PORTING.md`. The real
profile is `~/.local/share/exorchy`; do not modify it from a session.

Logs: `~/.local/state/exorchy/logs/exorchy.log`, `dosbox-<id>.log`.
Database: `~/.local/share/exorchy/exorchy.db`. Delete both dirs for a clean slate
(or Settings → About → Factory reset).

## Conventions (inherited from Exodium, still binding)

- Every backend command function in `crates/core/src/commands` that touches
  DB, filesystem or network is `async fn` and runs on tokio through
  `app::spawn` / `app::call`; the GTK thread never blocks on it.
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
- Nothing DOS-specific outside `launchers/dosbox.rs` and the `exodos*`
  registry rows; nothing keys on a collection id string except the registry
  and the enabled-set filter.

## Where to look for what

| Task | Start at |
|---|---|
| Launch fails / DOSBox not found | `emulators.rs`, `launchers/dosbox.rs::prepare`, `dosbox-<id>.log` |
| Download stuck | `crates/app/src/ui/downloads.rs` (status strings, thresholds), `install.rs::get_download_progress`, DECISIONS invariants |
| Wrong game merged with another | `db/queries.rs` `primary_row_condition` / `family_expr`, `collection_base_id` |
| Theme not applied | `omarchy.rs` (watcher log line "Watching ..."), `crates/app/src/theme.rs` (log line "Theme applied"), `style.css` fallbacks |
| Grid slow or not virtualising | `ui/library.rs` (GridView must be the ScrolledWindow's direct child), `ui/covers.rs` (card-size textures) |
| Backend call from the UI | `docs/PORTING.md` "Calling the backend", `crates/app/src/app.rs` |
| Adding a pack / emulator | `manifest.json`, `commands/content_packs.rs` (`unwrapped_source`, xz support) |
| Adding or debugging a collection | `docs/COLLECTIONS.md`, `commands/win9x.rs`, `commands/scummvm.rs` |
| Uninstall lost saves? | `user_data.rs` pristine index; `!save/<shortcode>` |
| What the app looks like | `EXORCHY_SNAPSHOT` (see Commands), then the Read tool on the PNG |

## Field fixes on this machine (2026-09-27, see DECISIONS)

- Win3x conf paths resolve case-insensitively (`eXoWin3X` vs `eXoWin3x`).
- Emulators get `SDL_AUDIODRIVER=pulseaudio` when pipewire-pulse's socket exists (ScummVM pack had no sound).
- (Tauri build only, now moot) the WebKit web process was SIGKILLed on close on NVIDIA.

## Known gaps / TODO

- The feature modules in the checklist above are in progress; until they
  land, Settings, the Reading Room, previews/music, playlists, per-game
  settings, onboarding and launch notes show stubs.
- `git ls-files target | wc -l` is 53,627: the `target/` directory was
  committed before `.gitignore` covered it (28 GB on disk). Run
  `git rm -r --cached target` and commit; likewise
  `legacy/webui/public/pdfjs` (189 files staged from pdfjs-dist) is tracked
  although ignored. Neither was done in the packaging/docs pass.
- `manifest.json` still points the poster and emulator packs at Exodium's GitHub
  release asset (`posters-eXoDOS-v5.tar.gz`); build and host eXorchy's own
  with `scripts/gen_thumbnails.py` + `gen_previews.py` (same hash scheme, drop-in).
- No HTTP manifest refresh; packs change only with an app release.
- `LICENSE` still spells the project "Exorchy" in the copyright line.
- E2E tests (Exodium used tauri-driver in a VM lab) were not ported; the UI
  has unit tests for pure logic only.

## Verification status at handover (2026-09-27, packaging/docs pass)

What was actually run in this pass, on the tree with the feature modules
still being written by other sessions:

| Check | Result |
|---|---|
| `cargo test -p exorchy-core --release` | 243 passed, 3 ignored (lib); `import_smoke` 2 passed |
| `cargo test -p exorchy` | 6 passed (UI unit tests) |
| `cargo clippy --workspace --all-targets -- -D warnings` | FAILS on `crates/app`: 12 dead-code errors for the hooks the in-progress feature modules will consume (`toolbar_slot`, `media_slot`, `on_shown`, `current`, `notify_*_changed`, `stop_all`, `active_ids`, `watch_extras_if_pending`, ...) plus 3 ordinary lints (`type_complexity`, `needless_borrow`, `field_reassign_with_default`) in `theme.rs` / `ui/*`; expected to clear when the modules land |
| `bash -n packaging/PKGBUILD`, `bash -n packaging/install-dev.sh` | ok |
| `desktop-file-validate packaging/org.exorchy.eXorchy.desktop` | ok |
| every path the PKGBUILD installs exists (`target/release/exorchy`, `metadata`, `torrents`, `manifest.json`, `crates/core/resources/previews`, `packaging/icons/{32,128,256,512}`, `crates/app/assets/exorchy.svg`, `LICENSE`, `ACKNOWLEDGEMENTS.md`) | ok |

NOT verified in this pass:
- `makepkg` end to end (`--locked` needs the committed `Cargo.lock` to match; it is in the tree);
- `packaging/install-dev.sh` end to end (it rebuilds the release binary);
- a run of the app (other sessions were editing `crates/app/src`);
- a DOSBox Staging launch through the emulator pack path, a real Win9x /
  ScummVM launch, a preview video / theme track (needs `gst-plugins-good`,
  `gst-libav`), the reading room, `omarchy theme set` while the window is
  open, Hyprland half-width tiling.
