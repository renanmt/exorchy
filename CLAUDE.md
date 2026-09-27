# Exorchy - guide for Claude sessions

Omarchy-native launcher for the eXo collections (Tauri v2 + Rust + SolidJS),
derived from Exodium (MIT) with its full feature set: eXoDOS + language packs,
eXoWin3x, eXoWin9x, eXoScummVM, previews/music, the reading room. Only eXoDOS
is enabled by default; the rest are switches in Settings → Collections.

Read in this order before changing code:
1. `docs/HANDOVER.md` - state, toolchain, commands, conventions, known gaps
2. `docs/ARCHITECTURE.md` - module map, launch pipeline, downloads, theming
3. `docs/DECISIONS.md` - why (append-only; add an entry for every non-obvious decision)
4. `docs/COLLECTIONS.md` - how each collection is wired and how enabling/hiding works

Hard rules:
- `export PATH="$HOME/.cargo/bin:$PATH"` before cargo / `pnpm tauri`.
- Every Tauri command touching DB/FS/network is `async fn`; never hold the DB mutex across `.await`.
- No hard-coded colors; tokens only (`--om-*` → semantic tokens in `src/styles/main.css`).
- Emulator-specific code stays in its launcher module (`launchers/dosbox.rs`, `commands/win9x.rs`, `commands/scummvm.rs`); nothing keys on a collection id outside `commands/collections.rs` and the enabled-set filter.
- Credits to eXoDOS and to Thomas Vollstädt (Exodium) stay in README, ACKNOWLEDGEMENTS.md, LICENSE and Settings → About.
- Raise `db::CATALOG_VERSION` BEFORE `pnpm gen-db`.
- Keep the librqbit `[patch.crates-io]` pin.
- Update the four docs above when you change behaviour; future sessions start from them.
