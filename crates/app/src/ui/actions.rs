//! The game actions every surface shares: download, play, stop, uninstall,
//! reset, favourite, manual. Each one owns its toasts and library-change
//! notifications, so a card menu and the detail panel behave alike.

use std::rc::Rc;

use exorchy_core::commands::{games, library};
use exorchy_core::models::Game;
use gtk::glib;
use gtk::prelude::*;

use crate::app;
use crate::ui::{bus, dialogs, downloads};

pub fn download(game: &Game) {
    let Some(id) = game.id else { return };
    let title = if game.available_languages.as_deref().map(|l| l.contains(',')).unwrap_or(false) {
        format!("{} [{}]", game.title, game.language)
    } else {
        game.title.clone()
    };
    downloads::start(id, Some(title));
}

/// Launch; `busy(true)` while the emulator window comes up (4 s minimum so
/// the spinner is seen), `busy(false)` after.
pub fn play(id: i64, title: String, busy: Rc<dyn Fn(bool)>) {
    busy(true);
    let started = std::time::Instant::now();
    let core = app::core();
    app::spawn(async move { games::launch_game(core.clone(), core.state(), id).await }, move |res| {
        match res {
            Ok(_) => {
                bus::mark_running(id, true);
                let remaining = std::time::Duration::from_secs(4).saturating_sub(started.elapsed());
                let busy = busy.clone();
                glib::timeout_add_local_once(remaining, move || busy(false));
            }
            Err(e) => {
                busy(false);
                let detail = e.trim_start_matches("Error: ").to_string();
                bus::toast_with(&format!("Couldn't launch {title}"), Some(&detail), None);
            }
        }
    });
}

pub fn stop(id: i64) {
    app::spawn(async move { games::stop_game(id).await }, move |res| {
        if let Err(e) = res {
            bus::toast_with("Couldn't stop the game", Some(&e), None);
        }
    });
}

/// Uninstall one row: cancel its download first, then remove. The backend
/// says what it kept ("saved data kept (38 MB)").
pub fn uninstall(id: i64, title: String, status: Rc<dyn Fn(&str)>, done: Rc<dyn Fn(bool)>) {
    if downloads::is_downloading(id) {
        status("Cancelling download…");
        downloads::cancel(id);
    }
    downloads::stop_tracking(id);
    status("Uninstalling...");
    let core = app::core();
    app::spawn(async move { library::uninstall_game(core.state(), core.state(), id).await }, move |res| {
        status("");
        match res {
            Ok(msg) => {
                bus::notify_library_changed(id);
                bus::toast(&if msg.is_empty() { format!("Uninstalled {title}") } else { msg });
                done(true);
            }
            Err(e) => {
                bus::toast_with(&format!("Couldn't uninstall {title}"), Some(&e), None);
                done(false);
            }
        }
    });
}

/// Every installed row of a merged card, the given one first.
pub fn installed_group_ids(game: &Game, done: impl FnOnce(Vec<i64>) + 'static) {
    let Some(self_id) = game.id else {
        done(vec![]);
        return;
    };
    let (Some(sc), Some(src)) = (game.shortcode.clone(), game.torrent_source.clone()) else {
        done(vec![self_id]);
        return;
    };
    let core = app::core();
    app::spawn(async move { games::get_game_variants(core.state(), core.state(), sc, src).await }, move |res| {
        let rows = res.unwrap_or_default();
        let mut ids: Vec<i64> = rows.iter().filter(|r| r.installed || r.in_library).filter_map(|r| r.id).collect();
        if ids.is_empty() {
            ids.push(self_id);
        } else if ids.contains(&self_id) {
            ids.retain(|i| *i != self_id);
            ids.insert(0, self_id);
        }
        done(ids);
    });
}

/// Confirm, then uninstall every installed row of the group.
pub fn uninstall_group(parent: &impl IsA<gtk::Widget>, game: &Game, status: Rc<dyn Fn(&str)>) {
    let g = game.clone();
    let parent = parent.clone();
    installed_group_ids(game, move |ids| {
        let n = ids.len().max(1);
        let body = if n > 1 {
            format!("Remove {} installed versions of {}? Save games are kept and restored on reinstall.", n, g.title)
        } else {
            format!("Remove {}? Save games are kept and restored on reinstall.", g.title)
        };
        let title = g.title.clone();
        dialogs::confirm(&parent, "Uninstall", &body, "Uninstall", true, move || {
            uninstall_chain(ids, title, status);
        });
    });
}

fn uninstall_chain(mut ids: Vec<i64>, title: String, status: Rc<dyn Fn(&str)>) {
    if ids.is_empty() {
        return;
    }
    let id = ids.remove(0);
    let (t2, s2) = (title.clone(), status.clone());
    let next = Rc::new(move |_ok: bool| uninstall_chain(ids.clone(), t2.clone(), s2.clone()));
    uninstall(id, title, status, next);
}

/// Reset game data (confirming first): discard saves and unpack the game
/// again; when the archive is gone the download tracker unpacks a new copy.
pub fn reset(parent: &impl IsA<gtk::Widget>, game: &Game, status: Rc<dyn Fn(&str)>) {
    let Some(id) = game.id else { return };
    let title = game.title.clone();
    dialogs::confirm(
        parent,
        "Reset game data",
        &format!("Discard saves and every in-game change of {title}, then unpack the game again?"),
        "Discard and reset",
        true,
        move || {
            status("Resetting…");
            let core = app::core();
            let title = title.clone();
            let status = status.clone();
            app::spawn(async move { library::reset_game_data(core.state(), core.state(), id).await }, move |res| {
                status("");
                match res {
                    Ok(o) => {
                        if o.redownload {
                            bus::notify_library_changed(id);
                            downloads::start(id, Some(title));
                        }
                        bus::toast(&o.message);
                    }
                    Err(e) => bus::toast_with(&format!("Couldn't reset {title}"), Some(&e), None),
                }
            });
        },
    );
}

pub fn toggle_favorite(id: i64, done: impl FnOnce(Result<bool, String>) + 'static) {
    let core = app::core();
    app::spawn(async move { games::toggle_favorite(core.state(), id).await }, done);
}

/// Open a manual or image with the desktop's default application.
pub fn open_document(path: String) {
    let core = app::core();
    app::spawn(async move { games::open_document(core.clone(), core.state(), path).await }, |res| {
        if let Err(e) = res {
            bus::toast_with("Couldn't open the document", Some(&e), None);
        }
    });
}

/// "Add to playlist…": the playlist picker (`ui::playlists`).
pub fn add_to_playlist(parent: &impl IsA<gtk::Widget>, game: &Game) {
    crate::ui::playlists::pick_for_game(parent, game);
}

/// "Game settings…": per-game emulator options (`ui::game_settings`).
pub fn game_settings(parent: &impl IsA<gtk::Widget>, game: &Game) {
    crate::ui::game_settings::open(parent, game);
}
