# Acknowledgements

## eXoDOS and eXo

Exorchy exists because of the eXoDOS project (https://www.retro-exo.com) and
its creator, eXo. For well over a decade, eXo and the eXoDOS community have
collected, configured, tested and preserved thousands of DOS, Windows 3.x,
Windows 9x and ScummVM games, each one set up to run out of the box, together
with manuals, box art, soundtracks, magazines and books. That archive is a
piece of gaming history that would otherwise be scattered or lost.

Exorchy does not host or distribute any of it. Everything comes from the
official eXo torrents that you download and seed yourself; the catalogue,
the DOSBox configurations and the media in this app are eXo's work. If
Exorchy is useful to you, please support the eXoDOS project directly at
https://www.retro-exo.com.

## Exodium and Thomas Vollstädt

Exorchy is a derivative of **Exodium** (https://github.com/tvollstaedt/exodium),
written by **Thomas Vollstädt** and released under the MIT License.

Almost everything that makes Exorchy work was designed and built by Thomas:
the idea of shipping a small bundled catalogue instead of eXo's multi-gigabyte
metadata archive, streaming single games out of the torrents with selective
downloads, translating eXo's DOSBox-ECE configurations for DOSBox Staging, the
language-pack overlay mounts, the save-preserving uninstall, the Windows 9x
and ScummVM launchers, the preview player, the reading room, and the hundreds
of tests and documented decisions that record why each of those works the way
it does. Exorchy takes that codebase and gives it an Omarchy home: a Linux-only
build, Omarchy's live theming, XDG paths, native packaging and an emulator
provisioning path that fits Arch.

Thank you, Thomas. The care in Exodium's code and documentation is the reason
this port could be made without breaking what you got right.

Exodium's copyright notice is preserved in `LICENSE`. Consider supporting his
work at https://github.com/sponsors/tvollstaedt or https://ko-fi.com/tvollstaedt.

## Emulators and libraries

- DOSBox Staging (https://www.dosbox-staging.org, GPL-2.0), downloaded from its
  official releases; DOSBox-X and 86Box for Windows 9x titles; ScummVM.
- librqbit by Igor Katson (the BitTorrent engine), with a patch carried from
  Thomas Vollstädt's fork.
- Tauri, SolidJS, Ark UI, pdf.js, and the Rust and TypeScript ecosystems.
