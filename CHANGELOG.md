# Changelog

What changed in each eXorchy release, newest first. The section for a
version becomes its GitHub release notes, so write it for the people who use
eXorchy: what they will notice, not how it was done.

<!--
Adding a release: put a new "## [X.Y.Z] - YYYY-MM-DD" section at the top with
Added / Changed / Fixed lists (leave out the empty ones), in the same pull
request that raises the version. The release workflow publishes that section
and refuses to release a tag that has none.
-->

## [0.4.0] - 2026-09-29

### Added
- eXorchy now tells you when a new release is out: a banner at the top of the
  library offers **Update**, **What's new** and **Skip this version**. Update
  opens a terminal that installs the new version (pacman asks for your
  password) and starts eXorchy again.
- Settings → General → **Check for updates** turns the check off;
  Settings → About shows your version, **Check now** and **Update**.

### Fixed
- Starring a game in the detail panel no longer makes the panel flicker.
  Thanks to the Reddit user who reported it.

## [0.3.0] - 2026-09-29

### Changed
- A squarer look that matches Omarchy's windows: no rounded corners anywhere,
  and slightly thicker 2 px outlines on cards, panels, buttons and fields.

### Fixed
- The per-game settings dialog opened as a thin strip showing a single row;
  it now opens at full size with every setting visible.

## [0.2.0] - 2026-09-28

The first public release of the native eXorchy.

### Added
- A native GTK4 / libadwaita app (no web view) for the eXo collections:
  eXoDOS, eXoWin3x, eXoWin9x and eXoScummVM, the German, Spanish and Polish
  language packs, and the Media Pack reading room.
- Download single games straight from eXo's torrents and play them with one
  click; DOSBox Staging, DOSBox-X, 86Box and ScummVM are fetched for you.
- Follows your Omarchy theme live, including a pixel-art eXorchy wordmark in
  the theme's colours, and adapts to any Hyprland tile size.
- Settings as a full page, a Transfers page (click the connection badge) with
  downloads, queue and every torrent's speed, peers and upload.
- Hide titles you do not want to see (installed ones stay searchable and
  playable); adult titles are hidden until you switch them on.
- Search in My Library looks through your installed games.
- Your own background image with an opacity slider, and an option to open
  straight into My Library.
- A one-line installer for Arch / Omarchy using a prebuilt pacman package.
