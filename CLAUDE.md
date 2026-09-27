# eXorchy - guide for Claude sessions

Omarchy-native launcher for the eXo collections, written in Rust: a GTK4 +
libadwaita app (`crates/app`) over the backend derived from Exodium (MIT)
(`crates/core`), with Exodium's full feature set: eXoDOS + language packs,
eXoWin3x, eXoWin9x, eXoScummVM, previews/music, the Reading Room. The four game
collections are enabled by default; the language packs are switches in Settings → Collections. The
app is called "eXorchy" (capital X) in every user-facing string; the binary,
package, directories and identifiers stay lowercase `exorchy`; the
GApplication id is `org.exorchy.eXorchy`.

Read in this order before changing code:
1. `docs/HANDOVER.md` - state, toolchain, commands, conventions, port checklist, known gaps
2. `docs/ARCHITECTURE.md` - module map, host shim, bridge, launch pipeline, downloads, theming
3. `docs/DECISIONS.md` - why (append-only; add an entry for every non-obvious decision)
4. `docs/COLLECTIONS.md` - how each collection is wired and how enabling/hiding works
5. `docs/PORTING.md` - conventions for UI code while the native port is in progress

Hard rules:
- `export PATH="$HOME/.cargo/bin:$PATH"` before any `cargo` command. No node, no pnpm.
- Every backend command function in `crates/core/src/commands` touching DB/FS/network is
  `async fn`; never hold the DB mutex across `.await`.
- No hard-coded colours anywhere: tokens only. `theme.rs` defines them on `:root` from
  Omarchy's palette; `style.css` and the per-module sheets derive from them with `color-mix()`.
- UI modules own their CSS sheet (`crates/app/src/styles/<module>.css`, registered in
  `theme.rs`). A new feature is a new `ui/<module>.rs` plus one hook call in `window.rs` /
  `library.rs` / `detail.rs`, not edits spread across modules.
- Emulator-specific code stays in its launcher module (`launchers/dosbox.rs`,
  `commands/win9x.rs`, `commands/scummvm.rs`); nothing keys on a collection id outside
  `commands/collections.rs` and the enabled-set filter (`db::queries::enabled_sql`).
- Credits to eXoDOS and to Thomas Vollstädt (Exodium) stay in README, ACKNOWLEDGEMENTS.md,
  LICENSE and Settings → About.
- Raise `db::CATALOG_VERSION` BEFORE `cargo run -p exorchy-core --release --example generate_db`.
- Keep the librqbit `[patch.crates-io]` pin in the root `Cargo.toml`.
- Visual checks go through the app itself: `EXORCHY_SNAPSHOT=/path.png[:ms]`
  (`EXORCHY_SNAPSHOT_GAME=<id>` opens a detail panel first, `EXORCHY_DUMP_TREE=1` prints the
  widget tree). No grim, no compositor screenshots.
- Update the docs above when you change behaviour; future sessions start from them.
