//! Windows, pages and widgets.
//!
//! - `window`    the one application window: splash, setup ↔ library stack, toasts, startup order
//! - `splash`    the key-art overlay shown on every start
//! - `setup`     first run: fresh data dir or import, network choice
//! - `library`   Browse / My Library / Reading Room, filters, grid and list, jump bar
//! - `card`      one game in the grid or on a shelf
//! - `detail`    the game detail panel
//! - `model`     `GameObject`, a `Game` row for `gio::ListStore`
//! - `covers`    cover art tiers and the texture cache
//! - `downloads` download trackers (1 Hz poll, extraction on completion)
//! - `actions`   download / play / stop / uninstall / reset / favourite / open
//! - `bus`       library-change bus, running set, offline flag, toasts
//! - `settings`  the Settings dialog and its pages
//! - `reading`   the Reading Room (Media Pack) and the PDF reader
//! - `media`     preview videos and theme music (now-playing bar)
//! - `playlists` playlist picker and management
//! - `game_settings` per-game emulator options
//! - `onboarding` welcome modal and seeding consent
//! - `dialogs`   confirm / folder picker / error
//! - `util`      formatting helpers shared by the pages

pub mod actions;
pub mod game_settings;
pub mod launch_notes;
pub mod media;
pub mod onboarding;
pub mod playlists;
pub mod reading;
pub mod settings;
pub mod bus;
pub mod card;
pub mod covers;
pub mod detail;
pub mod dialogs;
pub mod downloads;
pub mod library;
pub mod model;
pub mod setup;
pub mod splash;
pub mod util;
pub mod window;
