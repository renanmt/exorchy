//! The Reading Room (the eXoDOS Media Pack: magazines, books, catalogues
//! and disk magazines) and its entry points for the rest of the app.
//!
//! - `logic`  pure filters, sort orders, sections, wording (unit tested)
//! - `store`  catalogue, on-disk set, fetch jobs with their 1 Hz poll
//! - `card`   an issue as a grid card or a list row
//! - `reader` the issue reader dialog (fetch panel → document viewer) and
//!   the media notice
//! - `room`   the tab itself and the detail panel's "Covered in" section
//!
//! The document viewer proper lives in `ui::pdf`.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

pub mod card;
pub mod logic;
pub mod reader;
pub mod room;
pub mod store;

use room::Room;

thread_local! {
    static ROOM: RefCell<Option<Rc<Room>>> = const { RefCell::new(None) };
}

/// The Reading Room tab. Built once per library page.
pub fn build(window: &gtk::Window) -> gtk::Widget {
    let room = Room::new(window);
    ROOM.with(|r| *r.borrow_mut() = Some(room.clone()));

    // Developer aids (see snapshot.rs): EXORCHY_SNAPSHOT_READING switches to
    // this tab after 1 s; EXORCHY_SNAPSHOT_DOC=<file> opens the viewer on it.
    if std::env::var_os("EXORCHY_SNAPSHOT_READING").is_some() {
        glib::timeout_add_local_once(Duration::from_secs(1), || {
            if let Some(lib) = crate::ui::window::library() {
                lib.set_tab("reading");
            }
        });
    }
    if let Some(path) = std::env::var("EXORCHY_SNAPSHOT_DOC").ok().filter(|p| !p.is_empty()) {
        let window = window.clone();
        glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Document".into());
            crate::ui::pdf::open_document_viewer(&window, &path, &name, None, 1);
        });
    }
    // EXORCHY_SNAPSHOT_READING_VIEW=list switches the room to the list.
    if std::env::var("EXORCHY_SNAPSHOT_READING_VIEW").ok().as_deref() == Some("list") {
        let r = room.clone();
        glib::timeout_add_local_once(Duration::from_millis(1200), move || r.set_view(false));
    }
    // EXORCHY_SNAPSHOT_ISSUE=<key> opens the reader on that issue.
    if let Some(key) = std::env::var("EXORCHY_SNAPSHOT_ISSUE").ok().filter(|k| !k.is_empty()) {
        let window = window.clone();
        glib::timeout_add_local_once(Duration::from_millis(2500), move || {
            if let Some(issue) = store::issue(&key) {
                reader::open(&window, issue, None);
            }
        });
    }
    room.widget.clone().upcast()
}

/// The top bar's search box, when the library forwards it to this tab.
#[allow(dead_code)]
pub fn set_query(q: &str) {
    if let Some(room) = ROOM.with(|r| r.borrow().clone()) {
        room.set_query(q);
    }
}

/// The "Covered in" section for a game's detail panel: eXo's article index
/// (review, preview, ...) with the page each one starts on. Returns an
/// invisible widget until (and unless) articles arrive.
#[allow(dead_code)]
pub fn game_articles_widget(game_id: i64, window: &gtk::Window) -> gtk::Widget {
    room::game_articles_widget(game_id, window)
}

