# Collections in eXorchy

Every eXo collection Exodium supports is built into eXorchy. Each one is a row
in `COLLECTION_MAP` (`crates/core/src/commands/collections.rs`), its bundled
files under `metadata/`, `torrents/` and `crates/core/resources/previews/<col>/`,
its shelf art under `crates/app/assets/collections/<col>.jpg`, and a manifest
entry in `manifest.json`. Only eXoDOS is enabled on a fresh install; the rest
are switches in Settings → Collections.

## Enabling and hiding

The `collections` config key (comma list) is the enabled set. It decides:

- which torrents get a `DownloadManager` (`setup::init_download_manager`),
- which bundled config zips are laid down in the game root,
- which rows the catalogue shows. `db::queries::set_enabled_collections`
  mirrors the key into a process-wide set that `enabled_sql(alias)` turns
  into `torrent_source IN (...)`, applied in `build_where_clause` (top level
  and inside the per-variant EXISTS), `attach_language_maps` (language chips),
  `fetch_game_variants`, `get_genres`, `fetch_installed_games` and
  `fetch_recently_played`. A disabled language pack therefore contributes no
  chip, no variant and no page; a disabled Win9x pack no card. eXoDOS cannot
  be switched off (`set_enabled_collections` re-inserts it).
- The set is loaded at startup (`bootstrap()`), by `set_config("collections", …)`,
  by `setup_fresh` (writes `eXoDOS`) and by `setup_from_local`, which enables
  every pack whose tree exists in the imported eXo folder
  (`collections_present_on_disk`).

UI: Settings → Collections (`crates/app/src/ui/settings.rs`) writes the key,
re-initialises the download managers, then `bus::notify_collections_changed()`
makes the library reload cover dirs and packs, rescan and refetch
(`window::reinit_library()` when the enabled set changed). The collection
shelf in Browse (`library.rs::load_collections`) shows enabled collections
only and hides itself with a single one.

Exodium appended newly shipped packs to the key on every catalogue refresh
(`enable_new_collections`). eXorchy does not: a new pack must never switch
itself on.

## Per-collection notes (what differs from DOS)

| | eXoDOS + GLP/SLP/PLP | eXoWin3x | eXoWin9x | eXoScummVM | Media Pack (reading room) |
|---|---|---|---|---|---|
| launcher | `Launcher::DosBox` → `launchers/dosbox.rs` | `DosBox` (same pipeline, plus `[ide]` translation) | `Launcher::Win9x` → `commands/win9x.rs` | `Launcher::ScummVm` → `commands/scummvm.rs` | not a collection: `commands/reading.rs`, disk magazines run under DOSBox Staging |
| emulator | DOSBox Staging pack or system binary (`emulators.rs`) | same | DOSBox-X / 86Box: PATH (with `cap_net_raw` preferred for DOSBox-X), else emulator packs `dosbox-x` / `86box` (Linux AppImages from the manifest), else Flatpak | per-version ScummVM packs (`scummvm-<v>`), PATH/Flatpak only when no pack exists for this platform | DOSBox Staging |
| game path | `eXo/eXoDOS[/!lang]/<sc>` | `eXo/eXoWin3x/<sc>` (catalogue spells it `eXoWin3X`; resolved case-insensitively) | `eXo/eXoWin9x/<year>/<Title (Year)>` (title dir = shortcode) | `eXo/eXoScummVM/<Title (Platform)>` | `eXo/Magazines/...` |
| support payload | `util/util.zip` → `eXo/mt32` (`DOS_SUPPORT`) | same (MT-32 from the DOS pack) | `util/utilWin9x.zip` → parent VHDs + eXo's emulator builds (`WIN9X_SUPPORT`, +8 GiB preflight) | `util/utilSVM.zip` → MT-32 (`SCUMMVM_SUPPORT`) | none |
| conf translation | host paths, MIDI keys, `[ide]` | same | `play.conf` verbatim plus host paths and zip-mount extraction (`<x>.exorchy_mount/`) | none (command line from `scummvm.txt`) | `run.bat` from a template |
| saves | game dir, `!save/<sc>` backups | same | on the game's own VHD (D:) | `<game>/!saves` | n/a |
| settings keys | `glshader`, `fullscreen`, `cycles`, `custom_conf` | same | `fullscreen`, `custom_conf` | `svm_*`, `fullscreen` | n/a |
| variant index | `dosbox.txt` | `dosbox3x.txt` | `dosbox9x.txt` (launcher slug: x98 / 86box / 86boxME / NetHost / NetJoin / pcbox) | `dosboxsvm.txt` (build) | n/a |
| Tier 0 covers | `previews/eXoDOS` | `previews/eXoWin3x` | `previews/eXoWin9x` | `previews/eXoScummVM` | `previews/eXoMedia` (`covers::MEDIA_SOURCE`) |

The dispatch is `launchers::prepare(kind, ctx)`: DOSBox hands the spine a
command to spawn; the Win9x and ScummVM launchers (ported whole from Exodium)
spawn through the shared `spawn_emulator_and_track` themselves and return
`LaunchOutcome::Launched`. `download_game` still asks the launcher kind for
its support payload and emulator pack (`commands/install.rs`).

Windows 9x multiplayer (PPTP/GRE through DOSBox-X's pcap backend) needs a
system `dosbox-x` with `cap_net_raw` (`pkexec setcap`) and a wired interface;
Settings → Network carries the switch. The Media Pack's magazines, books and
catalogues live in their own torrent (`eXoDOS Media Pack.torrent`) and join
the shared session on first use (`media_sources.rs`).

In the UI nothing keys on a collection id: the detail panel asks
`ui/util.rs::platform_tag` / `collection_label` and the launch notes module
for what to show, and those read the row's `platform` / `torrent_source`
through the same helpers the web UI used.

## Adding a collection eXo publishes later

1. `CollectionDef` row (id, display name, files, `game_prefix`,
   `shortcode_segment`, `lang_dir` / `year_subdirs`, platform, launcher kind).
2. Bundled files: `<name>.xml.gz`, `<name>_configs.zip`, variant index,
   `.torrent`, Tier 0 previews under `crates/core/resources/previews/<col>/`,
   shelf art under `crates/app/assets/collections/`, manifest entry.
   Exodium's `scripts/gen_win3x_assets.py` / `gen_win9x_assets.py` /
   `gen_scummvm_assets.py` show how each was derived from the torrent's own
   metadata zip.
3. A `(index_file, family)` pair in `crates/core/examples/generate_db.rs`;
   raise `db::CATALOG_VERSION`;
   `cargo run -p exorchy-core --release --example generate_db && gzip -kf metadata/exorchy.db`.
4. A new emulator: a `launchers/<kind>.rs`, a `Launcher` variant, one match arm
   in `launchers::prepare`, a `SupportPack` if it has a util zip, resolvers in
   `emulators.rs` and emulator packs in the manifest.
5. UI: a label/hint in the Settings → Collections list
   (`ui/settings.rs`), the shelf art, and, if the panel needs it, an arm in
   `ui/launch_notes.rs`.
