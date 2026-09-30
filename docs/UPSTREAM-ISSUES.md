# Exodium issues vs eXorchy

Every issue on [tvollstaedt/exodium](https://github.com/tvollstaedt/exodium/issues),
checked against eXorchy's code. Surveyed 2026-09-30, issues #1-#38 (22 open,
16 closed). To refresh, list the issues newer than #38 (and any closed since)
and add rows here; do not re-derive the ones below.

Priorities: **P1** do next · **P2** real feature, larger effort · **P3** nice to
have or blocked · **Done** already in eXorchy · **N/A** does not apply to a
native GTK app on Omarchy (Linux x86_64, pacman package). The P1 plan is
[ROADMAP.md](ROADMAP.md).

## Summary

| # | Title (short) | Upstream | Us |
|---|---|---|---|
| [18](https://github.com/tvollstaedt/exodium/issues/18) | Catalogue updates: new eXoDOS release without losing the library | open | **P1** |
| [32](https://github.com/tvollstaedt/exodium/issues/32) | Fetch collection torrents instead of bundling | open | **P1** (as part of self-hosting) |
| [15](https://github.com/tvollstaedt/exodium/issues/15) | Printer emulation / DOSBox-X routing | open | **P1** |
| [29](https://github.com/tvollstaedt/exodium/issues/29) | ScummVM AppImages on native x86_64 Linux | open | **P1** |
| [14](https://github.com/tvollstaedt/exodium/issues/14) | Media Pack: soundtracks, videos, TV | open | **P2** |
| [13](https://github.com/tvollstaedt/exodium/issues/13) | DOS multiplayer (IPX) | open | **P2** |
| [31](https://github.com/tvollstaedt/exodium/issues/31) | Distribution through package managers | open | **P2** (AUR only) |
| [19](https://github.com/tvollstaedt/exodium/issues/19) | 3D box art | open | P3 |
| [23](https://github.com/tvollstaedt/exodium/issues/23) | YDKJ Movies draws garbage (Win9x) | open | P3 |
| [16](https://github.com/tvollstaedt/exodium/issues/16) | eXoWAD / eXoZZT / Unlimited Adventures | open | P3 |
| [11](https://github.com/tvollstaedt/exodium/issues/11) | eXoDREAMM collection | open | P3 |
| [20](https://github.com/tvollstaedt/exodium/issues/20) | RetroAchievements | open | P3 (blocked) |
| [6](https://github.com/tvollstaedt/exodium/issues/6) | librqbit holds one fd per torrent file | open | P3 (mitigated) |
| [37](https://github.com/tvollstaedt/exodium/issues/37) | Mute video previews | open | Done |
| [38](https://github.com/tvollstaedt/exodium/issues/38) | Custom DOSBox binary | open | Done (DOSBox Pure N/A) |
| [24](https://github.com/tvollstaedt/exodium/issues/24) | Grid virtualization | open | N/A |
| [33](https://github.com/tvollstaedt/exodium/issues/33) | Linux ARM64 build | open | N/A |
| [34](https://github.com/tvollstaedt/exodium/issues/34) | Portable mode | open | N/A |
| [35](https://github.com/tvollstaedt/exodium/issues/35) | Storage page pulls cloud placeholder files | open | N/A |
| [1](https://github.com/tvollstaedt/exodium/issues/1) | Online/offline mode, seeding opt-in | closed | Done |
| [2](https://github.com/tvollstaedt/exodium/issues/2) | Screenshot thumbnail cache | closed | Done |
| [3](https://github.com/tvollstaedt/exodium/issues/3) | Cover preloading | closed | N/A |
| [4](https://github.com/tvollstaedt/exodium/issues/4) | Slow jump bar | closed | N/A |
| [5](https://github.com/tvollstaedt/exodium/issues/5) | Section header dividers | closed | Done? (verify) |
| [7](https://github.com/tvollstaedt/exodium/issues/7) | Spanish LP variants launch EN / fail | closed | Done |
| [8](https://github.com/tvollstaedt/exodium/issues/8) | eXoWin3x collection | closed | Done |
| [9](https://github.com/tvollstaedt/exodium/issues/9) | eXoWin9x collection | closed | Done |
| [10](https://github.com/tvollstaedt/exodium/issues/10) | eXoScummVM collection | closed | Done |
| [12](https://github.com/tvollstaedt/exodium/issues/12) | Video previews in the detail panel | closed | Done |
| [17](https://github.com/tvollstaedt/exodium/issues/17) | AppImage SIGKILLed after install | closed | N/A |
| [21](https://github.com/tvollstaedt/exodium/issues/21) | List view | closed | Done |
| [22](https://github.com/tvollstaedt/exodium/issues/22) | Download tracker refactor (TS store) | closed | N/A |
| [25](https://github.com/tvollstaedt/exodium/issues/25) | REELMagic playtest | closed | Done |
| [26](https://github.com/tvollstaedt/exodium/issues/26) | GLP German titles without ApplicationPath | closed | Done |
| [27](https://github.com/tvollstaedt/exodium/issues/27) | Media Pack spike, zip64 | closed | Done |
| [28](https://github.com/tvollstaedt/exodium/issues/28) | Search freeze (WebKit emoji crash) | closed | N/A |
| [30](https://github.com/tvollstaedt/exodium/issues/30) | "Open in system PDF viewer" does nothing | closed | Done |
| [36](https://github.com/tvollstaedt/exodium/issues/36) | Android version | closed | N/A |

## P1

### #18 Catalogue updates for a new eXo release

Upstream: a new eXoDOS torrent (eXoDOS 7 is being discussed) invalidates every
stored torrent file index; the only path upstream is Factory Reset.

Our state: better than upstream's issue text assumes.
- `db::refresh_catalog` (`crates/core/src/db/mod.rs:91`) updates rows **in
  place**, matched on `application_path` (title + language when empty), so
  `games.id`, `game_config`, favourites, `in_library`, `installed`,
  `last_played` and user playlists survive. New rows are inserted; curated
  playlists are replaced.
- Rows that vanished from the catalogue are kept but only logged
  ("torrent indices may be stale"). Nothing marks or surfaces them.
- `init_download_manager` records `<col>_infohash` write-if-absent
  (`commands/setup.rs:319`), and nothing reads it: there is no handling of
  the torrent itself changing (old fastresume, old files on disk).
- Detection: none. `manifest_url` is empty and `load_manifest` has an HTTP
  `TODO` (`commands/updates.rs:130`); torrents ship in the package
  (`packaging/PKGBUILD` copies `torrents/` to `/usr/lib/exorchy/torrents`).

Plan: [ROADMAP.md](ROADMAP.md) step 4.

### #32 Torrents fetched, not bundled (folded into self-hosting)

Upstream motivation is package-review hygiene and decoupling torrent updates
from app releases. For us the second half matters: together with the remote
manifest it lets a new eXo torrent ship without an app release. Same
constraint as upstream: the fetched torrent must match the manifest's
infohash exactly, because the catalogue stores file indices into it.

Related gap from the old handover notes: `manifest.json` still points the
poster packs and the DOSBox-X / 86Box / ScummVM packs at Exodium's GitHub
releases (`content-v4` … `content-v8`). Plan: [ROADMAP.md](ROADMAP.md) step 2.

### #15 Printer emulation, and routing DOSBox-X titles

Upstream: 13 eXoDOS titles print (Print Shop, Newsroom, Laffer Utilities, …).
DOSBox Staging has no printer (the `jn/printing` branch missed 0.83.0).
DOSBox-X has one. Upstream also noted 11 eXoWin3x titles eXo assigns to
DOSBox-X.

Our state:
- Printer titles only get a note (`crates/app/src/ui/launch_notes.rs:407`),
  detected by `conf_requests_printer` (`launchers/dosbox.rs:157`).
- Every DOS/Win3x game goes to Staging (`launchers/dosbox.rs:93`,
  `emulators::resolve_dosbox_staging`).
- DOSBox-X is resolvable only from `commands/win9x.rs:174`
  (`resolve_dosbox_x`: pack AppImage, PATH, Flatpak).
- Our catalogue has **30** DOSBox-X titles, not 11: eXoWin3x `x` 11, eXoDOS
  `x` 16 and `x2` 3 (`games.dosbox_variant`).
- The IDE/ATAPI half upstream shipped is in (`translate_ide_for_staging`).

Plan: [ROADMAP.md](ROADMAP.md) step 3.

### #29 ScummVM AppImages on native x86_64

Upstream only ran the Linux ScummVM AppImages under qemu-user on ARM64.
eXorchy runs exactly on native x86_64. `commands/scummvm.rs` resolves the
pinned pack first, then `scummvm` on PATH, then the Flatpak.

Extra risk found in the survey: every non-Staging emulator pack (DOSBox-X,
86Box, ScummVM) is an AppImage run straight from
`content/emulators/<pack>/`, and nothing sets `APPIMAGE_EXTRACT_AND_RUN` or
extracts it. A stock Omarchy install has `fuse3` but not `fuse2`, so if these
AppImages use the classic runtime (which loads `libfuse.so.2`) they will not
start. Checked 2026-09-30: all four Linux packs (DOSBox-X, 86Box, ScummVM
2.5.0 and 2026.1.0) start natively with only `fuse3`, and a game on each
ScummVM pack (Deja Vu on 2.5.0, Alpha Polaris on 2026.1.0) installed,
launched and saved through eXorchy. For us this is done; the Win9x launches
are tracked in the roadmap. Plan: [ROADMAP.md](ROADMAP.md) step 1.

## P2

### #14 Media Pack (soundtracks, videos, TV)

The Reading Room covers magazines, books and catalogs from the Media Pack
(`media_sources.rs`, `commands/reading.rs`), and `zip_range` has zip64 and the
nested stored zip reader from the #27 spike. There is no soundtrack album
player and no Media Pack video browsing yet; each game's single theme track
from its GameData archive does play. Upstream's spike
facts: 126 album zips, all stored; only 54 match a catalogue title exactly,
so the album → game link has to be a hand-curated index in `generate_db`.
Rule from upstream: lazy, per file, never a bulk install.

### #13 DOS multiplayer (IPX)

Only Windows 9x multiplayer exists (`ui/settings/network.rs`,
`commands/win9x.rs`). DOS games get nothing: no multiplayer flag in the
catalogue, no host/join dialog, no `[ipx]` settings written at launch.
Scoping still needed: how eXo marks multiplayer titles.

### #31 Distribution

Only the AUR matters to us. `packaging/aur/PKGBUILD` is ready;
`docs/RELEASING.md` ("The AUR (when registrations reopen)") has the steps.
The in-app updater already exists (`commands/app_update.rs`); it would need to
step aside for an AUR-managed install, as upstream plans for package managers.

## P3

- **#19 3D box art.** Cosmetic. Only ~1,112 of ~7,600 eXoDOS games have a `Box - 3D`
  scan; `commands/assets.rs:279` already ranks it first in the detail gallery.
  A drawn pseudo-3D effect over the front covers is the only option that
  covers the whole grid.
- **#23 YDKJ Movies garbage window.** Emulator-level, parked upstream,
  diagnosed only on macOS ARM. Try it once under our DOSBox-X on Linux; the
  untried lead upstream is `[voodoo] voodoo_card = false`.
- **#16 eXoWAD / eXoZZT / Unlimited Adventures** and **#11 eXoDREAMM.** Not
  scoped by anyone; nothing in our code.
- **#20 RetroAchievements.** Blocked until RetroAchievements supports MS-DOS (late 2026
  at the earliest). Tier 1 (display only, Web API from Rust) is the first
  step once it does.
- **#6 fd per torrent file.** `raise_fd_limit` (`crates/core/src/lib.rs:51`)
  lifts the soft limit; Arch's default hard limit is far above the ~14k the
  eXoDOS torrent needs. Only matters on locked-down setups.

## Done in eXorchy

- **#37** preview videos have a mute button, persisted as `preview_muted`,
  default muted (`ui/media/preview.rs`, `ui/media/store.rs`).
- **#38** custom DOSBox binary and "prefer system DOSBox" (`emulators.rs:65`).
  DOSBox Pure is a libretro core and does not fit the launch model.
- **#1** network mode and seeding consent in onboarding (`ui/onboarding.rs`).
- **#2** thumbnail cache (`content/thumbcache/`, `commands/assets.rs`).
- **#5** section dividers: not verified. `ui/library.rs` loads section
  keys for the jump bar, but whether the `GridView` draws a divider per
  section was not checked. Look at a snapshot before treating this as done.
- **#7** LP launch path (base game auto-install), inherited from the backend.
- **#8 / #9 / #10** the three collections, inherited.
- **#12** preview videos, **#21** list view, **#25** REELMagic (Staging 0.83
  keeps `[reelmagic]`; eXo's keys pass through verbatim).
- **#26** checked in `metadata/exorchy.db`: Serrated Scalpel DE, CyberMage DE
  and Die Kathedrale have shortcodes and torrent indices.
- **#27** zip64 and nested-zip reading in `torrent/zip_range.rs`.
- **#30** `commands/shell_open.rs` strips the AppImage environment before
  `xdg-open`; moot for the pacman build anyway.

## N/A

- **#3, #4, #24** web-grid rendering costs. The native grid is a
  `gtk::GridView` (`ui/library.rs:173`), which only builds visible cells.
- **#17, #28** AppImage/WebKit-specific crashes; we ship neither.
- **#22** refactor of the TypeScript download store, gone with the web UI.
- **#33** ARM64 (Omarchy is x86_64), **#34** portable mode (Windows profile
  paths; we are a pacman package), **#35** Windows cloud placeholder files,
  **#36** Android.
