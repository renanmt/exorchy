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

## 2026-09-27 - Headless snapshots use Broadway (or X11) when no frame is painted

`EXORCHY_SNAPSHOT` needs a painted frame; with the monitors off (DPMS) a Wayland window never gets
one and the render finds nothing. Running the same binary on `gtk4-broadwayd` (or XWayland) keeps
the frame clock ticking without a display, so visual checks work unattended. The `PORTING.md`
recipe says so; the app is single-instance, so a leftover instance must be killed first.
