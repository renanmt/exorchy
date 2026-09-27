//! Preview videos and theme music, natively: the preview under the cover in
//! the detail panel (fetch status, frames, play/pause/mute, lightbox), the
//! Theme row, the now-playing bar under the library and the toolbar's ♪.
//!
//! - `store`: fetch jobs + scheduler + the player state machine (`videos.ts`,
//!   `mediaQueue.ts`, `music.ts`)
//! - `audio`: the one `gtk::MediaFile` the player drives, with fades
//! - `preview`: the hero controller (`heroVideo.ts`), the panel slot, the lightbox
//! - `bar`: the now-playing bar and the toolbar button
//!
//! Playback is GTK's GStreamer backend, so the cached files play by path -
//! the web UI's localhost media server is not used. Where GStreamer cannot
//! decode (no audio sink, no H.264 or MP3 decoder) the backend's probes
//! stand the feature down and the panel note says why; an element error on
//! a file that got through is shown in place ("Preview can't play - …",
//! "Can't play this track - …").
//!
//! Developer aid: `EXORCHY_MEDIA_ASSUME_SUPPORTED=1` skips the probes.

mod audio;
mod bar;
mod preview;
mod store;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;

use crate::ui::bus;
use crate::ui::library::LibraryPage;

/// For the rows and the panel note: the Browse list's play buttons and the
/// list queue (`play_from_list` walks the list `set_list_source` provides),
/// `video_supported` feeds the "no GStreamer" launch note.
#[allow(unused_imports)]
pub use store::{play_from_list, play_theme, playable_hint, set_list_source, video_supported, Track};

thread_local! {
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    /// The bar of the current library page; the previous one goes with its page.
    static BAR: RefCell<Option<Rc<bar::Bar>>> = const { RefCell::new(None) };
}

/// Mount the media features: the preview slot in the detail panel, the
/// toolbar button and the now-playing bar under the library. The player
/// itself is created once; a re-created library page gets new widgets on
/// the same state.
pub fn install(window: &gtk::Window, library: &Rc<LibraryPage>, bar_slot: &gtk::Box) {
    if !INSTALLED.with(|c| c.replace(true)) {
        store::init();
        // The games running: each launch is a reason to stay quiet, the last
        // exit (the backend's `game-exited`, relayed by the bus) resumes.
        bus::on_running_changed(store::sync_running);
    }

    while let Some(c) = bar_slot.first_child() {
        bar_slot.remove(&c);
    }
    let bar = bar::Bar::new(library);
    bar_slot.append(&bar.root);
    BAR.with(|b| *b.borrow_mut() = Some(bar));

    library.toolbar_slot.append(&bar::toolbar_button());

    let detail = library.detail();
    let preview = preview::Preview::new(window);
    detail.media_slot.append(&preview.root);
    detail.on_shown(move |g| preview.on_shown(g));
}
