//! Playlist management: "Add to playlist…", create / rename / delete. Stub.

use exorchy_core::models::Game;
use gtk::prelude::*;

pub fn pick_for_game(parent: &impl IsA<gtk::Widget>, game: &Game) {
    crate::ui::dialogs::error(parent, "Playlists", &format!("Playlists are being ported ({}).", game.title));
}
