# eXorchy

An [Omarchy](https://omarchy.org)-native launcher for the
[eXoDOS](https://www.retro-exo.com/exodos.html) collections. Browse the
catalogue, download single games straight from the eXo torrents, and play
DOS, Windows 3.x, Windows 9x and ScummVM games on Arch + Hyprland. A native
GTK4 application written in Rust; the interface follows your Omarchy theme,
live.

<p align="center">
  <img src="crates/app/assets/splash.jpg" width="720" alt="eXorchy splash: a beige PC with C:\>_ on screen and a floppy that says GOOD GAMES NEVER DIE" />
</p>

---

## A tribute to eXoDOS

eXorchy would not exist without the extraordinary work of the
[eXoDOS project](https://www.retro-exo.com/exodos.html) and its creator,
**eXo**. Over many years, eXo and the eXoDOS community have collected,
configured, tested and preserved thousands of DOS games, and then did it again
for Windows 3.x, Windows 9x and ScummVM titles, each one pre-configured to run
out of the box, with manuals, box art, soundtracks, magazines and books
alongside. The result is an irreplaceable archive of gaming history, kept
alive by people who ask nothing in return.

eXorchy is only a frontend for that collection. It does not host or distribute
any game files; everything it shows and plays comes from the eXo torrents that
you download and seed yourself. If you find value in eXorchy, please support
the eXoDOS project directly at [retro-exo.com](https://www.retro-exo.com).

## Thank you, Thomas

eXorchy is derived from [Exodium](https://github.com/tvollstaedt/exodium) by
**Thomas Vollstädt**, released under the MIT License. Almost everything that
makes eXorchy work is his: the bundled catalogue instead of eXo's 5 GB metadata
archive, streaming one game at a time out of the torrents, translating eXo's
DOSBox-ECE configurations for DOSBox Staging, the language-pack overlays, the
save-preserving uninstall, the Windows 9x and ScummVM launchers, the preview
player and the reading room, and the hundreds of tests and written-down
decisions that explain why each of them works the way it does. eXorchy gives
that codebase an Omarchy home: Linux-only, themed by Omarchy, packaged for
Arch. Thank you, Thomas. If eXorchy is useful to you, consider supporting his
work on [GitHub Sponsors](https://github.com/sponsors/tvollstaedt) or
[Ko-fi](https://ko-fi.com/tvollstaedt). See `ACKNOWLEDGEMENTS.md` for the
full list of credits.

---

## What it does

- Every eXo collection: eXoDOS, the German, Spanish and Polish language packs,
  eXoWin3x, eXoWin9x, eXoScummVM, plus the Media Pack reading room. Only
  eXoDOS is on at first; the others are switches in Settings → Collections
  and stay invisible until enabled.
- Streams single games on demand from the eXo torrents (no full download);
  downloads resume after a restart, seeding is opt-in.
- DOSBox Staging 0.83.0 downloaded automatically (29 MB) or your own
  `dosbox-staging`; DOSBox-X and 86Box fetched for Windows 9x; ScummVM builds
  pinned per game.
- eXo's DOSBox-ECE configs translated for Staging; MT-32 and General MIDI via
  the ROMs and soundfont fetched from the collection.
- Preview videos and theme music streamed straight out of the archives, a
  gallery of box scans and screenshots, manuals and the reading room's
  magazines rendered in-app (poppler).
- Per-game settings, favourites, playlists, My Library, save-preserving
  uninstall, storage overview.
- A native GTK4 / libadwaita window: no web view, no title bar (Hyprland
  draws the borders and tiles it), a virtualised grid that stays smooth on
  11,000 games, keyboard-first navigation.
- Omarchy theming: palette, light/dark mode, font and base size from the
  current theme, updated the moment you run `omarchy theme set`.

## Install

Requirements: Omarchy, or any Arch with `gtk4`, `libadwaita`, `poppler-glib`
and GStreamer. Preview videos and music need `gst-plugins-good` and
`gst-libav` (the players hide themselves when GStreamer cannot decode).

```bash
# system-wide, from a checkout
cd packaging && makepkg -si
# or user-level under ~/.local (needs rustup)
packaging/install-dev.sh
```

Then launch **eXorchy** from the Omarchy launcher (Super+Space) or run
`exorchy`. Optional: `omarchy pkg aur add dosbox-staging-bin` and enable
"Prefer system dosbox-staging" in Settings → Emulators.

## Keyboard

| Key | Action |
|---|---|
| `/` | focus the search field |
| arrows, Page Up/Down, Home/End | move through the grid or list |
| Enter (or double-click) | open the selected game in the detail panel |
| Esc | close the detail panel |
| Ctrl+, | open Settings |

## Build from source

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release -p exorchy          # target/release/exorchy
cargo run -p exorchy                      # run from the checkout (resources resolve to it)
cargo test -p exorchy-core                # backend tests
cargo test -p exorchy                     # UI unit tests
cargo clippy --workspace --all-targets -- -D warnings
```

`RUST_LOG=exorchy_core=debug,librqbit=info` makes the log verbose;
`EXORCHY_SNAPSHOT=/tmp/shot.png:4000` renders the window to a PNG and quits
(a developer aid for checking layouts without a compositor screenshot).

Documentation for contributors and future agents lives in `docs/`:
`HANDOVER.md`, `ARCHITECTURE.md`, `DECISIONS.md`, `COLLECTIONS.md`,
`PORTING.md`.

## Credits

Besides eXoDOS and Exodium (above): the Omarchy integration took inspiration
from [omakade](https://github.com/btsouth/omakade), a native Omarchy front
end for emulators. omakade is GPL-licensed and eXorchy shares no code with
it; what carried over is the idea that an Omarchy app should be a native
window that reads the theme directory directly.

## License

MIT. Copyright (c) 2026 Renan Tonheiro; portions copyright (c) 2026 Thomas
Vollstädt (Exodium). DOSBox Staging is GPL-2.0 and downloaded separately from
its own releases.
