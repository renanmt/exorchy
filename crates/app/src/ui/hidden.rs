//! Hidden titles and the adult filter, over the backend's `hidden_games`
//! table and `show_adult` switch: the hidden-id set the game menus read,
//! hide / unhide with an Undo toast, taking a game off Recently played, and
//! the switch. Every change re-reads the lists through
//! `bus::notify_visibility_changed`. A hidden installed game stays findable
//! by name and playable; the queries enforce that, this module only asks.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use exorchy_core::commands::games;

use crate::app;
use crate::ui::bus;

thread_local! {
    /// Every row id of every hidden group (`get_hidden_ids`).
    static HIDDEN: RefCell<HashSet<i64>> = RefCell::new(HashSet::new());
}

/// Read the hidden set (library start, and after every change).
pub fn load() {
    let core = app::core();
    app::spawn(async move { games::get_hidden_ids(core.state()).await }, |res| match res {
        Ok(ids) => HIDDEN.with(|h| *h.borrow_mut() = ids.into_iter().collect()),
        Err(e) => log::error!("hidden titles: {e}"),
    });
}

pub fn is_hidden(id: i64) -> bool {
    HIDDEN.with(|h| h.borrow().contains(&id))
}

/// Re-read the set, then tell the lists.
fn changed() {
    let core = app::core();
    app::spawn(async move { games::get_hidden_ids(core.state()).await }, |res| {
        if let Ok(ids) = res {
            HIDDEN.with(|h| *h.borrow_mut() = ids.into_iter().collect());
        }
        bus::notify_visibility_changed();
    });
}

/// Hide a title; the toast offers Undo.
pub fn hide(id: i64, title: &str) {
    let core = app::core();
    let title = title.to_string();
    app::spawn(async move { games::hide_game(core.state(), id).await }, move |res| match res {
        Ok(()) => {
            changed();
            let undo_title = title.clone();
            bus::toast_with(
                &format!("Hidden: {title}"),
                Some("Unhide it in Settings → Hidden titles."),
                Some(("Undo", Rc::new(move || unhide(id, &undo_title)))),
            );
        }
        Err(e) => bus::toast(&format!("Could not hide {title}: {e}")),
    });
}

pub fn unhide(id: i64, title: &str) {
    let core = app::core();
    let title = title.to_string();
    app::spawn(async move { games::unhide_game(core.state(), id).await }, move |res| match res {
        Ok(()) => {
            changed();
            bus::toast(&format!("Shown again: {title}"));
        }
        Err(e) => bus::toast(&format!("Could not unhide {title}: {e}")),
    });
}

/// Take a game off Recently played (its play history only).
pub fn remove_from_recent(id: i64, title: &str) {
    let core = app::core();
    let title = title.to_string();
    app::spawn(async move { games::remove_from_recently_played(core.state(), id).await }, move |res| match res {
        Ok(()) => bus::notify_visibility_changed(),
        Err(e) => bus::toast(&format!("Could not remove {title} from Recently played: {e}")),
    });
}

/// Switch adult titles on or off; `done` gets the outcome (the settings
/// switch rolls back on an error).
pub fn set_show_adult(on: bool, done: impl FnOnce(Result<(), String>) + 'static) {
    let core = app::core();
    app::spawn(async move { games::set_show_adult(core.state(), on).await }, move |res| {
        if res.is_ok() {
            bus::notify_visibility_changed();
        }
        done(res);
    });
}
