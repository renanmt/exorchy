# Exorchy - Decision Record

Append-only. One entry per decision, newest at the bottom. Each entry says what was decided,
why, and what it rules out. `ARCHITECTURE.md` describes the resulting shape; this file explains it.

## 2026-09-26 - Exorchy is a Linux/Omarchy-only fork of Exodium's architecture, not a rewrite

Exodium (MIT, https://github.com/tvollstaedt/exodium, analysed at commit `f7e9fcde`, v0.15.1)
already solves the hard problems: bundled LaunchBox metadata in SQLite, selective torrent streaming
with librqbit, eXo config translation for DOSBox Staging, save-preserving uninstall. Its stack is
Tauri v2 + Rust + SolidJS, which runs on Omarchy's WebKitGTK 2.52 / Wayland stack without changes.
Rewriting in GTK4/Qt/Quickshell would cost the whole backend for a UI toolkit gain we do not need:
Exorchy's theming is CSS custom properties fed from Omarchy's palette, which a webview does better
than a native toolkit.

Ruled out: cross-platform builds (macOS/Windows code paths are dropped, not carried), the custom
title bar (Hyprland manages windows), the auto-updater (Arch packaging updates the app).

## 2026-09-26 - Scope v0.1 is eXoDOS; Win3x/Win9x/ScummVM/Media stay behind a registry

The user's requirement: DOS now, eXoWin3x and eXoWin9x later. Every place Exodium branches on a
collection is folded into two seams: the `collections` registry (static `CollectionDef` table with
family, launcher kind, torrent name, metadata file, path shapes) and the `launchers` trait (prepare
launch dir, build command line, spawn). v0.1 registers `exodos` only; the German/Spanish/Polish
language packs, eXoWin3x and eXoWin9x are documented as future registry entries with their known
differences (see `docs/FUTURE-COLLECTIONS.md`). Nothing DOS-specific may live outside
`launchers/dosbox_staging.rs` and the `exodos` registry entry.

## 2026-09-26 - Theme source is Omarchy's current theme directory, read directly and watched

Omarchy writes the active palette to `~/.local/state/omarchy/current/theme/colors.toml` and swaps
the whole `theme` directory atomically on `omarchy theme set` (see `omarchy-theme-set`). Exorchy
reads that file at startup, watches `~/.local/state/omarchy/current/` with inotify (`notify`
crate), re-reads on change and pushes the parsed palette to the webview as a Tauri event; the
frontend maps palette keys to CSS custom properties on `:root`. Font comes from `fc-match monospace`
(what `omarchy font current` does) and base size from `shell.toml [font] base-size`.

Why not a `themed/*.tpl` template rendered by `omarchy-theme-set-templates`: templates only
regenerate when the user runs the theme setter, live on the theme dir anyway, and would make Exorchy
depend on being installed before the theme was applied. Reading `colors.toml` works on a theme set
before Exorchy was installed and needs no hook. A built-in fallback palette (Tokyo Night values)
covers machines without Omarchy so the app still runs elsewhere on Linux.

## 2026-09-26 - Crate versions follow Exodium's lockfile for copied code

Copied Rust modules are pinned to the versions Exodium built against (`rusqlite 0.31`, `quick-xml
0.36`, `zip 2`, `reqwest 0.12`, `librqbit 9.0.0-rc.0` with the `[patch.crates-io]` fork commit
`ad57d887` that fixes the `update_only_files` lock order and the sparse-flag bug). Upgrading APIs
while porting doubles the surface for mistakes. New crates (`notify 8`, `toml 1`) are added only
for Exorchy-specific code. Upgrades are a separate, tested step.

## 2026-09-26 - DOSBox Staging comes from an emulator pack, with the system binary as override

`dosbox-staging` is AUR-only on Arch (0.83.0), so Omarchy users cannot be assumed to have it.
Exorchy downloads the official Linux x86_64 release tarball (v0.83.0,
`dosbox-staging-linux-x86_64-v0.83.0.tar.xz`, sha256
`d3a94f7f1c3e68a47ec88d61145506c7904452adb0c9c5928cb8cfe2331d6c5c`) into
`~/.local/share/exorchy/emulators/dosbox-staging/<version>/` and runs its `dosbox` binary in place:
the tarball is self-contained (`lib/`, `plugins/`, `resources/shaders`, `resources/shader-presets`
next to the binary), so no shader copying into DOSBox's config dir is needed. A `dosbox-staging`
found on `PATH` (AUR install) is preferred when the user enables "use system emulator" in Settings,
otherwise the pack wins so behaviour is reproducible. This is the same "emulator pack" mechanism
Win9x will need for DOSBox-X and 86Box, which is why it is a registry of packs, not a special case.

## 2026-09-26 - XDG paths, not Tauri identifier paths

Data lives in `~/.local/share/exorchy/`, config in `~/.config/exorchy/`, logs/state in
`~/.local/state/exorchy/`, caches in `~/.cache/exorchy/`. Tauri's `app_data_dir()` would give
`~/.local/share/<bundle identifier>`; Omarchy tools use plain XDG names and users look there. The
games root (`data_dir`) is user-chosen and separate, as in Exodium.

## 2026-09-26 - The language packs ship in v0.1; media previews, music and the PDF reader do not

"DOS" means the four DOS packs: eXoDOS plus its German, Spanish and Polish language packs are one
family with one game tree, and Exodium's LP machinery (overlay mount, patch variants needing the
English base, family-scoped shortcodes) is covered by the tests that were copied. Leaving the LPs
out would have meant removing tested code paths, not adding untested ones. Registry order stays
LPs-first (title matching prefers LP rows).

Preview videos, theme music and the in-app PDF manual reader were removed instead: they need the
localhost media server, the GStreamer plugin set (not installed on Omarchy by default) and a
pinned pdfjs build, all orthogonal to launching games. `torrent/zip_range.rs` stays because it is
what brings them back (v0.2). Manuals open in the system PDF viewer through `open_document`.

## 2026-09-26 - Launch pipeline restructured: `launchers/` spine + one module per emulator kind

Exodium's `launch_game` was one 250-line function mixing dispatch, DOS specifics and fragment
writing. Exorchy keeps the spine in `commands/games.rs::launch_game` (op-lock, DB read, guards,
`extract_before_launch`, spawn) and moves everything DOSBox-specific into
`launchers/dosbox.rs::prepare`, dispatched by `launchers::prepare(kind, ctx)` on the collection's
`Launcher`. `Launcher::Win9x` is a reserved variant that errors politely; the process machinery
(`running_games`, `running_pids`, `game_op_lock`, `spawn_emulator_and_track`) lives in
`launchers/mod.rs` because every kind shares it. A trait object was not used: async dispatch over
an enum with two arms reads better and the compiler enforces exhaustiveness when a kind is added.

## 2026-09-26 - Unknown collection sources are their own family

Exodium's `collection_base_id` answered "eXoDOS" for any id it did not know, which made the Rust
side and the SQL `family_expr` (COALESCE to the source itself) disagree once eXoWin3x/eXoWin9x
left the registry. Exorchy returns the source itself for unknown ids, matching the SQL, so a row
from a not-yet-registered collection never merges with a DOS game.

## 2026-09-26 - Content packs accept `.tar.xz` and version-suffixed wrapper directories

The DOSBox Staging release tarball is `.tar.xz` wrapped in `dosbox-staging-linux-x86_64-<ver>/`.
`content_packs::do_install` picks the decoder from the URL suffix, and `unwrapped_source` also
strips a lone wrapper whose name is `<install leaf>-...`, so the binary lands at
`content/emulators/dosbox-staging/dosbox`. Exodium's rule (wrapper must equal the leaf) is kept
for every other pack.

## 2026-09-27 - Full Exodium feature set, all collections bundled, only eXoDOS enabled by default

The user asked for everything Exodium gives. The frontend was re-baselined on Exodium's full UI
(media player, reading room, PDF manuals, Win9x/ScummVM panels) with the Exorchy changes re-applied
on top (theme store, no title bar, no updater, no legacy layout migration, Setup with
`setup_fresh`, Settings → Emulators/Appearance/Collections, splash). The backend modules
`media.rs`, `reading.rs`, `win9x.rs`, `scummvm.rs`, `vhd.rs`, `media_sources.rs` and the reading
tables came back whole; the Win9x and ScummVM launchers plug into `launchers::prepare` through
`LaunchOutcome::Launched` because they spawn themselves (they call the shared
`spawn_emulator_and_track`). The earlier "DOS-only v0.1" entries above describe an intermediate
state and are kept as history.

The user also asked that the German, Spanish and Polish packs not show unless enabled one by one.
That became a general rule: every collection is a switch in Settings → Collections, off by default
except eXoDOS, and the enabled set filters every catalogue query (`db::queries::enabled_sql`) so a
disabled pack contributes no card, no language chip and no variant. Exodium's
`enable_new_collections` (auto-enabling packs shipped later) was dropped for the same reason.
`setup_from_local` enables the packs present in the imported tree, because those games are
already on disk.

## 2026-09-27 - Splash screen on every start, in the webview

The key art (`src/assets/splash.jpg`, from the user) is shown as an overlay until the app knows
whether to render setup or the library, for at least 1.6 s, then fades. Not a second Tauri
window: Hyprland would tile it. The overlay keeps its own dark backdrop instead of the theme's
because the artwork's colours are fixed.

## 2026-09-27 - Credits are part of the product

README carries a tribute to the eXoDOS project (in Exorchy's own words, following Exodium's
example) and a thank-you to Thomas Vollstädt with support links; `ACKNOWLEDGEMENTS.md` has the full
list; Settings → About repeats the credits; `LICENSE` keeps his copyright notice. Code comments
that reference Exodium by URL are left untouched by the rename.

## 2026-09-27 - Game confs resolve case-insensitively (first field bug)

eXoWin3x's LaunchBox XML spells the pack folder `eXoWin3X`; the torrent writes `eXoWin3x`. On the
user's ext4 tree "SimCity 2000: CD Collection" failed with "Game config not found" although the
conf was there. Exodium never saw it because its Win3x testing ran on case-insensitive filesystems.
`launchers::dosbox::resolve_game_conf` now walks the catalogue path component by component with a
case-insensitive match (`resolve_rel_ignoring_case`) before the language-pack probes, and returns the
on-disk spelling. The catalogue is left as eXo wrote it: `application_path` is the refresh key.

## 2026-09-27 - Emulator packs get `SDL_AUDIODRIVER=pulseaudio`

eXoScummVM games had no sound. The pack's own SDL picks the PipeWire backend and fails
("Could not open audio device: Pipewire: Failed to connect stream!", `dosbox-<id>.log`), while
Omarchy's PipeWire always exposes a PulseAudio server (`$XDG_RUNTIME_DIR/pulse/native`).
`launchers::prefer_pulse_audio_backend` sets `SDL_AUDIODRIVER=pulseaudio` for every emulator spawn
when that socket (or `PULSE_SERVER`) exists and the variable is not already set - a user's own
choice always wins. Every emulator, not only ScummVM: DOSBox-X and 86Box packs bundle SDL the same
way, and the DOSBox Staging tarball loses nothing by talking to pipewire-pulse.

## 2026-09-27 - The WebKit web process is ended with SIGKILL before exit on NVIDIA

Every close of the app produced an Omarchy "process crashed" notification. The core dump: thread
`SkiaGPUWorker` of `WebKitWebProcess` segfaults inside `libnvidia-eglcore.so.610.57.04` under
`__call_tls_dtors`, i.e. during the web process's own orderly exit, with the main thread also inside
the NVIDIA GL stack. That is the driver tearing down a thread-local GL context, a WebKitGTK 2.52 +
NVIDIA 610 bug on the accelerated render path Exorchy keeps for performance (§17 in Exodium's
notes). No app state is involved and nothing is lost. `lib.rs::end_webkit_web_processes_hard` runs on
the window's `CloseRequested`, walks `/proc` for `WebKitWebProces` descendants (bwrap sits in
between) and SIGKILLs them, so no teardown runs and no core is dumped. Gated on the proprietary
driver being loaded; elsewhere the normal exit is clean. Switching to the software render path
would avoid it too, at 10 fps.

## 2026-09-27 - Native rewrite in Rust with GTK4 + libadwaita, replacing the webview

The Tauri build worked, but it was a web page in a WebKitGTK window on a desktop where everything
else is native. Three things tipped it: Omarchy users expect a native window (theme, font, Hyprland
tiling, keyboard-first), the WebKitGTK + NVIDIA render path crashed on every close and needed the
SIGKILL workaround above, and GTK 4.22's CSS supports custom properties and `color-mix()`, so the
token layer (`--om-*` → semantic tokens) ports one to one and the "no colour literals" rule keeps
holding. GTK4 and libadwaita are already installed on Omarchy (no WebKit, no node, no pnpm in the
build), and the backend is kept whole: `crates/core` is Exodium's tested backend, `crates/app` the
new shell. Cost: the UI is rewritten module by module against the web UI as reference
(`docs/PORTING.md`); the web UI stays under `legacy/webui` until parity.

Ruled out: C++/Qt as omakade did (would have meant rewriting the 26k-line tested backend or
bridging it through FFI); iced or egui (no GStreamer video, no PDF rendering, no native
accessibility, their own widget look instead of Omarchy's GTK look).

## 2026-09-27 - A host shim instead of a full de-Tauri refactor

The backend's 103 command functions take `State<'_, T>` and `AppHandle` arguments and emit events.
`crates/core/src/host.rs` reimplements those four things (`AppHandle` with `manage`/`state`/
`emit`/`subscribe`, `State<'a, T>` as an `Arc<T>` with a phantom lifetime, empty `Manager` /
`Emitter` traits, `async_runtime` over one process-wide tokio runtime), so every command kept its
signature and its tests. `AppHandle::state()` returns `State<'static, T>`, which a `State<'_, T>`
parameter accepts. Ruled out: rewriting the commands as plain functions over a context struct
(touches every call site and every test for no behaviour gain) and keeping a `tauri` dependency
without the webview (still pulls the GTK3/WebKit stack in).

## 2026-09-27 - Cover textures are pre-scaled to the card size

`gtk::GridView` sizes its cells from each child's natural size, and a `gtk::Picture`'s natural
size is its texture's, so a 400 px poster in a 180 px card made the grid grow and stutter. Covers
are therefore decoded and cover-cropped to the exact card pixel size (or fit-scaled for the panel)
with the `image` crate on tokio's blocking pool, then uploaded as a `gdk::Texture` and cached by
(path, size). Side effect that matters: GPU memory holds a card's worth of pixels per cover, not a
poster's. Ruled out: `Picture::set_can_shrink` alone (still uploads the full texture) and
`content-fit` (fixes the layout, not the memory).

## 2026-09-27 - The GridView is the ScrolledWindow's direct child

GridView only virtualises when its scrollable parent gives it the viewport: wrapped in a
`gtk::Stack` (or any box) inside the ScrolledWindow it was allocated its full natural height and
instantiated every row, which is 11,000 cards. The Browse tab therefore puts the GridView (and the
ListView) directly into their ScrolledWindows and switches the ScrolledWindows, not the views.
Everything that must scroll with the grid (the collection shelf, the jump bar) is laid out around
it, not inside the scroller.

## 2026-09-27 - EXORCHY_SNAPSHOT replaces the headless Chromium smoke script

The web UI's `scripts/ui-smoke.mjs` rendered pages in headless Chromium against mocked IPC. There
is no browser now, and `grim` hangs on this compositor, so the app renders itself:
`EXORCHY_SNAPSHOT=<png>[:<ms>]` snapshots the window with `gtk::WidgetPaintable` after the delay
and quits; `EXORCHY_SNAPSHOT_GAME=<id>` opens a detail panel first; `EXORCHY_DUMP_TREE=1` prints
the widget tree. It runs against the real backend on an isolated XDG profile
(`docs/PORTING.md`), so it exercises the real startup order too.

## 2026-09-27 - The web UI stayed under `legacy/webui` until parity, then was deleted

The SolidJS sources are the specification for the port: every feature, string, invariant and
test (`stallDetector.test.ts`, `launchNotes.test.ts`, ...) lives there. They are moved out of the
build (no `package.json` at the root, no node in the PKGBUILD) but kept in the tree so a module
port can diff against them. They are deleted, in one commit, when the last feature module lands;
nothing new is written there.

## 2026-09-27 - The name is eXorchy

Spelled "eXorchy" (capital X, as in eXoDOS and Exodium's "eXo" heritage) in every user-facing
string: window title, desktop entry, README, About, backend messages. Identifiers stay lowercase
`exorchy`: the binary, the pacman package, the crate names (`exorchy`, `exorchy-core`), the XDG
directories, the resource dir, the icon name. The GApplication id is `org.exorchy.eXorchy`, which
is also the Wayland app id and the desktop file name so the launcher and Hyprland match the
window.

## 2026-09-27 - Settings is a sidebar + stack dialog with full-width pages

Eight sections do not fit libadwaita's view switcher, so `settings.rs` mirrors the web layout:
an `adw::Dialog` with a `gtk::ListBox` nav and a `gtk::Stack` of pages. The pages are plain
scrolled boxes of `adw::PreferencesGroup`s rather than `adw::PreferencesPage`, whose clamp caps
content at ~600 px and would squash the storage list and the pack rows. The storage usage bar is a
400-column homogeneous `gtk::Grid` so every segment colour stays a CSS token.

## 2026-09-27 - The document viewer is continuous-scroll poppler on its own thread

`pdf.rs` keeps the `poppler::Document` on one dedicated thread (mpsc requests, oneshot replies) and
renders pages to cairo surfaces that become `gdk::MemoryTexture`s; the view is a continuous scroll
of per-page placeholders (an `Overlay` per page so a HiDPI 2× bitmap lays out at logical size),
with a 6-page LRU budget, nearest-first scheduling and a generation counter on zoom. Manuals use the
same viewer; HTML manuals are shown as text (no webview in the GTK shell).

## 2026-09-27 - The Reading Room grid is a ListView of sections, each a FlowBox

`GtkGridView` has no section headers, and the reading room's sections (publication names) are what
the jump bar targets. A `ListView` whose rows are section boxes holding a `FlowBox` of cards keeps
virtualisation per section and gives real headers. The game grid keeps `GridView` (one flat list,
jump bar scrolls to positions).

## 2026-09-27 - Launch notes are dismissed per kind; reopening the same game re-notifies

`dismissed_notes` (config, comma list) stores the note kind, as the web did, so a dismissed
"tuned for ECE" note stays dismissed for every ECE game. `DetailPanel::show` fires `on_shown` again
when the panel was closed in between even for the same game: the media and note listeners tear down
on close and must come back.

## 2026-09-27 - Previews start muted; cached media plays by path

The hero video defaults to muted (`preview_muted` unset) because a tiling desktop opens the panel
often and the web's unmuted default was a surprise in a shared room; a stored preference wins.
Cached videos and tracks are handed to GTK's GStreamer-backed `MediaFile` by file path, so the
localhost media server the webview needed is not used.

## 2026-09-27 - The four game collections are on by default; the layout adapts to the tile

The user asked (2026-09-27) for eXoDOS, eXoWin3x, eXoWin9x and eXoScummVM to be enabled on a fresh
install (`db::queries::DEFAULT_COLLECTIONS`); the German, Spanish and Polish language packs stay
off until switched on. This supersedes the earlier "only eXoDOS" default; the enabled-set filter
and the per-collection switches are unchanged.

The same feedback covered Omarchy tiles: a half-width or quarter tile cut the detail panel off and
the window refused to shrink. The library page is now an `adw::BreakpointBin` whose only demand is
360×300: the detail panel is the end sidebar of an `adw::OverlaySplitView` (beside the grid on a
wide window, an overlay with a scrim below 1100 sp), the filter rows are `adw::WrapBox`es, the brand
hides below 760 sp, the setup card is clamped and scrolls, the splash scales whole (`Contain`) and
the window carries no hard minimum size.

## 2026-09-27 - Headless snapshots use Broadway (or X11) when no frame is painted

`EXORCHY_SNAPSHOT` needs a painted frame; with the monitors off (DPMS) a Wayland window never gets
one and the render finds nothing. Running the same binary on `gtk4-broadwayd` (or XWayland) keeps
the frame clock ticking without a display, so visual checks work unattended. The `PORTING.md`
recipe says so; the app is single-instance, so a leftover instance must be killed first.

## 2026-09-27 - Mount targets in confs and bats resolve case-insensitively (second field bug)

After Dark 3.2 (eXoWin3x) exited 0.3 s after launch: its conf mounts `.\eXoWin3x\Adark3` and
its CD image under it, but the archive unpacks the folder as `adark3`. The host-path rewriter
only replaced a `.\` path when the target existed with that exact spelling, so the Windows
path survived untouched and DOSBox Staging on ext4 found neither the drive nor the image
("MOUNT: Image file not found"), then ran into the autoexec's `exit`. The first field bug
(2026-09-27, above) fixed the same mismatch for the conf file's own path; `patch_dosbox_conf`
and `rewrite_bat_host_paths` now fall back to `resolve_rel_ignoring_case` and write the on-disk
spelling. Windows DOSBox never saw this because NTFS is case-insensitive.

## 2026-09-28 - New key art, icon and an ASCII wordmark drawn in theme colours

The user supplied new artwork in `media/` (splash, icon, logo PNG and `logo.txt`). The splash
is `media/splash.png` re-encoded as `assets/splash.jpg`. The icon is `media/icon.png` with the
black surround outside its rounded frame made transparent (flood fill from the corners), cut
square and scaled to the `packaging/icons` sizes and `assets/exorchy.png` (256 px, the runtime
copy). The old SVG is gone: a scalable icon would win the hicolor lookup over the new raster
ones, so `install-dev.sh` deletes one a previous install left behind. The toolbar brand and the
About header use `logo.txt` instead of the logo PNG, because the PNG's colours are fixed and the
text art can follow the Omarchy palette: it is drawn as rectangles rather than typeset, since
glyph metrics and line height would open gaps between the half blocks.


## 2026-09-28 - About names the author; Thomas's credit row carries the invitation to support him

Settings → About gains an Author row (Renan Tonheiro). Thomas Vollstädt's credit stays in
About, as the hard rule requires: the Exodium row credits him as its creator and invites the
user to visit his GitHub page, both to support his work and to get Exodium on other operating
systems (eXorchy is Linux/Omarchy only). That row replaces the separate "Support Thomas" row
(GitHub Sponsors, Ko-fi), which duplicated its link; README and ACKNOWLEDGEMENTS.md keep the
sponsor links. The eXoDOS credit is unchanged.

## 2026-09-28 - Settings stacks its rows in a narrow tile

In a ~520 px Hyprland tile the Settings dialog is ~460 px wide, but its minimum was 509-581 px
(190 px sidebar, a 170 px label column per row, button groups and the Storage breakdown lines
that could not shrink), so the right edge was clipped and hints wrapped a character per line. A
720 sp breakpoint on the dialog now turns the sidebar into icons (names in tooltips) and stacks
every row: label, then value and hint, then the action, which is a `WrapBox`. Widgets opt in with
`widgets::follow_narrow`; the state is per dialog (`begin_dialog`) because a closing dialog's
breakpoint unapplies after the next one has opened.

## 2026-09-28 - Settings is a page in place of the library, not a dialog

The Settings dialog fought the tiling window manager: in a tile it was narrower than the window
and clipped, and a floating sheet inside a tiled window is the pattern Omarchy avoids anyway. It
is now a full-body page in the window's stack (`window::show_page` / `close_page`), opened from
the gear button or Ctrl+,, closed with the back arrow or Esc; the library stays in the stack
underneath, so filters, scroll position and the detail panel survive. Not a fourth tab: Settings
is not browsed, and the tab row stays short in narrow tiles. Inside, an `AdwNavigationSplitView`
shows the section list beside the section, collapsing below 720 sp to list → section with back
arrows; this replaces the icon-only sidebar of the entry above, while rows still stack when
narrow. The page is built on open and dropped on close, as the dialog was, so the pages' timers
keep their "while Settings is open" lifetime (`Ctx::is_alive`, also cleared on unrealize).
Confirmations stay `adw::AlertDialog`s. Content is clamped to 960 px so a full-screen page does
not stretch rows across a wide monitor.

## 2026-09-28 - The toolbar stacks in a narrow tile instead of dropping the logo

Below 760 sp the toolbar used to hide the wordmark and wrap control by control, which left the
gear alone on a line. It is now two groups, wordmark + tabs and search + connection badge +
feature controls + gear, side by side when there is room and stacked (one `orientation`
setter) in a narrow tile; the tabs wrap under the wordmark in the narrowest tiles. The
connection badge became a button with an icon, visible when idle, that opens Transfers.

## 2026-09-28 - Transfers page, and a per-torrent listing in the backend

The badge only had session totals. `DownloadManager::session_torrents` reads librqbit's
`ManagedTorrent::stats()` for every torrent in the shared session (rates, live peers, uploaded,
progress of the selected files) and `games::get_session_torrents` labels each with the
collection whose manager owns it; `stats()` runs on `spawn_blocking`. It copies each torrent's
file-progress vector, so it is polled only while the page is open (1 s). There is no download
queue to show for games (selected files all download at once), so "Queued" lists what really
waits: preview and theme fetches beyond the three media slots. The page is a full-body page
like Settings, reusing `window::show_page`. Rows are named from the bundled `.torrent` they came
from (every eXoDOS torrent, the Media Pack too, is called "eXoDOS" inside), and each torrent's
rates are derived from its uploaded / on-disk byte counters between polls: librqbit's
per-torrent speed estimates read zero while the session total moves.

## 2026-09-28 - Hidden titles and the adult filter are a query predicate, not a UI filter

Hiding had to reach every list (Browse, search, shelves, genres, music shuffle) and counts, so
it is `visible_sql` beside `enabled_sql` in `db::queries`, not a filter over fetched pages
(which would break paging and section keys). Hidden titles live in a `hidden_games` table keyed
by game id (catalog refreshes update rows in place and keep ids, like playlists); a hide covers
the shortcode group. A hidden installed title still matches a name search so it can be played,
as asked. Adult titles are eXo's `Adult` genre token; they are off by default, hidden even when
installed, and their genre is not offered. My Library says how many installed games are
hidden, with a link to Settings → Hidden titles. "Remove from Recently played" clears
`last_played` for the whole group, or another variant would take the card's place.

## 2026-09-28 - Adult means eXo's age rating "A - Adult" or the Adult genre (catalogue 17)

The Adult genre tags only 23 titles, all eXoWin3x, while eXo's LaunchBox `<Rating>` rates 133
more "A - Adult" (DOS, ScummVM, Win3x, Win9x) whose genres say nothing, so adult titles kept
showing with the filter off. The importer now keeps `<Rating>` as `age_rating` (catalogue 17,
new column migrated in place, refreshed at startup) and `adult_sql` checks either marker.
"M - Mature" (violence, e.g. Doom) stays visible. `age_rating` is compared through `COALESCE`:
a bare `= 'A - Adult'` is NULL for unrated rows and `NOT adult` would hide them all.

## 2026-09-28 - The search box searches the tab it is on

In My Library the shared search box used to refetch Browse, which is not on screen. There it now
replaces the shelves with one "Installed" shelf from `search_library`: installed games whose
title (any variant) matches, hidden ones included so they stay findable and playable. Not
favourites or playlists: membership in eXo's curated playlists would pull in thousands of
games that are not on disk. Browse and the Reading Room keep their own search. The search runs once typing pauses (entry `search-delay` 100 ms plus
`SEARCH_PAUSE` 300 ms) and only on the tab in view; Browse and the Reading Room catch up in
`set_tab` when shown (Browse only if its last fetch used another query). Before, every keystroke
refetched Browse, refiltered the Reading Room and searched the shelves, all at once.

## 2026-09-28 - Start tab and a user background image

Settings → General → "Open in My Library" stores `start_tab` (`library` / `browse`); the
library reads it first thing in its startup sequence so Browse does not flash. Settings →
Appearance takes a background image: the file is copied into the data folder under a new name
per choice (moving the original cannot lose it, and GTK's texture cache cannot serve the old
one), shown by a `gtk::Picture` under the page stack at the chosen opacity over the theme's
background colour. Only the full-width surfaces (toolbar, filter row, Settings, Transfers) turn
translucent, through a `has-backdrop` window class; cards, panels, entries and dialogs stay
solid so text keeps its contrast. The overlay measures the pages, not the picture, so a large
image never forces the window's size.

## 2026-09-28 - The detail panel's visibility is re-applied when the split view (un)collapses

`AdwOverlaySplitView` restores its own `show-sidebar` when it crosses the 1100 sp breakpoint, so
a window started in a narrow tile and then maximised showed an empty detail panel. On every
`collapsed` change the library sets `show-sidebar` to "a game is open", on idle so it lands
after the split view's own change.

## 2026-09-28 - The source checkout is used only by a binary running from it; AUR packaging

`metadata/`, `torrents/` and `manifest.json` were looked up in the checkout the binary was
built from (`CARGO_MANIFEST_DIR`) before `/usr/lib/exorchy`. A packaged binary keeps that path,
and an AUR helper keeps its build tree in a cache, so an installed eXorchy would have read its
resources from a stale build directory. `dev_project_root()` now answers the checkout only while
the running executable sits inside it (`cargo run`, tests, generate_db); otherwise a path that
does not exist, so every lookup falls through to the installed resources. The AUR package is
`packaging/aur/PKGBUILD` (GitHub release tarball, `cargo fetch --locked` in `prepare()`,
`--frozen` build, core tests in `check()`); the steps are in `docs/RELEASING.md`.

## 2026-09-28 - Distribution: a prebuilt pacman package on GitHub releases, installed by a script

AUR registrations are closed, and a script that compiles from source would need Rust and minutes
of building on every machine. Each tag instead builds the package from `packaging/aur/PKGBUILD`
in a clean Arch container (GitHub Actions) and attaches it, its SHA-256 and `install.sh` to the
release. The script only downloads, verifies and hands the file to `pacman -U`, so pacman
resolves the dependencies from the Arch repos, tracks every file and removes it cleanly; the
build is reproducible and not made on a personal machine. Assets keep stable names so
`releases/latest/download/…` always resolves. Prompts read `/dev/tty` because a piped script's
stdin is the script, and the body runs from `main "$@"` on its last line so a truncated
download runs nothing. A pacman repository (updates through `pacman -Syu`) is the next step if
wanted; the AUR package is ready for when registration reopens.

## 2026-09-29 - Square corners and 2 px outlines, like Omarchy's windows

Omarchy's Hyprland draws windows with rounding 0 and 2 px borders; eXorchy's 8 / 12 px radii
and pill shapes looked foreign inside them. The radius tokens (`--radius`, `--radius-lg`, the
new `--radius-pill`) are 0 and every hard-coded radius goes through them, so one token brings
roundness back. Outlines (cards, panels, entries, buttons, lists) use `--border` (2 px); row
dividers stay 1 px so lists do not get heavy. libadwaita rounds its own widgets (buttons,
boxed lists, popovers, dialogs, switches, toasts), so style.css sets those square too; radio
buttons stay round. The Settings boxed-list rules now match (`list`, not `listbox`: GtkListBox's
CSS node is `list`), so the groups take the app's own flat, outlined look.

## 2026-09-29 - A favourite is its own signal, not a library change

Starring a game sent `bus::notify_library_changed`, whose listeners include the detail panel's
full refresh (re-read the row, variants, metadata, cover, media): the panel flickered on every
star (reported on Reddit). Stars now send `notify_favorite_changed(id, favorited)`: the library
refreshes its grid flags and shelves as before, and the panel only repaints its action bar when
it shows that game. `library_changed` stays for what really changes a row's state (downloads,
installs, uninstalls, playlists).

## 2026-09-29 - Self-update: check GitHub releases, install in a terminal, restart

eXorchy checks `releases/latest` of the GitHub repository shortly after start and every six
hours (switchable, skipped offline) and shows a banner above the toolbar. Update does not
install from inside the app: installing is `sudo pacman -U`, and Omarchy ships no graphical
polkit agent to ask for the password, so the app opens Omarchy's floating terminal (the one its
own Install menu uses; `xdg-terminal-exec` otherwise) running that exact release's `install.sh
--version vX.Y.Z`, which downloads, verifies and installs, then waits for the old process to
exit and starts eXorchy again through `uwsm-app` (whether or not the install succeeded). The app
quits right after opening the terminal. Only the pacman-installed `/usr/bin/exorchy` offers
Update; other copies get the notice and a hint, since the script would put a second install
beside them. The tag goes into a shell command, so only `vN.N.N` with digits is accepted.

## 2026-09-29 - Release notes come from a hand-written CHANGELOG.md

GitHub's generated notes list pull-request titles, which here were branch names ("Merge pull
request #6 from renanmt/fix-favorite-flicker"), and the app's "What's new" opens those notes.
`CHANGELOG.md` (Keep a Changelog shape, written for users) now holds one section per version;
the release workflow publishes the tag's section above the install instructions
(`packaging/release/changelog-section.sh`) and fails before building when the section is
missing, so a release cannot go out without notes.

