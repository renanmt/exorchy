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
