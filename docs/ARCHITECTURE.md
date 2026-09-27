# Exorchy - Architecture

Exorchy is an Omarchy-native (Arch Linux + Hyprland/Wayland) launcher for the
eXo collections: browse the catalogue, stream single games out of the eXo
torrents, and play DOS, Windows 3.x, Windows 9x and ScummVM games, with
previews, theme music and the Media Pack reading room, themed from Omarchy's
current theme. It is derived from Exodium (MIT,
https://github.com/tvollstaedt/exodium, analysed at commit `f7e9fcde`,
v0.15.1) and carries its full feature set; see `docs/DECISIONS.md` for what
was changed and why, and `docs/COLLECTIONS.md` for how each collection is wired
and how enabling/disabling works.

## Stack

| Layer | Technology | Why |
|---|---|---|
| Shell | Tauri v2, window without decorations (Hyprland draws borders, tiles, moves, closes) | runs on Omarchy's WebKitGTK 2.52 / Wayland as-is |
| Frontend | SolidJS + TypeScript + Vite, Ark UI headless, lucide icons | Exodium's proven UI, re-themed |
| Backend | Rust (`src-tauri/`) | Exodium's backend, pruned to Linux + DOS |
| Database | SQLite via `rusqlite` (WAL), pre-built catalogue shipped gzipped | ~11 MB catalogue instead of eXo's 5 GB metadata zip |
| Torrent | `librqbit` 9.0.0-rc.0 (fork pin, see DECISIONS) with selective file downloads | stream one game at a time |
| Emulators | DOSBox Staging 0.83.0 fetched as an emulator pack (or the system binary); DOSBox-X / 86Box / ScummVM packs for the other collections | AUR-only on Arch |
| Theme | Omarchy `colors.toml` + `shell.toml`, watched with inotify | live theme switching |

## Repository layout

```
exorchy/
├── src/                      SolidJS frontend
│   ├── api/tauri.ts          typed invoke() wrappers = the backend contract
│   ├── App.tsx               phase machine (loading | setup | ready), startup order
│   ├── pages/                Setup (first run), Library (Browse + My Library)
│   ├── components/           GameCard, GameRow, GameDetailPanel, SettingsDialog, ...
│   ├── stores/               module-level signals; theme.ts + running.ts are Exorchy's
│   ├── launchNotes.ts        the one panel note (pure function)
│   └── styles/main.css       token layer (--om-* -> semantic tokens), all component CSS
├── src-tauri/
│   ├── src/lib.rs            startup: XDG dirs, logger, DB install/refresh, render path, theme watcher
│   ├── src/commands/         Tauri commands by responsibility (see below)
│   ├── src/launchers/        the emulator seam: mod.rs (spine, process tracking, dispatch), dosbox.rs
│   ├── src/emulators.rs      where DOSBox Staging comes from (pack / system / custom)
│   ├── src/media_sources.rs  the Media Pack torrent joining the shared session
│   ├── src/vhd.rs            differencing VHDs for 86Box (Windows 9x)
│   ├── src/omarchy.rs        theme bridge
│   ├── src/db/               schema, refresh_catalog, queries
│   ├── src/torrent/          librqbit manager, .torrent index, ranged zip reader
│   ├── src/import/           LaunchBox XML parser (build time + import backfill)
│   ├── src/support_files.rs  util.zip payloads (MT-32 ROMs) - queue, watch, extract
│   ├── examples/generate_db.rs   builds metadata/exorchy.db
│   └── resources/previews/eXoDOS/  Tier 0 covers (120 px), bundled
├── metadata/                 bundled XML (gz), configs zips, variant indexes, media index, exorchy.db.gz
├── torrents/                 every eXo .torrent (DOS packs, Win3x, Win9x, ScummVM, Media Pack)
├── manifest.json             content packs + emulator packs per collection
├── packaging/                PKGBUILD, .desktop, install-dev.sh
├── scripts/                  pack-building scripts (thumbnails, previews, LP configs)
└── docs/                     this file, DECISIONS.md, COLLECTIONS.md, HANDOVER.md
```

## Directories at runtime (XDG, not Tauri identifier paths)

| Path | Holds |
|---|---|
| `~/.local/share/exorchy/exorchy.db` | the catalogue + user state (WAL) |
| `~/.local/share/exorchy/launch/` | per-launch DOSBox conf fragments, emptied at startup |
| `~/.local/share/exorchy/librqbit-fastresume/` | `session.json`, `<infohash>.bitv`, `.torrent` copies |
| `~/.local/state/exorchy/logs/exorchy.log` | app log (10 MiB rotation), `dosbox-<id>.log` per launch |
| `~/.local/state/exorchy/accel-attempt` | WebKitGTK render-path crash sentinel |
| `<data_dir>` (user-chosen, default `$HOME`) | the game data dir |
| `<data_dir>/eXoDOS/` (`root_folder`) | the single game root shared by every collection, eXo's own merged layout |
| `<data_dir>/content/` | Exorchy's own: `posters/<col>`, `metadata/<col>`, `emulators/<pack>`, `thumbcache`, `videocache`, `musiccache`, `magazinecache`, `pristine` |

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

## Backend modules (`src-tauri/src/commands`)

| Module | Responsibility |
|---|---|
| `collections.rs` | `CollectionDef`, `COLLECTION_MAP` (GLP, PLP, SLP, eXoDOS, eXoWin3x, eXoWin9x, eXoScummVM), `Launcher` enum, path shapes (`collection_rel_game_dir`, `collection_rel_zip`), family (`collection_base_id`) |
| `paths.rs` | XDG app dirs, resource dir, the cached `root_folder`, `game_root()`, launch-conf dir |
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
| `media.rs` | preview videos and theme music read out of GameData zips by ranged reads, cached, served by the localhost media server |
| `reading.rs` | the Media Pack reading room: magazines, books, catalogues, disk magazines |
| `playlists.rs`, `shell_open.rs` | playlists; xdg-open with exit-code reading |

Other backend modules: `launchers/` (below), `emulators.rs`, `omarchy.rs`,
`support_files.rs`, `db/`, `torrent/`, `import/`.

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

`launchers/dosbox.rs::prepare` (DOSBox Staging): `resolve_game_conf` (catalogue
path, then lang-scoped alternates) → `rewrite_bat_host_paths` (`.\x\y` →
`./x/y` in the game's bats when the target exists) → `patch_dosbox_conf`
(host-path rewrite with trailing-separator and quoting rules, LP overlay mount
via a symlink staging dir, ECE→Staging translation of `[midi]` keys and
DOSBox-X `[ide]` sections) → `emulators::resolve_dosbox_staging` → command
line `dosbox -conf <patched> [-conf options.conf] -conf global_overrides_<id>.conf
[-conf game_<id>.conf]` with cwd `<root>/eXo`. The field knowledge encoded in
these functions is covered by the unit tests copied from Exodium; keep them.

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
set; progress is `get_download_progress` polled at 1 Hz by `stores/downloads.ts`;
completion triggers extraction from inside the poll; `list_active_downloads`
re-arms trackers after a restart. Seeding is opt-in (`seeding_enabled = "1"`
only; off = 1 KB/s upload cap); offline mode creates no session at all.
Invariants that cost Exodium field bugs are listed in `docs/DECISIONS.md`
("Operational invariants").

## Data layer

`metadata/exorchy.db.gz` is built by `pnpm gen-db` (`examples/generate_db.rs`)
from the LaunchBox XML, the `.torrent` file lists (`game_torrent_index`,
`gamedata_torrent_index`, `download_size`), `dosbox.txt` (eXo's emulator
variant per game), `Playlists.xml.gz` (curated playlists) and the LP
`confdirs`/`multilanguage` lists. `db::CATALOG_VERSION` gates
`refresh_catalog`, which updates catalogue columns in place and preserves
`id`, `in_library`, `installed`, `favorited`, `last_played`, `game_config` and
user playlists. Raise `CATALOG_VERSION` BEFORE running `pnpm gen-db`.

Identity: a game is (family, shortcode); rows sharing it are language
variants merged into one card (`queries::primary_row_condition`,
`attach_language_maps` → `available_languages = "EN:2,DE:0"`).

## Frontend

- `App.tsx` phases: `loading` → `setup` (no `data_dir`) → `ready`. Startup
  order in `ready` matters: listeners first (`initTheme`, content-pack
  events, dependency downloads, running games), then `getSetupStatus`,
  `loadNetworkMode`, `loadThumbnailDir`, `initDownloadManager`, seeding
  consent, `refreshInstalledPacks`, `startTransferPolling`, `ensureDosboxStaging`,
  `scanInstalledGames` → `fetchGames` + `resumeDownloads`.
- Stores are module-level signals. `games.ts` owns the paged list and the
  `lastGameLibraryChange` bus; `downloads.ts` owns per-game trackers;
  `contentPacks.ts` the pack jobs; `theme.ts` the palette; `running.ts` the
  running set.
- Covers: Tier 0 bundled previews (120 px), Tier 1 poster pack (400 px), both
  `<thumbnail_key>.jpg`; `thumbnailCandidates` walks Tier 1 → Tier 0 on
  `<img onError>`, loaded near the viewport (`nearViewport.ts`).
- The detail panel shows ONE variant at a time (chip switcher); every effect
  keys on ids, never on the game object.

## Theming

`omarchy.rs` reads `~/.local/state/omarchy/current/theme/colors.toml` (the
palette) and `shell.toml` (`[font] base-size`), plus `fc-match monospace`
(what `omarchy font current` prints), and emits `theme-changed` whenever the
`current/` directory changes (`omarchy theme set` swaps the `theme` directory
atomically). `stores/theme.ts` writes `--om-<key>` custom properties,
`data-mode`, `--font-size-base` and `--font-mono` on `<html>`.
`main.css` defines Tokyo Night fallbacks for every `--om-*` and derives all
semantic tokens (`--bg-*`, `--text-*`, `--accent*`, `--danger`, `--success`,
`--line-n`, `--fill-n`, `--scrim`, ...) with `color-mix()`, so component
rules never carry a literal color. Font sizes are `rem` on
`html { font-size: var(--font-size-base) }`, so Omarchy's base size scales the UI.

## Window and desktop integration

- `tauri.conf.json`: `decorations: false` (Hyprland draws no CSD anyway;
  borders and tiling are the compositor's), no custom title bar, min size
  900×600, app id `org.exorchy.Exorchy`, window title "Exorchy".
- `packaging/exorchy.desktop` + icons; `packaging/PKGBUILD` installs the
  binary to `/usr/bin` and resources to `/usr/lib/exorchy` (Tauri's
  `resource_dir()` on Linux); `packaging/install-dev.sh` does the same under
  `~/.local`.
- The WebKitGTK render path is chosen per GPU/backend at startup (`lib.rs`
  `choose_render_path`): NVIDIA + Wayland keeps DMA-BUF with explicit sync
  off, NVIDIA + X11 disables DMA-BUF, everyone else gets upstream defaults;
  a sentinel file detects a first-paint crash and falls back next start.
- Single instance: a second `exorchy` focuses the running window.
- Emulators spawn with the app's environment; `xdg-open` is used for
  manuals (`open_document`) and the log folder.

## Collections and what is enabled

All eXo collections are bundled; only eXoDOS is on at first. The `collections`
config key is the enabled set and gates the torrent managers, the config
extraction and every catalogue query (`db::queries::enabled_sql`), so a
disabled pack is invisible. Settings → Collections flips them. Details and
the per-collection differences: `docs/COLLECTIONS.md`.

## Splash

`src/components/Splash.tsx` shows `src/assets/splash.jpg` (the app's key art,
its own dark backdrop, not themed) on every start until the app knows what to
render and at least 1.6 s have passed, then fades out; first-run dialogs wait
for it. It is an in-webview overlay, not a second window: a second window
would tile under Hyprland.

## Deferred

- HTTP fetch of `manifest.json` (packs are re-hosted only with an app release).
- Exorchy's own poster/emulator pack hosting: the manifest still points at
  Exodium's GitHub release assets (MIT-licensed tooling; eXo content either
  way). `scripts/gen_thumbnails.py` + `gen_previews.py` build drop-in packs.
- Keyboard-first grid navigation.
