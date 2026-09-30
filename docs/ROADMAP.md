# eXorchy roadmap: P1

The four P1 items from [UPSTREAM-ISSUES.md](UPSTREAM-ISSUES.md), in the
order to do them. Written 2026-09-30. Tick items off here as they land, and
record each non-obvious choice in `DECISIONS.md`.

| Step | What | Upstream | Size | Depends on |
|---|---|---|---|---|
| 1 | Make sure the emulator AppImage packs start on Omarchy; ScummVM end to end | #29 | hours | - |
| 2 | Host our own content and manifest | #32 + handover gap | 1-2 days | - |
| 3 | Route printer and DOSBox-X titles to DOSBox-X | #15 | 3-5 days | 1 |
| 4 | Survive a new eXo torrent release | #18 | 1-2 weeks | 2 |

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
- [ ] ScummVM end to end: install one game pinned to 2.5.0 and one pinned
      to 2026.1.0, check that the pinned pack is queued with the first
      download, launch both, and check that saves land in `<game dir>/!saves`.
- [ ] Also launch one Win9x `x98` title and one `86box` title. The
      handover lists every non-Staging launch as unverified.
- [ ] Comment the result on upstream #29; they asked for exactly this run.

## Step 2: own hosting (#32, and the manifest gap)

Today 11 pack entries point at `github.com/tvollstaedt/exodium/releases`
(`content-v4` … `content-v8`): five poster packs, DOSBox-X, 86Box and the four
ScummVM packs. The seven collection `.torrent` files and the Media Pack
torrent ship in the package, and `manifest_url` is empty.

Re-hosting without forcing reinstalls: the pack ledger keys on `version`
(`commands/content_packs.rs:161`), and downloads are checked against the
manifest's `sha256`. Mirroring the files byte for byte and changing only the
URLs therefore leaves installed packs alone.

- [ ] Licensing first. DOSBox-X, 86Box and ScummVM are GPL. Redistributing
      Exodium's builds obliges us to provide the corresponding source (or
      point at the exact upstream tags plus Exodium's build scripts, MIT).
      Decide which, and put it in the release notes and
      `ACKNOWLEDGEMENTS.md`.
- [ ] Create a `content-v1` release on our repo and mirror the Linux
      x86_64 assets. Verify each file's `sha256` against the manifest before
      uploading. Leave the macOS and aarch64 variants out.
- [ ] Rewrite the URLs in `manifest.json` and drop the non-Linux
      `platforms` entries. Add a test that every pack URL is either on our
      repo or on the emulator's own project (like DOSBox Staging's
      official release).
- [ ] Later, rebuild the posters ourselves (`scripts/gen_thumbnails.py`,
      `gen_previews.py`; same hash scheme), so art fixes such as the
      `Feria D'Arles` cover from #29 do not wait for upstream.
- [ ] Remote manifest: point `manifest_url` at a stable asset, e.g. a
      `manifest.json` on a fixed `manifest` release tag. `load_manifest`
      (`commands/updates.rs:109`) currently reads only the dev copy and the
      bundled copy. New order: a cached remote copy in `content/` if its
      `schema_version` is supported and `generated_at` is newer than the
      bundled one, else the bundled copy. Fetch at startup, never in
      offline mode, and fail silently to the cache.
- [ ] Rule for the remote manifest: it may change pack URLs, hashes and
      versions. It may **not** change a collection's `torrent_infohash`
      unless a catalogue that matches that torrent comes with it (step 4).
      Enforce this in code, not by convention.
- [ ] Torrents (#32): keep bundling them for now. Add a
      `content/torrents/` cache that `bundled_torrent_path`
      (`commands/paths.rs:176`) checks first, plus an infohash check against
      the manifest on load. That is the hook step 4 uses to deliver a new
      torrent without an app release.
- [ ] Docs: ARCHITECTURE (manifest sources), RELEASING (publishing a
      content release), DECISIONS entry.

## Step 3: DOSBox-X for printer and x-variant titles (#15)

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
- [ ] Playtest: Laffer Utilities, The New Print Shop, one eXoDOS `x`, one
      `x2` and one eXoWin3x `x` title.
- [ ] Keep an eye on Staging `jn/printing`. If it lands, printer titles could
      go back to Staging and only the 30 x-variant titles would keep
      DOSBox-X.
- [ ] Docs: COLLECTIONS (engine per variant), ARCHITECTURE (launch
      pipeline), DECISIONS.

## Step 4: a new eXo release without losing the library (#18)

What already works: `db::refresh_catalog` (`db/mod.rs:91`) updates rows in
place, matched on `application_path` and then on title + language. So ids,
`game_config`, favourites, install state, `last_played` and user playlists
already survive a catalogue bump. What is missing:

- [ ] **Better matching.** Add a shortcode + collection pass between the
      `application_path` pass and the title pass, so a renamed `.bat`
      keeps its row. The shortcode is the key upstream calls "the only one
      that survives a torrent rebuild".
- [ ] **Orphans.** Rows that are no longer in the catalogue are kept and
      logged only. Add an `orphaned` user column (add it to `USER_COLS`, or
      the next refresh overwrites it). Hide orphaned games that are not
      installed from Browse. Keep installed ones launchable with a "no
      longer in eXoDOS <n>" note. Never offer a download for them: their
      torrent index is invalid.
- [ ] **Torrent swap.** When a collection's bundled or cached torrent
      infohash differs from the recorded `<col>_infohash`
      (`commands/setup.rs:319`, currently write-if-absent and never read):
      drop the old torrent from the session along with its fastresume
      (`torrent/manager.rs:260`); add the new torrent with an empty
      selection and a seeded empty bitfield (the same trick the first add
      uses); then update `<col>_infohash`. Installed games keep running from
      their extracted directories and are not re-verified. Only a reinstall
      downloads from the new torrent.
- [ ] **Delivery.** A new catalogue and torrent arrive either with an app
      release (`CATALOG_VERSION` bump, same as today) or through step 2's
      remote manifest plus the torrent cache. Decide whether a catalogue
      may come without an app release. The simple answer is no: ship the
      catalogue in the package and let the remote manifest only announce
      "a release supporting eXoDOS <n> is out".
- [ ] **Detection.** A scheduled GitHub Action fetches the published
      collection torrents from retro-exo.com, computes the infohashes with
      the existing bencode code (`TorrentIndex::infohash`), and opens an
      issue on our repo if one differs from `manifest.json`. It is a
      backstop only; eXo announces major releases well ahead.
- [ ] **Dry-run tool.** `examples/catalog_diff.rs`: given the installed DB
      and a new catalogue, report matched, re-keyed, new and orphaned rows,
      and which installed games are affected. Run it the day eXoDOS 7
      appears, before shipping anything.
- [ ] **Tests** with synthetic fixtures: a renamed `application_path`
      matched by shortcode, an installed game that vanished (becomes
      orphaned, stays launchable), changed torrent indices, a user playlist
      that contains an orphan, and an infohash change that triggers the
      swap exactly once.
- [ ] Docs: ARCHITECTURE (catalogue refresh, torrent swap), COLLECTIONS,
      DECISIONS.
