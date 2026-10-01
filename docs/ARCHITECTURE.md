# eXorchy - Architecture

eXorchy is an Omarchy-native (Arch Linux + Hyprland/Wayland) launcher for the
eXo collections: browse the catalogue, stream single games out of the eXo
torrents, and play DOS, Windows 3.x, Windows 9x and ScummVM games, with
previews, theme music and the Media Pack reading room, themed from Omarchy's
current theme. It is a native GTK4 + libadwaita application written in Rust.
The backend is derived from Exodium (MIT,
https://github.com/tvollstaedt/exodium, analysed at commit `f7e9fcde`,
v0.15.1) and carries its full feature set; the user interface was first
Exodium's SolidJS web UI in a Tauri webview and is being rebuilt natively
(see `docs/DECISIONS.md` 2026-09-27 and `docs/PORTING.md`).

## Stack

| Layer | Technology | Why |
|---|---|---|
| Shell | GTK 4.22 + libadwaita 1.9 (`gtk4` / `libadwaita` crates, gtk-rs), one undecorated `adw::ApplicationWindow` | what Omarchy ships; Hyprland draws borders, tiles, moves, closes |
| Backend | Rust library `exorchy_core` (`crates/core`), no GUI dependency | Exodium's backend, pruned to Linux, driven through a small host shim |
| Bridge | tokio runtime owned by `exorchy_core::host::async_runtime`; results and events pumped to the GTK main loop (`crates/app/src/app.rs`) | backend `async fn`s never touch widgets, widgets never block |
| Database | SQLite via `rusqlite` (WAL), pre-built catalogue shipped gzipped | ~6 MB catalogue instead of eXo's 5 GB metadata zip |
| Torrent | `librqbit` 9.0.0-rc.0 (fork pin, see DECISIONS) with selective file downloads | stream one game at a time |
| Emulators | DOSBox Staging 0.83.0 fetched as an emulator pack (or the system binary); DOSBox-X / 86Box / ScummVM packs for the other collections | AUR-only on Arch |
| Media | GStreamer (previews, music), poppler-glib (manuals, magazines), `image` (covers) | native players instead of the webview's |
| Theme | Omarchy `colors.toml` + `shell.toml`, watched with inotify, applied as GTK CSS custom properties | live theme switching |

## Repository layout

```
exorchy/
├── Cargo.toml                workspace: members, shared deps, the librqbit [patch.crates-io] pin, release profile
├── crates/core/              exorchy_core - the backend (lib crate, no GUI dependency)
│   ├── src/lib.rs            bootstrap(): XDG dirs, logger, DB install/refresh, root folder, enabled set,
│   │                         managed state, theme watcher; shutdown(); resource_dir()
│   ├── src/host.rs           the host shim: AppHandle, State, events, async_runtime (replaces Tauri)
│   ├── src/commands/         command functions by responsibility (see below)
│   ├── src/launchers/        the emulator seam: mod.rs (spine, process tracking, dispatch), dosbox.rs
│   ├── src/emulators.rs      where DOSBox Staging (pack / system / custom) and DOSBox-X come from
│   ├── src/media_sources.rs  the Media Pack torrent joining the shared session
│   ├── src/vhd.rs            differencing VHDs for 86Box (Windows 9x)
│   ├── src/omarchy.rs        theme bridge (colors.toml, shell.toml, fc-match, inotify)
│   ├── src/db/               schema, refresh_catalog, queries, reading tables
│   ├── src/torrent/          librqbit manager, .torrent index, ranged zip reader
│   ├── src/import/           LaunchBox XML parser (build time + import backfill)
│   ├── src/support_files.rs  util.zip payloads (MT-32 ROMs) - queue, watch, extract
│   ├── examples/generate_db.rs   builds metadata/exorchy.db
│   └── resources/previews/<col>/  Tier 0 covers (120 px), bundled; eXoMedia = reading-room covers
├── crates/app/               exorchy - the GTK4 app (bin `exorchy`)
│   ├── src/main.rs           adw::Application (id org.exorchy.eXorchy), bootstrap, single instance
│   ├── src/app.rs            the tokio ↔ GTK bridge: spawn / call / local / on_event
│   ├── src/theme.rs          Omarchy palette → CSS custom properties on :root, live
│   ├── src/style.css         base rules; src/styles/<module>.css one sheet per feature module
│   ├── src/snapshot.rs       EXORCHY_SNAPSHOT: render the window to a PNG (developer aid)
│   ├── src/ui/               window, splash, setup, library, card, detail, model, covers, downloads,
│   │                         actions, bus, dialogs, util + the feature modules (settings, reading,
│   │                         media, playlists, game_settings, onboarding)
│   └── assets/               splash.jpg, exorchy.png (icon), logo.txt (ASCII wordmark), collections/<col>.jpg
├── metadata/                 bundled XML (gz), configs zips, variant indexes, media index, exorchy.db.gz
├── torrents/                 every eXo .torrent (DOS packs, Win3x, Win9x, ScummVM, Media Pack)
├── manifest.json             content packs + emulator packs per collection
├── packaging/                PKGBUILD, org.exorchy.eXorchy.desktop, icons/, install-dev.sh
├── scripts/                  pack-building scripts (thumbnails, previews, LP configs)
└── docs/                     this file, HANDOVER.md, DECISIONS.md, COLLECTIONS.md, PORTING.md
```

## Directories at runtime (XDG)

| Path | Holds |
|---|---|
| `~/.local/share/exorchy/exorchy.db` | the catalogue + user state (WAL) |
| `~/.local/share/exorchy/launch/` | per-launch DOSBox conf fragments, emptied at startup |
| `~/.local/share/exorchy/librqbit-fastresume/` | `session.json`, `<infohash>.bitv`, `.torrent` copies |
| `~/.local/state/exorchy/logs/exorchy.log` | app log (10 MiB rotation), `dosbox-<id>.log` per launch |
| `<data_dir>` (the library folder; user-chosen, default `~/Games/eXorchy`, `$HOME` before 2026-09-30) | the game data dir; `library_location.rs` moves it |
| `<data_dir>/eXoDOS/` (`root_folder`) | the single game root shared by every collection, eXo's own merged layout |
| `<data_dir>/content/` | eXorchy's own: `posters/<col>`, `metadata/<col>`, `emulators/<pack>`, `thumbcache`, `videocache`, `musiccache`, `magazinecache`, `pristine` |

Bundled resources (`exorchy_core::resource_dir()`): `EXORCHY_RESOURCE_DIR`
if set, else the first of `/usr/lib/exorchy` (package) and
`~/.local/lib/exorchy` (`install-dev.sh`) that has a `metadata/` directory,
else the source checkout (`commands::paths::dev_project_root`, two levels
above `crates/core`). A resource dir holds `metadata/`, `torrents/`,
`manifest.json` and `previews/<col>/`.

Game root layout (unchanged from eXo / Exodium):

```
<root>/eXo/                            DOSBox cwd; every `.\` in eXo's confs is relative to it
  eXoDOS/<shortcode>/                  extracted game
  eXoDOS/<Title (Year)>.zip            archive from the torrent
  eXoDOS/!dos/<shortcode>/dosbox.conf  per-game conf (+ .bat), laid down from eXoDOS_configs.zip
  eXoDOS/!german|!polish|!spanish/...  language packs (same shape, own !save)
  eXoDOS/!save/<shortcode>/            uninstall backups (changed files only)
  mt32/                                MT-32 ROMs + SoundCanvas.sf2 from util.zip
  .exorchy_launch_<shortcode>.conf     the patched conf for the running launch
<root>/Content/GameData/eXoDOS/<Title (Year)>.zip   extras (manuals, videos), shared by all languages
```

## The host shim (`crates/core/src/host.rs`)

The backend was written against Tauri's `AppHandle`, `State<'_, T>`, `emit`
and `async_runtime`. `host.rs` provides the same four things without a
webview, so the 100-odd command functions kept their signatures:

- `AppHandle`: cheap, cloneable; `manage(value)` / `state::<T>()` /
  `try_state` over a type-keyed map of `Arc<dyn Any>`; `emit(name, payload)`
  broadcasts an `Event { name, payload: serde_json::Value }` on a tokio
  broadcast channel; `subscribe()` returns a receiver.
- `State<'a, T>`: `Arc<T>` with a phantom lifetime, `Deref<Target = T>`;
  `AppHandle::state()` returns `State<'static, T>` and a `State<'_, T>`
  parameter accepts it.
- `Manager` / `Emitter`: empty traits kept so the ported `use` lines compile.
- `async_runtime`: the one multi-thread tokio runtime the process owns
  (`runtime()`, `handle()`, `spawn`, `spawn_blocking`, `block_on` that
  yields the worker inside a runtime).

`exorchy_core::bootstrap()` does what Tauri's `setup` did and returns
`Bootstrap { app, startup_error, log_path }`; `exorchy_core::shutdown(&app)`
flushes the torrent session. Both are synchronous and are called from
`crates/app/src/main.rs`.

## Backend modules (`crates/core/src/commands`)

| Module | Responsibility |
|---|---|
| `collections.rs` | `CollectionDef`, `COLLECTION_MAP` (GLP, PLP, SLP, eXoDOS, eXoWin3x, eXoWin9x, eXoScummVM), `Launcher` enum, path shapes (`collection_rel_game_dir`, `collection_rel_zip`), family (`collection_base_id`) |
| `paths.rs` | XDG app dirs, resource dir, the cached `root_folder`, `game_root()`, launch-conf dir |
| `library_location.rs` | where the library lives: `library_status` (found? old `$HOME` layout? pending move?), `plan_library_move` / `request_library_move` (validate, record `library_move_target`), `run_library_move` (rename on one disk, staged copy + `library-move-progress` events across disks, `session.json` output folders rewritten, `data_dir` switched last, originals deleted via `library_move_cleanup`), `locate_library`, `relaunch_after_exit` |
| `setup.rs` | `TorrentState`, `init_download_manager` (session, one `DownloadManager` per enabled collection, bundled config extraction, hydration), `setup_fresh`, `setup_from_local` (import an existing eXo tree), `factory_reset`, `shutdown_torrent_session` |
| `games.rs` | catalogue queries (`get_games` merged-per-shortcode, variants, genres, section keys), config, per-game settings, transfer stats, **`launch_game` spine**, `stop_game`, `running_game_ids`, `game_printing_unavailable` |
| `install.rs` | `download_game` (disk preflight, GameData, MT-32 payload, LP base dependency), `get_download_progress` (1 Hz poll that also extracts on completion), `cancel_download`, `extract_before_launch`, `LaunchError` |
| `library.rs` | torrent-index matching, install scan (three passes, "library follows intent"), `uninstall_game`, `reset_game_data` |
| `user_data.rs`, `archives.rs` | pristine CRC check on uninstall; keep/drop archives, disk usage |
| `lp_overlay.rs` | language-pack patches that need the English base (eXo's `multilanguage_<lang>.txt`) |
| `content_packs.rs`, `updates.rs` | manifest, HTTP (tar.gz / tar.xz) and torrent-sourced packs, ledger, adoption |
| `assets.rs` | cover tiers (`get_preview_dir`, `get_poster_dir`), gallery scan + thumb cache, manuals |
| `storage.rs` | Settings → Storage walk and cleanups |
| `emulator_cmds.rs` | `get_dosbox_status`, `ensure_dosbox_staging`, `get_dos_support_status` |
| `win9x.rs`, `scummvm.rs` | the Windows 9x (DOSBox-X / 86Box, VHDs, pcap multiplayer) and ScummVM (pinned builds, variant tree) launchers and their panel probes |
| `media.rs` | preview videos and theme music read out of GameData zips by ranged reads, cached, served by the localhost media server (axum) |
| `reading.rs` | the Media Pack reading room: magazines, books, catalogues, disk magazines |
| `playlists.rs`, `shell_open.rs` | playlists; xdg-open with exit-code reading |

Other backend modules: `launchers/` (below), `emulators.rs`, `omarchy.rs`,
`support_files.rs`, `db/`, `torrent/`, `import/`, `host.rs`.

Backend events (`AppHandle::emit`, received by `app::on_event`):
`theme-changed`, `game-exited`, `dependency-download-started`, the
content-pack progress events, and the media/reading events the feature
modules subscribe to.

## The launch pipeline

`commands::games::launch_game` is collection-agnostic:

1. take the per-game op-lock (shared with download / extract / uninstall / reset);
2. read the row, global prefs (`global_glshader`, `default_fullscreen`) and
   `game_config`; stamp `last_played`;
3. guards: installed, not already running (`launchers::running_games`);
4. `extract_before_launch` turns the archive into a game dir if needed
   (placeholder and still-downloading detection);
5. `launchers::prepare(kind, ctx)` dispatches on the collection's `Launcher`
   kind: DOSBox returns `LaunchOutcome::Prepared(cmd)`; the Win9x and ScummVM
   launchers spawn themselves and return `LaunchOutcome::Launched(msg)`;
6. for a prepared command, `launchers::spawn_emulator_and_track` sets stdio
   (`dosbox-<id>.log`), spawns, tracks pid + run key, and emits `game-exited`
   from the reaper. The other launchers call the same function.

`launchers/dosbox.rs::prepare` (DOS and Windows 3.x): `resolve_game_conf` (catalogue
path, case-insensitive walk, then lang-scoped alternates) →
`rewrite_bat_host_paths` (`.\x\y` → `./x/y` in the game's bats when the target
exists) → engine: `chosen_engine` = the per-game `engine` setting, else
`exo_engine` (DOSBox-X for eXo's `x` / `x2` pins and every conf that prints,
DOSBox Staging otherwise) → `patch_dosbox_conf` (host-path rewrite with
trailing-separator and quoting rules, LP overlay mount via a symlink staging
dir; for Staging only, translation of ECE `[midi]` keys and DOSBox-X `[ide]`
sections, which DOSBox-X reads natively) → `emulators::resolve_dosbox_staging`
or `emulators::resolve_dosbox_x` (pack AppImage, PATH, Flatpak with the root
and launch dir granted) → command line `<emulator> -conf <patched> [-conf
options.conf] [DOSBox-X: -nomenu, -conf printer_<id>.conf] -conf
global_overrides_<id>.conf [-conf game_<id>.conf]` with cwd `<root>/eXo`. The
printer fragment sends DOSBox-X's pages to PNG files in `<game dir>/!prints`
(eXo's confs say `printoutput=printer`, Windows-only); CRT shaders are
Staging's and are left out for DOSBox-X. `download_game` queues the
`dosbox-x` pack (owned by eXoWin9x in the manifest, `pack_collection`) with
the first download of a game that needs it. The field knowledge encoded in these functions is
covered by the unit tests copied from Exodium; keep them. Every emulator spawn
gets `SDL_AUDIODRIVER=pulseaudio` when pipewire-pulse's socket exists.

Adding a launcher: a module with `prepare(ctx)`, one arm in
`launchers::prepare`, one `Launcher` variant, one `CollectionDef` row (see
`docs/COLLECTIONS.md`).

## Emulator provisioning

`emulators.rs` resolves DOSBox Staging in this order: the `dosbox_binary`
config override → PATH when `use_system_dosbox = 1` → the emulator pack at
`<data_dir>/content/emulators/dosbox-staging/dosbox` → PATH. The pack is a
content pack (`manifest.json`, `eXoDOS.content_packs.dosbox-staging`, a
`platforms` map) that downloads the official Linux release tarball
(`.tar.xz`, sha256-verified) and unpacks it in place; the tarball is
self-contained (`lib/`, `plugins/`, `resources/shaders`). `ensure_dosbox_staging`
runs after every session start (and after going online) and queues the pack
when nothing resolves. Settings → Emulators shows the status.

MT-32 ROMs and the SoundCanvas soundfont come from the eXoDOS torrent's
`eXo/util/util.zip` (`support_files::DOS_SUPPORT`), queued with the first
game whose conf requests MIDI and extracted by a watcher; the launch never
waits for it.

## Downloads

One librqbit session per app (`~/.local/share/exorchy` as session dir), one
`DownloadManager` per enabled collection, all writing into the single game
root. "Download game X" = add X's file indices to the torrent's `only_files`
set; progress is `get_download_progress` polled at 1 Hz by
`crates/app/src/ui/downloads.rs`, whose trackers behave exactly like the web
store did: a null poll counts toward a threshold of 5 before the tracker
gives up, stall hints at 15 s and 90 s without progress, an "extras" phase
after the game itself is playable, a tracker started for every
`dependency-download-started` event (an English base for a translation), and
`list_active_downloads` re-arms trackers after a restart. Completion
triggers extraction from inside the poll. Seeding is opt-in
(`seeding_enabled = "1"` only; off = 1 KB/s upload cap); offline mode creates
no session at all. Invariants that cost Exodium field bugs are listed in
`docs/DECISIONS.md` ("Operational invariants").

## Data layer

`metadata/exorchy.db.gz` is built by
`cargo run -p exorchy-core --release --example generate_db && gzip -kf metadata/exorchy.db`
from the LaunchBox XML, the `.torrent` file lists (`game_torrent_index`,
`gamedata_torrent_index`, `download_size`), `dosbox.txt` (eXo's emulator
variant per game), `Playlists.xml.gz` (curated playlists) and the LP
`confdirs`/`multilanguage` lists. `db::CATALOG_VERSION` gates
`refresh_catalog`, which updates catalogue columns in place and preserves
`id`, `in_library`, `installed`, `favorited`, `last_played`, `game_config` and
user playlists. Raise `CATALOG_VERSION` BEFORE running the generator.

Identity: a game is (family, shortcode); rows sharing it are language
variants merged into one card (`queries::primary_row_condition`,
`attach_language_maps` → `available_languages = "EN:2,DE:0"`).

## The GTK app (`crates/app`)

### Bridge (`app.rs`)

`app::install(core)` stores the `AppHandle` in a thread-local on the GTK
thread and starts the event pump: a `glib::spawn_future_local` task that
awaits the broadcast receiver and dispatches each event to the listeners
registered with `app::on_event(name, closure)`. `app::core()` returns the
handle for building command futures. `app::spawn(fut, done)` runs a backend
future on tokio and calls `done(result)` on the GTK thread; `app::call(fut)`
awaits a backend future from inside a main-loop task; `app::local(fut)`
spawns a main-loop future that may capture widgets. Widgets are touched on
the GTK thread only; backend futures never capture widgets.

### Window (`ui/window.rs`)

One `adw::ApplicationWindow`, title "eXorchy", undecorated (Hyprland draws
borders and tiles it), default 1280×800, minimum 900×600, CSS class
`exorchy`. Inside: an `adw::ToastOverlay` around a `gtk::Overlay` whose
child is a `gtk::Stack` (setup ↔ library, crossfade) and whose overlay is the
splash. Startup order in the library phase mirrors the web `App.tsx`:
listeners first (theme, content packs, dependency downloads, running games),
then `get_setup_status`, network mode, cover dirs, `init_download_manager`,
seeding consent, packs, transfer polling, `ensure_dosbox_staging`,
`scan_installed_games` → first fetch + `resume_downloads`, then
`onboarding::run`. `restart_to_setup()` (factory reset) and
`reinit_library()` (collections changed) rebuild the pages; `library()`
returns the live page. Single instance comes from GApplication: a second
`exorchy` activates the running one, which presents its window.

The splash (`ui/splash.rs`) shows `assets/splash.jpg` on its own dark
backdrop for at least 1.6 s and until the app knows what to render, then
fades; it is an overlay inside the window, never a second toplevel.

The wordmark (`ui/logo.rs`) is `assets/logo.txt`, half-block ASCII art, drawn
as pixel art: one `DrawingArea` per text row, each character two square
pixels, coloured per row by `styles/logo.css` from the palette tokens. It is
the library toolbar's brand (1.5 px per pixel) and the About page's header
(4 px). The sources of the key art, icon and logo live in `media/`.

Keyboard (an `EventControllerKey` on the window): `/` focuses search unless
an entry has focus, Esc closes the detail panel, Ctrl+, opens Settings;
arrows, Page Up/Down, Home/End and Enter are GridView's / ListView's own.

### Library (`ui/library.rs`, `ui/card.rs`, `ui/model.rs`)

Tabs Browse / My Library / Reading Room. Browse: search entry, genre tree
dropdown, sort dropdown, playlist dropdown, grid/list switch, the collection
shelf (enabled collections only, hidden with a single one), the jump bar
(section keys per sort; jumping fetches everything when the target is not
loaded yet) and the detail panel beside them. The games live in a
`gio::ListStore<GameObject>` bound to a `gtk::GridView` (virtualised; the
GridView is the ScrolledWindow's direct child, see DECISIONS) with
`PER_PAGE = 100`, load-more on scroll, and a fetch epoch so a stale page
never lands on a newer filter; `refresh_loaded` re-reads the loaded rows in
place after library changes ("library follows intent"). The list view is a
`gtk::ListView` with a header over the same store. My Library: shelves for
recently played, installed, favourites and user playlists. `LibraryPage`
exposes `toolbar_slot`, `bar_slot` and `set_reading_widget()` for the feature
modules.

### Detail panel: the game dossier (`ui/detail.rs`, `styles/detail.css`)

The split view's end sidebar (`PANEL_WIDTH` 560 px; it overlays the grid in
narrow windows) showing one game as a dossier:
- header: favourite star (F), "GAME DOSSIER", close (Esc);
- hero: cover beside the platform (accent), title, year · genre · developer,
  language chips (one variant at a time; every refresh keys on ids, never on
  the game object), the primary action (Play as an `adw::SplitButton` whose
  menu holds Game settings and the manual; Stop / Download / Cancel /
  progress otherwise), then Add to playlist and ⋯ (hide, reset, uninstall);
- the launch note (`note_slot`), then `media_slot` (preview video and theme
  row; deliberately not in a tab, since the video pauses the theme music and
  a hidden tab would play it unseen), genre tags, description;
- tabs (`adw::ViewStack` + `adw::InlineViewSwitcher` styled as an underline
  row): Overview (facts beside Features, which are only true facts: emulator,
  printing, players, manual, language versions, CRT shaders; then eXo's
  notes), Media (screenshots, count in the title; "Covered in" press
  articles), Manuals (hidden without one), Setup (emulator and whose choice
  it is, collection, status, Game settings, Reset).
Hooks: `on_shown(cb)`, `refresh_by_id`, `refresh_download`,
`refresh_running`, `shows(id)`, `reopen()`, `play_shown()`,
`favorite_shown()`. Keys (library.rs): Enter plays the shown game, F stars
it, I shows or hides the dossier; Enter or a double-click on the grid card
the dossier already shows plays it.

### Covers (`ui/covers.rs`)

Tier 1 poster pack (400 px) then Tier 0 bundled preview (120 px), both
`<dir>/<thumbnail_key>.jpg`; directories resolved once per collection (and
again when a pack lands). Files are decoded and scaled with the `image`
crate on tokio's blocking pool to the card's pixel size (`Size::Fill(w, h)`
cover-crop, `Size::Fit` for the panel), uploaded as a `gdk::Texture` on the
GTK thread and cached by (path, size), 900 entries.

### Shared plumbing

`ui/actions.rs` (download, play, stop, uninstall, uninstall group, reset,
favourite, open document, add to playlist, game settings), `ui/bus.rs`
(library / collections / playlists / running change signals, the offline
flag, `toast()` / `toast_with()`), `ui/dialogs.rs` (confirm, error,
pick_folder over `adw::AlertDialog` / `gtk::FileDialog`), `ui/util.rs`
(`format_bytes`, `esc`, `collection_label`, `platform_tag`,
`parse_lang_entries`), `ui/setup.rs` (first run: fresh data dir or import,
network choice).

### Feature modules (being ported, see HANDOVER)

| Module | Integration point |
|---|---|
| `ui/settings.rs` | `settings::open(parent, section)` from the gear button and Ctrl+,: a full-body page in place of the library (`window::show_page`), back arrow or Esc returns. An `adw::NavigationSplitView` of General, Collections, Hidden titles, Emulators, Appearance, Storage, Network, Packs, About; below 720 sp it collapses to list → section with back arrows and every row stacks (`widgets::follow_narrow`); content is clamped to 960 px |
| `ui/transfers.rs` | `transfers::open(parent)` from the toolbar's connection badge (`library.activity_button`): a full-body page like Settings. Downloading (games, content packs, Reading Room, preview/theme fetches), Queued (media fetches waiting for a slot), Torrents (`games::get_session_torrents`: every session torrent with its own rates, peers, upload); 1 s poll, rows updated in place |
| `ui/library_location.rs` | `library_location::gate(window, stack, show_library)` from `window.rs` once setup is done, before the torrent session starts: runs a move Settings requested (progress page), asks where a missing library went (Locate / Continue without it), offers once to move a library still in `$HOME` to `~/Games/eXorchy` (refusal kept in `library_move_declined`). `move_from_settings` / `locate_from_settings` back Settings → General → Library folder; Move records the target and restarts the app, it never moves while the app runs |
| `ui/sidebar.rs` | `sidebar::build(on_pick, on_values)` from `library.rs`, left of Browse's grid (hidden below 1100 sp, where the filter bar's dropdowns carry the same filters): All Games (count), Platforms, Genres, Publishers, Series, Years, Regions, Tags, Play Status, Favorites. A category lists its values with counts (`games::get_facet_values`) on the `values` page, a virtualised `ListView` with a filter field that the library shows in its view stack; a pick goes back as `Pick` and `LibraryPage::apply_pick` sets the filters (one sidebar value at a time; Platforms / Genres / Years / Regions move their dropdowns, the rest show as a removable chip). Tags and series both come from eXo's `series` field: `Prefix: value` entries are tags (`queries::is_tag`). `styles/sidebar.css` |
| `ui/statusbar.rs` | `statusbar::build(&library.status)` from `library.rs`: the footer of the concept (version, the library's own count label reparented into it, favourite / playlist / collection counts refreshed on the bus, key hints Enter / I / F / / / Esc). Breakpoints in `library.rs`: the hints hide below 1300 sp, the counts below 760 sp; `styles/statusbar.css` |
| `ui/updates.rs` | `updates::install(window, library.banner_slot)`: checks GitHub for a newer release (`app_update::check_app_update`, 8 s after start then every 6 h, unless `update_check` = "0" or offline; `update_skipped` holds a dismissed tag) and shows a banner above the toolbar. Update (only for the pacman-installed `/usr/bin/exorchy`) confirms, opens Omarchy's floating terminal running that release's `install.sh --version`, waits for this process to exit and starts eXorchy again (`app_update::launch_app_update`); the app quits. Settings → General switches the check, About shows the status with Check now / Update |
| `ui/backdrop.rs` | `backdrop::install(window)` gives `window.rs` the picture it layers under the page stack (the overlay measures the pages, not the picture). Settings → Appearance chooses / removes the image (copied into the data folder, config `background_image`) and sets its opacity (`background_opacity`, 5–100 %); the window then wears `has-backdrop` and `styles/backdrop.css` turns the full-width bars and pages translucent |
| `ui/hidden.rs` | hidden-id set for the card and ⋯ menus (Hide / Unhide title, Remove from Recently played), Undo toast, the adult switch; `bus::notify_visibility_changed()` makes the library refetch Browse, genres and shelves (see COLLECTIONS.md) |
| `ui/reading.rs` | `reading::build(window)` → `library.set_reading_widget()`; the Reading Room tab and the PDF reader (`ui/pdf.rs`, poppler) |
| `ui/media.rs` | `media::install(window, library, bar_slot)`: fills `detail.media_slot` (preview video, theme music) and the now-playing bar under the library |
| `ui/playlists.rs` | `playlists::pick_for_game(parent, game)` from the ⋯ menu; create / rename / delete |
| `ui/game_settings.rs` | `game_settings::open(parent, game)` from the ⋯ menu; shader, fullscreen, cycles, custom conf, ScummVM options |
| `ui/onboarding.rs` | `onboarding::run(window)` after the library is up: seeding consent (online only), welcome modal once (`welcome_seen`) |
| `ui/launch_notes.rs` | the one panel note per game (port of `launchNotes.ts`), rendered in the detail panel |

Each module owns its CSS sheet under `crates/app/src/styles/` (registered in
`theme.rs::STYLES`) and uses libadwaita dialogs presented over the window;
no second toplevel windows.

## Theming

`omarchy.rs` (core) reads `~/.local/state/omarchy/current/theme/colors.toml`
(the palette) and `shell.toml` (`[font] base-size`), plus `fc-match monospace`
(what `omarchy font current` prints), and emits `theme-changed` whenever the
`current/` directory changes (`omarchy theme set` swaps the `theme` directory
atomically). `theme.rs` (app) turns the palette into a CSS provider that
defines `--om-<key>` custom properties on `:root`, derives the semantic tokens
(`--bg-*`, `--text-*`, `--accent*`, `--danger`, `--success`, `--warning`,
`--info`, `--line-1..4`, `--fill-1..3`, `--scrim`, `--shadow`, `--radius*`)
with `color-mix()`, and sets libadwaita's own variables (`--accent-bg-color`,
`--window-bg-color`, ...) so stock widgets follow too. The provider is
swapped live on every `theme-changed`; `adw::StyleManager` is forced to
dark or light from the palette's mode; the font comes from `shell.toml`'s
base size (px → pt) and the monospace family through `gtk-font-name`.
`style.css` carries Tokyo Night fallbacks for the `--om-*` keys so the app
renders before the first theme lands. No rule outside `theme.rs` names a
colour; each feature module keeps its rules in `styles/<module>.css`.

## Window and desktop integration

- GApplication id `org.exorchy.eXorchy`; on Wayland that is the app id
  Hyprland sees, so the desktop file is `org.exorchy.eXorchy.desktop`
  (`Name=eXorchy`, `Icon=exorchy`, `StartupWMClass=org.exorchy.eXorchy`).
- `packaging/PKGBUILD` installs the binary to `/usr/bin/exorchy` and
  `metadata/`, `torrents/`, `manifest.json`, `previews/` (from
  `crates/core/resources/previews`) to `/usr/lib/exorchy`; icons from
  `packaging/icons` (raster only, 32-512 px) into hicolor.
  `packaging/install-dev.sh` does the same under `~/.local`.
  `packaging/aur/PKGBUILD` is the release package (tag tarball); pushing a
  tag builds it in CI (`.github/workflows/release.yml`,
  `packaging/release/build-package.sh`) and publishes it with
  `packaging/release/install.sh` as a GitHub release. See `docs/RELEASING.md`.
- Emulators spawn with the app's environment; `xdg-open` is used for the
  log folder and for documents when the in-app reader cannot show them.
- `EXORCHY_SNAPSHOT=<png>[:<ms>]` renders the window to a PNG after the
  delay and quits (`snapshot.rs`); `EXORCHY_SNAPSHOT_GAME=<id>` opens that
  game's panel first; `EXORCHY_DUMP_TREE=1` prints the widget tree.

## Collections and what is enabled

All eXo collections are bundled; the four game collections are on at first (language packs off). The `collections`
config key is the enabled set and gates the torrent managers, the config
extraction and every catalogue query (`db::queries::enabled_sql`), so a
disabled pack is invisible. Settings → Collections flips them. Details and
the per-collection differences: `docs/COLLECTIONS.md`.

## Deferred

- HTTP fetch of `manifest.json` (packs are re-hosted only with an app release).
- eXorchy's own poster/emulator pack hosting: the manifest still points at
  Exodium's GitHub release assets (MIT-licensed tooling; eXo content either
  way). `scripts/gen_thumbnails.py` + `gen_previews.py` build drop-in packs.
