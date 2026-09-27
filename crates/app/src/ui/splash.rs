//! The splash: the key art on its own dark backdrop, shown on every start
//! until the app knows what to render and at least `MIN_MS` have passed. An
//! overlay inside the window, not a second window: Hyprland would tile one.

use gtk::glib;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;

const ART: &[u8] = include_bytes!("../../assets/splash.jpg");
const MIN_MS: u64 = 1600;

pub struct Splash {
    pub widget: gtk::Revealer,
    shown_at: Instant,
    released: Rc<Cell<bool>>,
}

impl Splash {
    pub fn new() -> Self {
        let bytes = glib::Bytes::from_static(ART);
        let picture = match gtk::gdk::Texture::from_bytes(&bytes) {
            Ok(t) => gtk::Picture::for_paintable(&t),
            Err(e) => {
                log::warn!("splash art unreadable: {e}");
                gtk::Picture::new()
            }
        };
        // Contain, not cover: in a narrow Hyprland tile the art scales down
        // whole on its own dark backdrop instead of being cropped.
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        picture.add_css_class("splash");

        let widget = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .transition_duration(450)
            .reveal_child(true)
            .child(&picture)
            .build();
        Splash { widget, shown_at: Instant::now(), released: Rc::new(Cell::new(false)) }
    }

    /// The app is ready: fade out once the minimum time has passed, then
    /// take the overlay out of the way of input.
    pub fn release(&self) {
        if self.released.replace(true) {
            return;
        }
        let elapsed = self.shown_at.elapsed();
        let wait = Duration::from_millis(MIN_MS).saturating_sub(elapsed);
        let revealer = self.widget.clone();
        glib::timeout_add_local_once(wait, move || {
            revealer.set_reveal_child(false);
            let r = revealer.clone();
            glib::timeout_add_local_once(Duration::from_millis(500), move || {
                r.set_visible(false);
                r.set_can_target(false);
            });
        });
    }
}
