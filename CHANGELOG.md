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

## [0.5.0] - 2026-10-02

A new look, modelled on eXorchy's concept art, and a round of fixes.

### Added
- A left sidebar to browse the library by platform, genre, publisher, series,
  year, region, tag, play status or playlist. What you pick shows as a chip at
  the top that you can clear on its own; sort and grid/list sit on the right.
- The **Game Dossier** replaces the detail panel. The cover, title and actions
  stay in place while you scroll, the tabs are Overview, Media, Manuals and
  Setup, and a click on the cover shows it at full size.
- A status bar along the bottom with the counts for the page you are on and
  its keyboard shortcuts.
- **Settings → Appearance → Interface size**: Compact, Medium (the default),
  Large and Extra large.
- The Reading Room works like the game library: the same sidebar, with a
  **Downloaded** entry for what you already have, and the same filter chips.
  An issue opens in place of the shelves with the whole page in view, and the
  full-screen button (or F11) gives it the whole screen. Game manuals open the
  same way.
- A new splash screen in your theme's colours, shown while eXorchy starts.
- DOS and Windows 3.x games that eXo runs under DOSBox-X now run under it in
  eXorchy too, in a large floating window on Hyprland. Games that print save
  each page as a picture in the game's `!prints` folder.
- Your game library lives in `~/Games/eXorchy` by default. A library in the
  old place is offered a move when eXorchy starts, and **Settings → General**
  can move it whenever you like.

### Changed
- Theme music no longer starts by itself: press Play in the dossier's Theme
  row and it plays there, with its own progress bar, and stops when you move
  to another game. The music bar at the bottom of the window is gone.
- The selected game has a stronger frame, and Reading Room covers are spaced
  like game covers.
- Installed games and the Play button use your theme's accent colour instead
  of a fixed green, and themes made before Omarchy's named palette (Aether and
  older ones) now get the right colours.

### Fixed
- eXorchy could close suddenly when you clicked quickly from game to game
  while preview videos or theme music were playing.
- Playlists did nothing: creating one, renaming it or adding a game to it
  now works.
- Games whose CD image names its files in a different upper/lower case, such
  as SimCopter, said the CD was missing.
- A magazine you have downloaded now opens while eXorchy is offline.
- The preview video in the dossier no longer changes size between tabs, and
  **Play preview** works again on a game whose preview you watched before.
- Cover art in the dossier keeps its proportions, and DOS title screens show
  at their original 4:3 shape.
- Sorting by rating now shows its jump bar.

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
