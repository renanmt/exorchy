//! Per-game emulator settings (shader, fullscreen, cycles, custom conf;
//! ScummVM options). Stub.

use exorchy_core::models::Game;
use gtk::prelude::*;

pub fn open(parent: &impl IsA<gtk::Widget>, game: &Game) {
    crate::ui::dialogs::error(parent, "Game settings", &format!("Game settings are being ported ({}).", game.title));
}
