# eXorchy roadmap: P1

The four P1 items from [UPSTREAM-ISSUES.md](UPSTREAM-ISSUES.md), in the
order to do them. Written 2026-09-30. Tick items off here as they land, and
record each non-obvious choice in `DECISIONS.md`.

| Step | What | Upstream | Size | Depends on | Status (2026-10-01) |
|---|---|---|---|---|---|
| 1 | Make sure the emulator AppImage packs start on Omarchy; ScummVM end to end | #29 | hours | - | done except one 86Box launch and the optional comment on upstream #29 |
| 2 | Host our own content and manifest | #32 + handover gap | 1 session | - | parked by the user; fully researched |
| 3 | Route printer and DOSBox-X titles to DOSBox-X | #15 | 3-5 days | 1 | done and playtested (PR #12) |
| 4 | Survive a new eXo torrent release | #18 | 2-3 sessions | 2 | parked until eXoDOS 7 is closer; fully researched |

Sizes are counted in working sessions with Claude, not person-days: the code
itself is quick, and what takes time is the user's review and playtests in
between (corrected 2026-10-01; earlier drafts said days and weeks).

Why this order: step 1 is cheap and can block step 3 (same DOSBox-X pack).
Step 2 removes our dependency on Exodium's release assets and gives step 4
its delivery channel. Step 4 is the biggest job and cannot be tested against
real data until eXoDOS 7 exists, so it builds on synthetic fixtures and
comes last.

## Step 1: emulator packs on native x86_64 (#29)

The DOSBox-X, 86Box and ScummVM packs are AppImages that eXorchy runs in
place from `content/emulators/<pack>/`. Omarchy ships `fuse3`, not `fuse2`. An
AppImage built on the classic runtime needs `libfuse.so.2` and will not
start.

- [x] Run each pack's AppImage once on a stock Omarchy box: DOSBox-X,
      86Box, ScummVM 2.5.0 and 2026.1.0. Get the tarballs from the
      `platforms.linux-x86_64` URLs in `manifest.json`. Run the AppImage with
      `--version`, or with no arguments for 86Box.
      **Done 2026-09-30:** all four start with `fuse3` and no `fuse2`
      (DOSBox-X 2025.02.01, ScummVM 2.5.0 and 2026.1.0 print their version,
      86Box prints its usage). So the AppImages do not need `libfuse.so.2`.
      They do still need FUSE itself, and `fuse3` is present on Omarchy here.
- [x] ~~If any of them needs libfuse2~~: not needed. Kept for the record: the
      fallback would be to extract it when the pack is installed
      (`--appimage-extract` into the pack dir) and resolve `AppRun`
      instead. Then nothing needs FUSE at runtime, and startup is faster
      too. Other options: add `fuse2` to `depends` (a system package pulled in
      for three packs), or set `APPIMAGE_EXTRACT_AND_RUN=1` on the child
      (extracts to `/tmp` on every launch).
      Resolution lives in `commands/win9x.rs:174` (`resolve_dosbox_x`),
      `pack_candidate` (`:119`) and `commands/scummvm.rs`; extraction goes in
      `commands/content_packs.rs`.
- [x] ScummVM end to end: install one game pinned to 2.5.0 and one pinned
      to 2026.1.0, check that the pinned pack is queued with the first
      download, launch both, and check that saves land in `<game dir>/!saves`.
      **Done 2026-09-30** on the real profile: Deja Vu (snapshot pin
      `svn2.3_18903` → 2.5.0 pack; the only game in the catalogue on that pack)
      and Alpha Polaris (2026.1.0). Each first download queued its pack, both
      ran from the pack AppImage and exited with status 0, and the saves are in
      `!saves`. Nothing was written to `~/.config/scummvm` or
      `~/.local/share/scummvm`.
- [ ] Also launch one Win9x `x98` title and one `86box` title. The
      handover lists every non-Staging launch as unverified.
      **`x98` done 2026-10-01:** Time Commando ran through the Win9x launcher
      on the DOSBox-X pack and exited cleanly; the user reported it working
      perfectly. Still owed: one `86box` title.
- [ ] Comment the result on upstream #29; they asked for exactly this run.

## Step 2: own hosting (#32, and the manifest gap)

**Status (2026-10-01): parked by the user, to be picked up later.** Everything
known so far is below; nothing here has been started.

### Where the files come from today

Exodium has no server. Its content packs are assets of GitHub releases in its
own repository (`github.com/tvollstaedt/exodium/releases`), tagged
`content-v3` … `content-v8` rather than version numbers. That is why they are
easy to miss among Exodium's app releases. `manifest.json` downloads straight
from them:

| Release | What eXorchy uses from it |
|---|---|
| `content-v4` | `posters-eXoDOS-v5.tar.gz` (379 MB), `posters-eXoWin3x-v1.tar.gz` (64 MB) |
| `content-v5` | `posters-eXoWin9x-v1.tar.gz` (38 MB) |
| `content-v6` | `dosbox-x-linux-x86_64-v1.tar.gz` (41 MB), `86box-linux-x86_64-v1.tar.gz` (88 MB) |
| `content-v7` | `scummvm-{2.5.0,2.8.0,2.9.0,2026.1.0}-linux-x86_64-v1.tar.gz` (104 / 135 / 142 / 146 MB), `posters-eXoScummVM-v1.tar.gz` (44 MB) |
| `content-v8` | `posters-eXoMedia-v1.tar.gz` (170 MB) |

Only DOSBox Staging comes from its own project (the official
`dosbox-staging-linux-x86_64-v0.83.0.tar.xz`). The seven collection
`.torrent` files and the Media Pack torrent ship inside our package
(`packaging/PKGBUILD` copies `torrents/` to `/usr/lib/exorchy/torrents`), and
`manifest_url` is empty.

**Why bother:** nothing is broken today. The risk is a dependency we do not
control: if Exodium deletes a release, renames its repo or makes it private,
box art and the DOSBox-X / 86Box / ScummVM downloads stop working for every
eXorchy user. The alternative, which is also legitimate, is to keep relying
on Exodium (MIT, credited in README / ACKNOWLEDGEMENTS / About) and mirror only
if something breaks. Recommended: mirror; one working session.

### Licensing: already solved

DOSBox-X, 86Box and ScummVM are GPL, so redistributing binaries obliges us to
offer the corresponding source. Exodium already publishes it next to every
build, so mirroring the source files with the binaries meets the obligation:
`dosbox-x-2025.02.01-source.tar.gz` (117 MB), `86box-6.0-source.tar.gz`
(16 MB), `scummvm-{2.5.0,2.8.0,2.9.0,2026.1.0}-source.tar.xz` (124 / 198 /
210 / 216 MB). `content-v6` and `content-v7` also carry a `SHA256SUMS`. Name
the sources in the release notes and in `ACKNOWLEDGEMENTS.md`.

**Full copy list:** the Linux x86_64 builds, all posters and the sources come
to **2.18 GB** (19 files including the two `SHA256SUMS`). Leave out the
`macos-*` and `linux-aarch64` variants; eXorchy is Omarchy, x86_64 only.

### Re-hosting without forcing reinstalls

The pack ledger keys on `version` (`commands/content_packs.rs:161`), and
downloads are checked against the manifest's `sha256`. Mirroring the files
byte for byte and changing only the URLs therefore leaves installed packs
alone: nobody downloads anything again.

### Plan

- [ ] Create a `content-v1` release on `renanmt/exorchy`. Download each file
      above from Exodium, check its sha256 against `manifest.json` (and the
      release's `SHA256SUMS` where there is one), upload it with `gh release
      upload`. Include the sources.
- [ ] Rewrite the URLs in `manifest.json` and drop the non-Linux `platforms`
      entries. Add a test that every pack URL is either on our repo or on the
      emulator's own project (like DOSBox Staging's official release).
- [ ] Later, rebuild the posters ourselves (`scripts/gen_thumbnails.py`,
      `gen_previews.py`; same hash scheme), so art fixes such as the
      `Feria D'Arles` cover from #29 do not wait for upstream.
- [ ] Remote manifest: point `manifest_url` at a stable asset, e.g. a
      `manifest.json` on a fixed `manifest` release tag. `load_manifest`
      (`commands/updates.rs:109`) currently reads only the dev copy and the
      bundled copy (its HTTP fetch is a `TODO`). New order: a cached remote
      copy in `content/` if its `schema_version` is supported and
      `generated_at` is newer than the bundled one, else the bundled copy.
      Fetch at startup, never in offline mode, and fall back silently to the
      cache.
- [ ] Rule for the remote manifest: it may change pack URLs, hashes and
      versions. It may **not** change a collection's `torrent_infohash`
      unless a catalogue that matches that torrent comes with it (step 4).
      Enforce this in code, not by convention.
- [ ] Torrents (#32): keep bundling them for now. Add a `content/torrents/`
      cache that `bundled_torrent_path` (`commands/paths.rs:176`) checks
      first, plus an infohash check against the manifest on load. That is the
      hook step 4 can use to deliver a new torrent without an app release.
- [ ] Docs: ARCHITECTURE (manifest sources), RELEASING (publishing a content
      release), DECISIONS entry.

## Step 3: DOSBox-X for printer and x-variant titles (#15)

**Done 2026-09-30** (branch `dosbox-x-routing`): engine choice, DOSBox-X
branch, PNG printouts in `!prints`, pack queued with the download, Game
Settings and panel notes. 31 games default to DOSBox-X (eXoDOS 20 incl.
Laffer Utilities via the printer rule, eXoWin3x 11). Playtested
2026-10-01. Still open below: a panel button to open the printouts folder,
and Laffer's partial printing.

Who is affected:
- 30 titles eXo assigns to DOSBox-X, counted from `games.dosbox_variant`:
  eXoWin3x `x` (11), eXoDOS `x` (16) and `x2` (3). They run under Staging
  today.
- 13 eXoDOS printer titles (`conf_requests_printer`,
  `launchers/dosbox.rs:157`), mostly `ece4230` variants. Staging cannot print,
  and 0.83.0 shipped without `jn/printing`.

Plan:
- [ ] Move DOSBox-X resolution out of `commands/win9x.rs` into
      `emulators.rs`, next to `resolve_dosbox_staging`, so the DOS launcher
      can use it without reaching into the Win9x module. Win9x keeps calling
      it from there.
- [ ] Engine choice in `launchers/dosbox.rs::prepare`: the default is
      DOSBox-X for `x`/`x2` variants and printer confs, Staging for
      everything else. The per-game `engine` key in `game_config` (already
      read by `set_game_settings`) overrides it. The Game Settings dialog
      shows its Emulator row only when ECE is available (never on Linux);
      show it when DOSBox-X is an option, with the note reworded.
- [ ] DOSBox-X branch of `prepare`: keep host-path rewriting and the LP
      overlay. Skip `translate_midi_for_staging` and
      `translate_ide_for_staging`, because DOSBox-X reads eXo's ECE/X keys
      natively. Skip the Staging-only `[render] glshader` fragment.
      Check whether `options.conf` is safe to pass to DOSBox-X.
- [ ] Printer output: add `[printer] printoutput=png` and
      `docpath=<game dir>/!prints`, so printouts sit next to saves and follow
      uninstall/backup. In the detail panel, show "Printouts (n)" that opens
      the folder. Converting the pages to PDF can come later. Check the
      Flatpak fallback can write to `docpath`.
- [ ] Pack dependency: queue the `dosbox-x` pack with the first download of
      these games. Win9x already does this through
      `emulator_pack_for_variant` (`commands/win9x.rs:247`) and
      `commands/install.rs:112`, so extend that path instead of adding a
      new one.
- [ ] Launch notes: replace the "cannot print" note with "runs under
      DOSBox-X", and adjust the shader note (DOSBox-X has no Staging shaders).
- [x] Playtest: Laffer Utilities, The New Print Shop, one eXoDOS `x`, one
      `x2` and one eXoWin3x `x` title. **Done 2026-10-01** on the real
      profile: The New Print Shop printed two PNG pages, Jet Stream (`x2`)
      and Disney's The Jungle Book (eXoWin3x `x`) run. Laffer prints only
      the start of a page and then waits (kept on DOSBox-X, see DECISIONS
      2026-10-01). The playtest also showed DOSBox-X cannot rescale or go
      fullscreen on Hyprland; it now opens as a large floating window, also
      confirmed working.
- [ ] Keep an eye on Staging `jn/printing`. If it lands, printer titles could
      go back to Staging and only the 30 x-variant titles would keep
      DOSBox-X.
- [ ] Docs: COLLECTIONS (engine per variant), ARCHITECTURE (launch
      pipeline), DECISIONS.

## Step 4: a new eXo release without losing the library (#18)

**Status (2026-10-01): parked by the user until eXoDOS 7 is closer.** The
pieces can be built and tested with synthetic data any time; real
verification needs the real torrent.

### What a new eXo release changes

A new torrent with a new infohash. Every file index inside it shifts. Games
are added, removed and renamed; configs, metadata and art change.

### What eXorchy keeps, and depends on

- **The catalogue** (`games` table): per game, its index into the current
  torrent (`game_torrent_index`, `gamedata_torrent_index`) and its
  `application_path` inside eXo's tree, which carries the shortcode
  (`eXo\eXoDOS\!dos\<shortcode>\<Title>.bat`) and rarely changes.
- **User data:** `favorited`, `in_library`, `installed`, `last_played`,
  `game_config` (per-game settings), user playlists, hidden titles.
- **On disk:** the extracted games and their saves (`!save`, the pristine
  index for uninstall backups); the torrent session (`session.json` plus a
  `<infohash>.bitv` per torrent under `~/.local/share/exorchy/librqbit-fastresume`),
  which knows the old torrent; and the configs unpacked from the bundled
  `*_configs.zip`.

### What already works

`db::refresh_catalog` (`db/mod.rs:91`), run at startup when the bundled
catalogue's `CATALOG_VERSION` is newer, updates rows **in place**, matched on
`application_path`, then on title + language when the path is empty. Ids do
not change, so favourites, library, install state, last played, per-game
settings and user playlists already survive. New games are inserted; curated
playlists and the Reading Room tables are replaced.

### What is missing, in order of importance

1. **Matching.** A game whose `.bat` eXo renames gets no match and becomes a
   new row, losing its user data. Add a shortcode + collection pass between
   the `application_path` pass and the title pass; the shortcode is what
   survives a torrent rebuild.
2. **Removed games.** Rows the catalogue no longer has are kept but only
   logged ("torrent indices may be stale"). Add an `orphaned` user column
   (it must go into `USER_COLS`, or the next refresh overwrites it). Hide
   orphaned games that are not installed from Browse; keep installed ones in
   My Library, playable, with a "no longer in eXoDOS <n>" note; never offer a
   download, since their torrent index is meaningless.
3. **The torrent switch.** `init_download_manager` records `<col>_infohash`
   write-if-absent (`commands/setup.rs:319`) and nothing ever reads it. When
   the bundled or cached torrent's infohash differs from it: remove the old
   torrent from the session along with its fastresume
   (`torrent/manager.rs:260`); add the new one with an empty selection and a
   seeded empty bitfield (the same trick the first add uses, so 600 GB are
   not hash-checked); then record the new infohash. Installed games keep
   running from their extracted folders; only a reinstall downloads from the
   new torrent. Seeding of the old torrent stops.
4. **Updated configs never reach existing games** (found 2026-10-01).
   `extract_bundled_configs` (`commands/setup.rs:380`) unpacks a collection's
   bundled configs once (marker `.<col>_configs_extracted`) and, through
   `extract_missing_entries`, only for games whose folder does not exist
   yet. A config fix in eXoDOS 7 would never replace the old config of a
   game already on disk. Decision needed (see below); the mechanics are a
   per-collection config version, re-extraction of the changed games, and
   leaving eXorchy's own layers alone (Game Settings live in `game_config`
   and the launch fragments, never in eXo's `dosbox.conf`).
5. **Spotting the release.** A scheduled GitHub Action fetches the published
   torrents from retro-exo.com, computes their infohashes with the existing
   bencode code (`TorrentIndex::infohash`) and opens an issue on our repo
   when one differs from `manifest.json`. A backstop only: eXo announces
   major releases well ahead.
6. **A dry-run tool.** `examples/catalog_diff.rs`: given an installed
   database and a new catalogue, report matched, re-keyed (shortcode pass),
   new and orphaned rows, and which installed games are affected. The first
   thing to run the day eXoDOS 7 appears, before shipping anything.

### How the day itself would go

1. Download eXo's new XML files, configs and metadata zips.
2. Regenerate the catalogue (`generate_db`, adjusted if eXo changed the XML
   format) after raising `CATALOG_VERSION`; rebuild thumbnails and the poster
   packs; update the `.torrent` files and `manifest.json` infohashes.
3. Run the dry run against a real library (the user's own profile, read-only).
4. Release eXorchy with the new catalogue and torrents.
5. On the user's machine, the update handles the rest on its own, with no
   factory reset: catalogue rows updated (gap 1), removed games marked
   (gap 2), torrent switched (gap 3), configs refreshed (gap 4).

### Decisions for the user (none urgent)

- **How a new catalogue arrives.** Recommended: only with an app release,
  since a format change needs new code anyway; the remote manifest (step 2)
  may only announce "a release supporting eXoDOS <n> is out".
- **How removed-but-installed games look.** Recommended: they stay in My
  Library with a "no longer in eXoDOS <n>" note, playable.
- **Configs of installed games** (gap 4). Recommended: replace them on
  upgrade, since the user's own settings sit in a separate layer. The
  alternative is a per-game "Update config" offer.
- **Flagging games eXo changed.** Optional, could come later: an "eXo
  updated this game" marker on installed games whose archive changed in the
  new torrent (different size or hash for the same shortcode), with a
  Re-download action.

### Size

Two to three working sessions plus the user's checks: matching and orphans,
the torrent switch, configs, detection and the dry run, docs (ARCHITECTURE:
catalogue refresh and torrent switch; COLLECTIONS; DECISIONS).

### Tests (synthetic fixtures)

- [ ] A renamed `application_path` matched by shortcode, user data kept.
- [ ] An installed game that vanished: orphaned, still launchable, no download.
- [ ] Changed torrent indices on matched rows.
- [ ] A user playlist that contains an orphan.
- [ ] An infohash change that triggers the torrent switch exactly once.
- [ ] A changed config replaced on upgrade, Game Settings untouched.

## UI revamp (before 0.5)

The user's design (2026-10-01, "Concept A — Archive" mockup): a left
navigation sidebar, a filter bar, denser cover cards, a "Game Dossier" detail
panel with tabs, and a status bar with counts and keyboard hints. The
mockup's colours are one theme; everything is built on the Omarchy tokens,
so it takes each theme's colours. Its content is placeholder: invented
covers, console platform codes, and a features list (save states, rewind,
achievements) that is not true for DOS games.

Decisions (user, 2026-10-01):
- The sidebar **browses**: Publishers, Series, Years, Regions list their
  values with counts, picking one shows its games; Platforms, Genres, Play
  status and Favorites filter directly. "Tags" is left out (no such data);
  catalogue has 5 platforms, 2,100 publishers, 2,306 series, 49 years,
  13 regions, 11 play modes.
- The **A–Z jump bar stays**.
- **No decoration**: no taglines, no pixel illustration.
- The **Game Dossier first**.

Phases, each its own PR checked with snapshots at wide and narrow widths
(the sidebar collapses and the dossier overlays in a half-screen tile):

- [x] **Game Dossier** (branch `ui-dossier`, 2026-10-01): header with
      favourite and close, hero row (cover, platform, title, meta, language
      chips, Play split button, Add to playlist, ⋯), genre tags, tabs
      Overview / Media / Manuals / Setup, real features only; shortcuts
      Enter (play), F (favourite), I (dossier); Enter or double-click on
      the shown card plays. Review fixes: Media counts screenshots and
      articles (no "Covered in" heading), tabs reset to Overview on a new
      game, full width in a small tile, a larger title, framed hairline
      borders after the concept.
- [x] **Shell and cards** (branch `ui-shell`, 2026-10-01): status bar
      (version, the view's game count, favourites, playlists, collections,
      key hints; hints hide below 1300 sp, counts below 760 sp), toolbar
      with a hairline and the tabs as one outlined segmented group (active
      tab in the accent), hairline search field, denser cards (title and
      "year · platform · genre", platform badge gone, the status line only
      for downloads and incomplete installs, star only on favourites or on
      hover, hairline outline, installed in an accent hairline, selected in
      a 2 px accent outline). Awaiting the user's review.
- [x] **Sidebar and filters** (branch `ui-sidebar`, 2026-10-01): the
      sidebar browses by Platforms, Genres, Publishers, Series, Years,
      Regions, Tags (122 from eXo's `series` field) and Play Status, with
      counts; Favorites filters. The filter bar has Platforms (replacing the
      collection chips), Genres, Years, Regions and Playlists dropdowns and
      a chip for a sidebar pick. Backend: `BrowseFilter` in `GameFilter`,
      `get_games_browse` / `get_section_keys_browse` / `get_facet_values`.
      Awaiting the user's review.

Reference for every phase: `tmp/concept 01.png` (the user's mockup, local and
gitignored). Borders as drawn there: framed panels inset from the window edge,
`--hairline` (1 px) outlines in `--line-frame`, outlined controls and tags, a
thicker accent outline on the selected card.
- [ ] **Polish:** narrow-window behaviour of all of it, docs.
