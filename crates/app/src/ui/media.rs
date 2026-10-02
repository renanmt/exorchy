//! Preview videos and theme music, natively: the preview under the cover in
//! the detail panel (fetch status, frames, play/pause/mute, lightbox) and
//! the Theme row, which plays a game's theme in place.
//!
//! - `store`: fetch jobs + scheduler + the player state machine (`videos.ts`,
//!   `mediaQueue.ts`, `music.ts`)
//! - `audio`: the music player's media element, with fades
//! - `playbin`: the media element itself, a `gtk::MediaStream` over GStreamer's
//!   classic `playbin` (GTK's built-in backend is playbin3, which aborts)
//! - `preview`: the hero controller (`heroVideo.ts`), the panel slot, the lightbox
//!
//! Playback is GStreamer (`playbin`), so the cached files play by path -
//! the web UI's localhost media server is not used. Where GStreamer cannot
//! decode (no audio sink, no H.264 or MP3 decoder) the backend's probes
//! stand the feature down and the panel note says why; an element error on
//! a file that got through is shown in place ("Preview can't play - …",
//! "Can't play this track - …").
//!
//! Developer aid: `EXORCHY_MEDIA_ASSUME_SUPPORTED=1` skips the probes.

mod audio;
mod playbin;
mod preview;
mod store;

/// For the Transfers page: the preview and theme fetches and their kind.
pub use store::{fetches, Kind};

use std::cell::Cell;
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
}

/// Mount the media features: the preview slot in the detail panel. The
/// player itself is created once; a re-created library page gets new widgets on
/// the same state.
pub fn install(window: &gtk::Window, library: &Rc<LibraryPage>) {
    if !INSTALLED.with(|c| c.replace(true)) {
        store::init();
        // The games running: each launch is a reason to stay quiet, the last
        // exit (the backend's `game-exited`, relayed by the bus) resumes.
        bus::on_running_changed(store::sync_running);
    }

    let detail = library.detail();
    let preview = preview::Preview::new(window);
    detail.media_slot.append(&preview.root);
    detail.on_shown(move |g| preview.on_shown(g));
}
