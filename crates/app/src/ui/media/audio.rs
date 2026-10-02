//! The music player's audio element: a `PlaybinStream` (GStreamer), driven
//! by the store's `PortOp`s. Fades: a start ramps up from silence, a pause ramps down and only then pauses the
//! element - a track cut off at full volume is a pop, not a pause. Each new
//! source gets a fresh element (see `fresh_stream`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;

use super::store::{self, PortOp};
use super::playbin::PlaybinStream;

const FADE_IN_MS: u64 = 600;
const FADE_OUT_MS: u64 = 250;
const FADE_TICK_MS: u64 = 40;
/// Safety net for a stream that never reports `prepared`: silence must not
/// be the price of a missing event.
const FADE_IN_FALLBACK_MS: u64 = 400;

pub struct AudioPort {
    stream: RefCell<PlaybinStream>,
    fade: RefCell<Option<glib::SourceId>>,
    fallback: Cell<Option<glib::SourceId>>,
    /// The listener's volume preference; every ramp ends there.
    fade_target: Cell<f64>,
    /// Armed by play(): the ramp begins when sound actually flows.
    fade_in_armed: Cell<bool>,
}

impl AudioPort {
    pub fn new() -> Rc<Self> {
        let port = Rc::new(AudioPort {
            stream: RefCell::new(PlaybinStream::new_audio()),
            fade: RefCell::new(None),
            fallback: Cell::new(None),
            fade_target: Cell::new(0.8),
            fade_in_armed: Cell::new(false),
        });
        let stream = port.stream.borrow().clone();
        port.wire(&stream);
        port
    }

    pub fn stream(&self) -> PlaybinStream {
        self.stream.borrow().clone()
    }

    fn wire(self: &Rc<Self>, stream: &PlaybinStream) {
        let weak = Rc::downgrade(self);
        stream.connect_ended_notify(move |s| {
            if s.is_ended() {
                if let Some(p) = weak.upgrade() {
                    p.stop_fade();
                }
                store::port_ended();
            }
        });
        stream.connect_error_notify(move |s| {
            if let Some(e) = s.error() {
                store::port_error(e.message().to_string());
            }
        });
        let weak = Rc::downgrade(self);
        stream.connect_prepared_notify(move |s| {
            if s.is_prepared() {
                if let Some(p) = weak.upgrade() {
                    p.begin_armed_fade_in();
                }
            }
        });
        let weak = Rc::downgrade(self);
        stream.connect_playing_notify(move |s| {
            if s.is_playing() {
                if let Some(p) = weak.upgrade() {
                    if s.is_prepared() {
                        p.begin_armed_fade_in();
                    }
                }
                store::port_playing();
            }
        });
    }

    /// Every source gets a fresh element: GTK keeps a stream that failed in
    /// its error state for good. The old element is stopped and dropped.
    fn fresh_stream(self: &Rc<Self>) -> PlaybinStream {
        let current = self.stream.borrow().clone();
        if current.file().is_none() && current.error().is_none() {
            return current;
        }
        current.pause();
        current.clear();
        let next = PlaybinStream::new_audio();
        self.wire(&next);
        *self.stream.borrow_mut() = next.clone();
        next
    }

    pub fn apply(self: &Rc<Self>, op: PortOp) {
        match op {
            PortOp::SetSrc(path) => {
                self.stop_fade();
                let stream = self.fresh_stream();
                match path {
                    Some(p) => stream.set_filename(Some(std::path::Path::new(&p))),
                    None => stream.clear(),
                }
            }
            PortOp::Play => {
                self.stop_fade();
                let stream = self.stream();
                if stream.file().is_none() {
                    return;
                }
                stream.set_volume(0.0);
                self.fade_in_armed.set(true);
                if stream.is_ended() && stream.is_seekable() {
                    stream.seek(0);
                }
                stream.play();
                let weak = Rc::downgrade(self);
                self.fallback.set(Some(glib::timeout_add_local_once(Duration::from_millis(FADE_IN_FALLBACK_MS), move || {
                    if let Some(p) = weak.upgrade() {
                        p.fallback.set(None);
                        p.begin_armed_fade_in();
                    }
                })));
            }
            PortOp::Pause => {
                let stream = self.stream();
                // Nothing loaded (a pause reason recorded before any track):
                // there is no element to pause, and touching one asserts.
                if stream.file().is_none() {
                    self.stop_fade();
                    return;
                }
                // Not yet audibly playing (a start still warming up, or
                // already paused): pause at once.
                if !stream.is_playing() || self.fade_in_armed.get() {
                    self.stop_fade();
                    stream.pause();
                    stream.set_volume(self.fade_target.get());
                    return;
                }
                let weak = Rc::downgrade(self);
                self.fade(0.0, FADE_OUT_MS, Some(Box::new(move || {
                    if let Some(p) = weak.upgrade() {
                        let s = p.stream();
                        s.pause();
                        s.set_volume(p.fade_target.get());
                    }
                })));
            }
            PortOp::SetVolume(v) => {
                self.fade_target.set(v);
                // GTK's GStreamer element only takes a volume once it has a
                // file (it asserts on an empty stream); `Play` applies it.
                let stream = self.stream();
                if self.fade.borrow().is_none() && stream.file().is_some() {
                    stream.set_volume(v);
                }
            }
        }
    }

    fn begin_armed_fade_in(self: &Rc<Self>) {
        if !self.fade_in_armed.replace(false) {
            return;
        }
        self.fade(self.fade_target.get(), FADE_IN_MS, None);
    }

    fn stop_fade(&self) {
        self.fade_in_armed.set(false);
        if let Some(src) = self.fade.take() {
            src.remove();
        }
        if let Some(src) = self.fallback.take() {
            src.remove();
        }
    }

    fn fade(self: &Rc<Self>, to: f64, ms: u64, done: Option<Box<dyn FnOnce()>>) {
        self.stop_fade();
        let stream = self.stream();
        let from = stream.volume();
        let t0 = Instant::now();
        let weak = Rc::downgrade(self);
        let done = Cell::new(done);
        self.fade.replace(Some(glib::timeout_add_local(Duration::from_millis(FADE_TICK_MS), move || {
            let k = (t0.elapsed().as_millis() as f64 / ms as f64).min(1.0);
            stream.set_volume(from + (to - from) * k);
            if k < 1.0 {
                return glib::ControlFlow::Continue;
            }
            if let Some(p) = weak.upgrade() {
                // This source is ending on its own; forget it without a remove.
                p.fade.replace(None);
            }
            if let Some(f) = done.take() {
                f();
            }
            glib::ControlFlow::Break
        })));
    }
}

impl Drop for AudioPort {
    fn drop(&mut self) {
        self.stop_fade();
    }
}
