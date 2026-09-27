# Exorchy - Handover for future sessions

Read this first, then `docs/ARCHITECTURE.md`. `docs/DECISIONS.md` explains why
things are the way they are; `docs/COLLECTIONS.md` explains how each collection
is wired and how enabling works. Keep all four current: a future agent starts from them.

## State of the project (2026-09-27, v0.1.0)

Scope (per the user, 2026-09-27): EVERYTHING Exodium gives, on Omarchy. Done:
- Rust backend = Exodium's full backend (all seven collections, media previews and music with the
  localhost media server, the Media Pack reading room, Win9x DOSBox-X/86Box + VHDs, ScummVM pinned
  builds), pruned to Linux, restructured around `launchers/` + `emulators.rs`, XDG dirs, the
  Omarchy theme bridge, DOSBox Staging as an emulator pack, and the enabled-collections filter
  (`db::queries::enabled_sql`). `cargo test --lib`: 241 passed; clippy `-D warnings` clean.
- Catalogue `metadata/exorchy.db.gz`, `CATALOG_VERSION` 16: 11,831 games (eXoDOS 7,666 / GLP 651 /
  SLP 642 / PLP 238 / Win3x 1,138 / Win9x 664 / ScummVM 832), 2,195 reading-room issues.
- Frontend = Exodium's full UI re-baselined, with: Omarchy token theming (no colour literals
  outside the lightbox), no custom title bar / updater / legacy layout migration, Setup with
  `setup_fresh`, Settings → Collections (per-pack switches, eXoDOS always on, default off for the
  rest), Emulators, Appearance, About credits, the splash screen (`components/Splash.tsx`),
  pdf.js manuals. `pnpm typecheck` clean, `pnpm test` 34 files / 284 tests, `pnpm build` ok.
- Credits: README tribute to eXoDOS + thank-you to Thomas Vollstädt, `ACKNOWLEDGEMENTS.md`,
  LICENSE, Settings → About.
- Packaging: `packaging/PKGBUILD`, `.desktop`, icons, `install-dev.sh`.
- `scripts/ui-smoke.mjs` (headless Chromium + mocked IPC) renders splash, Browse, detail, My
  Library and every Settings page with the real Omarchy palette; screenshots land in
  `work/ui-smoke/`.

## Machine / toolchain facts

- Omarchy 4.0.4 (Arch, Hyprland). WebKitGTK 2.52.6, gtk3, libsoup3 present.
- Rust via rustup in `~/.cargo` (not on PATH by default in fish):
  `export PATH="$HOME/.cargo/bin:$PATH"` before `cargo` / `pnpm tauri`.
- Node 22 via fnm; pnpm 12 installed with `npm install -g pnpm`.
- `dosbox-staging` is NOT in the Arch repos (AUR only: `dosbox-staging`,
  `dosbox-staging-bin`); the app downloads the official tarball instead.
- No `gst-plugins-good`/`gst-libav` installed (relevant only when media
  previews return in v0.2).

## Commands

```bash
export PATH="$HOME/.cargo/bin:$PATH"
pnpm install                       # once
pnpm tauri dev                     # run the app (Vite on :1420 + Rust)
pnpm typecheck && pnpm test        # frontend
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
pnpm gen-db                        # rebuild metadata/exorchy.db(.gz); raise db::CATALOG_VERSION FIRST
pnpm build && (cd src-tauri && cargo build --release)   # release binary
packaging/install-dev.sh           # user-level install (~/.local)
RUST_LOG=exorchy_lib=debug,librqbit=info pnpm tauri dev  # verbose
node scripts/ui-smoke.mjs         # headless UI screenshots (needs `pnpm dev` or `pnpm tauri dev` on :1420 + chromium)
```

Logs: `~/.local/state/exorchy/logs/exorchy.log`, `dosbox-<id>.log`.
Database: `~/.local/share/exorchy/exorchy.db`. Delete both dirs for a clean slate
(or Settings → About → Factory reset).

## Conventions (inherited from Exodium, still binding)

- Every Tauri command that touches DB, filesystem or network is `async fn`
  (a sync command runs on the main thread and freezes input).
- Never hold `DbState`'s mutex across an `.await`; blocking tasks open a side
  connection by path (`paths::app_data_dir().join(DB_FILE_NAME)`).
- Filesystem passes go through `block_in_place` / `spawn_blocking` (network
  shares).
- Tauri invoke args are camelCase in JS, snake_case in Rust.
- `raise_fd_limit` must run before the first torrent add (it does, in `run()`).
- Keep the `[patch.crates-io]` librqbit pin (deadlock fix) until upstream merges.
- SolidJS: `createSignal` + derived getters; stores for cross-component state;
  effects in the detail panel key on ids, never on `props.game`.
- Comments state the contract and the invariant, not the history; history
  goes to `docs/DECISIONS.md`.
- No hard-coded colors in CSS or TSX: only tokens derived from `--om-*`.
- Nothing DOS-specific outside `launchers/dosbox.rs` and the `exodos*`
  registry rows; nothing keys on a collection id string except the registry.

## Where to look for what

| Task | Start at |
|---|---|
| Launch fails / DOSBox not found | `emulators.rs`, `launchers/dosbox.rs::prepare`, `dosbox-<id>.log` |
| Download stuck | `stores/downloads.ts` (status strings), `install.rs::get_download_progress`, DECISIONS invariants |
| Wrong game merged with another | `db/queries.rs` `primary_row_condition` / `family_expr`, `collection_base_id` |
| Theme not applied | `omarchy.rs` (watcher log line "Watching ..."), `stores/theme.ts`, `main.css :root` |
| Adding a pack / emulator | `manifest.json`, `commands/content_packs.rs` (`unwrapped_source`, xz support) |
| Adding or debugging a collection | `docs/COLLECTIONS.md`, `commands/win9x.rs`, `commands/scummvm.rs` |
| Uninstall lost saves? | `user_data.rs` pristine index; `!save/<shortcode>` |

## Field fixes on this machine (2026-09-27, see DECISIONS)

- Win3x conf paths resolve case-insensitively (`eXoWin3X` vs `eXoWin3x`).
- Emulators get `SDL_AUDIODRIVER=pulseaudio` when pipewire-pulse's socket exists (ScummVM pack had no sound).
- On NVIDIA the WebKit web process is SIGKILLed on window close: its own exit segfaults in the
  NVIDIA EGL driver (`SkiaGPUWorker`, `__call_tls_dtors`) and produced a crash notification every time.

## Known gaps / TODO

- `manifest.json` still points the poster and emulator packs at Exodium's GitHub
  release asset (`posters-eXoDOS-v5.tar.gz`); build and host Exorchy's own
  with `scripts/gen_thumbnails.py` + `gen_previews.py` (same hash scheme, drop-in).
- No HTTP manifest refresh; packs change only with an app release.
- The Hyprland window: Tauri's GTK window on Wayland has app id
  `org.exorchy.Exorchy` (from `identifier`); the `.desktop` file's
  `StartupWMClass=exorchy` may need to become that id for the launcher to
  match the running window - verify with `hyprctl clients` once packaged.
- Keyboard-first navigation of the grid (Omarchy users live on the keyboard)
  is not implemented: `/` to focus search, arrows on cards, `Ctrl+,` for settings.
- E2E tests (Exodium used tauri-driver in a VM lab) were not ported.

## Verification status at handover (2026-09-27)

| Check | Result |
|---|---|
| `cargo test --lib` | 241 passed, 3 ignored |
| `cargo test --test import_smoke` | 2 passed |
| `cargo clippy --all-targets -- -D warnings` | clean |
| `pnpm typecheck` | clean |
| `pnpm test` (vitest) | 34 files, 284 tests passed |
| `pnpm build`, `cargo build --release` | ok |
| invoke-name coverage (`src/api/tauri.ts` + stores vs `lib.rs generate_handler!`) | every frontend command is registered |
| `node scripts/ui-smoke.mjs` | splash, Browse grid, detail panel, My Library, all eight Settings pages render (Ethereal palette) |
| `pnpm tauri dev` (earlier, DOS-only build) | booted on this NVIDIA + Wayland box, theme `ethereal` detected, watcher armed |

Verified by the user on 2026-09-27: the full-feature build runs under `pnpm tauri dev`, a fresh
setup at `~/eXoDOS`, eXoWin3x downloads land and extract (SkiFree). First field bug fixed the same
day: Win3x confs are spelled `eXoWin3X` in the catalogue but `eXoWin3x` on disk (see DECISIONS).

NOT verified end to end yet:
- a DOSBox Staging launch through the emulator pack path (the Win3x launch failed on the case bug
  before reaching the emulator; retry SimCity 2000 after the fix);
- a real Win9x / ScummVM launch, a preview video / theme track (needs `gst-plugins-good`,
  `gst-libav`; the UI hides the players when GStreamer cannot decode), the reading room;
- `omarchy theme set` while the window is open; Hyprland half-width tiling;
- `makepkg` from `packaging/PKGBUILD` (its steps were run individually).
