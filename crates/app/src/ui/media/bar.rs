//! The now-playing bar under the library (cover, title, transport, seek,
//! volume, hide) and the toolbar's music button. The bar shows once a track
//! is loaded, or while a track the listener asked for is still fetching; a
//! panel's autoplay probe is not such an ask.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use exorchy_core::commands::games;
use gtk::glib;
use gtk::prelude::*;

use crate::app;
use crate::ui::library::LibraryPage;
use crate::ui::{covers, window};

use super::store::{self, Change, Mode, PauseReason, PlayerView, Track, PHASE_QUEUED};

const TICK_MS: u64 = 250;
const COVER_PX: u32 = 80;

pub struct Bar {
    pub root: gtk::Box,
    cover: gtk::Picture,
    placeholder: gtk::Label,
    cover_btn: gtk::Button,
    title: gtk::Label,
    sub: gtk::Label,
    pos: gtk::Label,
    seek: gtk::Scale,
    dur: gtk::Label,
    prev_btn: gtk::Button,
    main_btn: gtk::Button,
    next_btn: gtk::Button,
    shuffle_btn: gtk::Button,
    cont_btn: gtk::Button,
    vol_icon: gtk::Image,
    vol: gtk::Scale,
    tick: RefCell<Option<glib::SourceId>>,
    cover_for: Cell<Option<i64>>,
    cover_gen: Cell<u64>,
    library: Weak<LibraryPage>,
}

impl Bar {
    pub fn new(library: &Rc<LibraryPage>) -> Rc<Self> {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).visible(false).css_classes(["now-playing-bar"]).build();

        let cover = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).width_request(40).height_request(40).can_shrink(true).build();
        let placeholder = gtk::Label::builder().label("♪").css_classes(["player-cover-placeholder"]).build();
        let cover_stack = gtk::Overlay::builder().child(&placeholder).build();
        cover_stack.add_overlay(&cover);
        let cover_btn = gtk::Button::builder().child(&cover_stack).css_classes(["player-cover"]).build();
        root.append(&cover_btn);

        let meta = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).valign(gtk::Align::Center).width_request(140).css_classes(["player-meta"]).build();
        let title = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["player-title"]).build();
        let sub = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["player-sub"]).build();
        meta.append(&title);
        meta.append(&sub);
        root.append(&meta);

        let seek_box = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).hexpand(true).valign(gtk::Align::Center).css_classes(["player-seek"]).build();
        let pos = gtk::Label::builder().label("--:--").css_classes(["player-time"]).build();
        let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.1);
        seek.set_draw_value(false);
        seek.set_hexpand(true);
        seek.set_sensitive(false);
        seek.add_css_class("player-seek-range");
        let dur = gtk::Label::builder().label("--:--").css_classes(["player-time"]).build();
        seek_box.append(&pos);
        seek_box.append(&seek);
        seek_box.append(&dur);
        root.append(&seek_box);

        let controls = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(4).valign(gtk::Align::Center).css_classes(["player-controls"]).build();
        let prev_btn = ctl("media-skip-backward-symbolic", "Previous game");
        let main_btn = ctl("media-playback-start-symbolic", "Play");
        main_btn.add_css_class("main");
        let next_btn = ctl("media-skip-forward-symbolic", "Next game");
        let shuffle_btn = ctl("media-playlist-shuffle-symbolic", "Shuffle all themes");
        let cont_btn = ctl("media-playlist-repeat-symbolic", "Continuous playback");
        cont_btn.add_css_class("player-continuous");
        controls.append(&prev_btn);
        controls.append(&main_btn);
        controls.append(&next_btn);
        controls.append(&shuffle_btn);
        controls.append(&cont_btn);
        root.append(&controls);

        let vol_box = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(6).valign(gtk::Align::Center).tooltip_text("Volume").css_classes(["player-volume"]).build();
        let vol_icon = gtk::Image::from_icon_name("audio-volume-high-symbolic");
        let vol = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.02);
        vol.set_draw_value(false);
        vol.set_width_request(90);
        vol_box.append(&vol_icon);
        vol_box.append(&vol);
        root.append(&vol_box);

        let close = ctl("window-close-symbolic", "Hide player");
        close.add_css_class("player-close");
        root.append(&close);

        let bar = Rc::new(Bar {
            root,
            cover,
            placeholder,
            cover_btn,
            title,
            sub,
            pos,
            seek,
            dur,
            prev_btn,
            main_btn,
            next_btn,
            shuffle_btn,
            cont_btn,
            vol_icon,
            vol,
            tick: RefCell::new(None),
            cover_for: Cell::new(None),
            cover_gen: Cell::new(0),
            library: Rc::downgrade(library),
        });

        bar.prev_btn.connect_clicked(|_| store::prev());
        bar.next_btn.connect_clicked(|_| store::next());
        bar.main_btn.connect_clicked(|_| store::toggle_play());
        bar.shuffle_btn.connect_clicked(|_| store::start_shuffle());
        bar.cont_btn.connect_clicked(|_| store::set_continuous(!store::view().continuous));
        close.connect_clicked(|_| store::hide_player());
        bar.vol.connect_change_value(|_, _, v| {
            store::set_volume(v);
            glib::Propagation::Proceed
        });
        bar.seek.connect_change_value(|_, _, v| {
            if let Some(s) = store::stream() {
                if s.is_seekable() {
                    s.seek((v * 1_000_000.0) as i64);
                }
            }
            glib::Propagation::Proceed
        });
        // The cover opens the game's panel.
        bar.cover_btn.connect_clicked(glib::clone!(#[weak] bar, move |_| {
            let Some(id) = bar_track(&store::view()).map(|t| t.game_id) else { return };
            let core = app::core();
            app::spawn(async move { games::get_game(core.state(), id).await }, glib::clone!(#[weak] bar, move |res| {
                if let (Ok(Some(g)), Some(lib)) = (res, bar.library.upgrade().or_else(window::library)) {
                    lib.detail().show(g);
                }
            }));
        }));

        let weak = Rc::downgrade(&bar);
        store::on_change(move |c| {
            if matches!(c, Change::Player | Change::Music(_) | Change::Support) {
                if let Some(b) = weak.upgrade() {
                    b.render();
                }
            }
        });
        bar.render();
        bar
    }

    fn render(self: &Rc<Self>) {
        let view = store::view();
        let track = bar_track(&view);
        let visible = !view.bar_hidden && track.is_some();
        self.root.set_visible(visible);
        let Some(track) = track.filter(|_| visible) else {
            self.stop_tick();
            return;
        };
        self.start_tick();

        if self.cover_for.get() != Some(track.game_id) {
            self.cover_for.set(Some(track.game_id));
            self.cover.set_paintable(gtk::gdk::Paintable::NONE);
            self.cover.set_visible(false);
            self.placeholder.set_visible(true);
            self.pos.set_label("--:--");
            self.dur.set_label("--:--");
            self.seek.set_value(0.0);
            let generation = self.cover_gen.get() + 1;
            self.cover_gen.set(generation);
            let weak = Rc::downgrade(self);
            covers::request(track.collection.as_deref(), track.thumbnail_key.as_deref(), covers::Size::Fill(COVER_PX, COVER_PX), move |t| {
                let Some(b) = weak.upgrade() else { return };
                if b.cover_gen.get() != generation {
                    return;
                }
                b.cover.set_paintable(t.as_ref());
                b.cover.set_visible(t.is_some());
                b.placeholder.set_visible(t.is_none());
            });
        }
        self.cover_btn.set_tooltip_text(Some(&format!("Open {}", track.title)));
        self.title.set_label(&track.title);

        let pending = pending_text(&view);
        let (sub, is_error) = match (&pending, &view.play_error) {
            (Some(p), _) => (p.clone(), false),
            (None, Some(e)) => (format!("Can't play this track - {e}"), true),
            (None, None) => (paused_note(&view).map(String::from).unwrap_or_else(|| mode_note(view.mode).into()), false),
        };
        self.sub.set_label(&sub);
        if is_error {
            self.sub.add_css_class("is-error");
        } else {
            self.sub.remove_css_class("is-error");
        }

        // Until a track is loaded the transport has nothing to act on: the
        // buttons stay in place but do nothing.
        let loading_only = view.current.is_none();
        let queue = view.mode != Mode::Theme;
        self.prev_btn.set_visible(queue);
        self.next_btn.set_visible(queue);
        self.shuffle_btn.set_visible(!queue);
        self.prev_btn.set_sensitive(!loading_only);
        self.next_btn.set_sensitive(!loading_only);
        self.main_btn.set_sensitive(!loading_only);
        // A pick already on its way owns the bar: a second shuffle click
        // would do nothing.
        self.shuffle_btn.set_sensitive(!loading_only && view.wanted.is_none());
        self.main_btn.set_icon_name(if view.playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" });
        self.main_btn.set_tooltip_text(Some(if view.playing { "Pause" } else { "Play" }));
        if view.continuous {
            self.cont_btn.add_css_class("is-on");
        } else {
            self.cont_btn.remove_css_class("is-on");
        }
        self.cont_btn.set_tooltip_text(Some(if view.continuous { "Continuous playback on" } else { "Continuous playback off" }));
        self.vol_icon.set_icon_name(Some(if view.volume == 0.0 { "audio-volume-muted-symbolic" } else { "audio-volume-high-symbolic" }));
        if (self.vol.value() - view.volume).abs() > 0.005 {
            self.vol.set_value(view.volume);
        }
    }

    fn start_tick(self: &Rc<Self>) {
        if self.tick.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        self.tick.replace(Some(glib::timeout_add_local(Duration::from_millis(TICK_MS), move || {
            let Some(b) = weak.upgrade() else { return glib::ControlFlow::Break };
            b.tick();
            glib::ControlFlow::Continue
        })));
        self.tick();
    }

    fn stop_tick(&self) {
        if let Some(t) = self.tick.take() {
            t.remove();
        }
    }

    /// Position is a property of the playing element, not of the queue: the
    /// seek bar reads it directly.
    fn tick(&self) {
        let Some(stream) = store::stream() else { return };
        let (position, duration) = (stream.timestamp(), stream.duration());
        let known = duration > 0;
        self.pos.set_label(&format_time(if known || position > 0 { Some(position) } else { None }));
        self.dur.set_label(&format_time(known.then_some(duration)));
        self.seek.set_sensitive(known && stream.is_seekable());
        if known {
            let secs = duration as f64 / 1_000_000.0;
            if (self.seek.adjustment().upper() - secs).abs() > 0.05 {
                self.seek.set_range(0.0, secs);
            }
            if !stream.is_seeking() {
                self.seek.set_value(position as f64 / 1_000_000.0);
            }
        }
    }
}

/// The toolbar's ♪: starts the shuffle, or shows / hides the bar once a
/// track is loaded. Hidden when the platform cannot play themes.
pub fn toolbar_button() -> gtk::Button {
    let btn = gtk::Button::builder().label("♪").css_classes(["btn", "icon", "ghost", "music-toolbar-btn"]).build();
    btn.connect_clicked(|_| {
        let view = store::view();
        if view.current.is_none() && view.wanted.is_none() {
            store::start_shuffle();
        } else if view.bar_hidden {
            store::show_player();
        } else {
            store::hide_player();
        }
    });
    let sync = glib::clone!(#[weak] btn, move || {
        let view = store::view();
        btn.set_visible(!view.unsupported);
        btn.set_tooltip_text(Some(toolbar_label(&view)));
    });
    sync();
    store::on_change(move |c| {
        if matches!(c, Change::Player | Change::Support) {
            sync();
        }
    });
    btn
}

fn ctl(icon: &str, tip: &str) -> gtk::Button {
    gtk::Button::builder().icon_name(icon).tooltip_text(tip).css_classes(["btn", "icon", "ghost", "player-btn"]).build()
}

/// What the bar shows: a loaded track, or one the listener asked for that is
/// still fetching. A panel's autoplay probe is not such an ask.
fn bar_track(view: &PlayerView) -> Option<Track> {
    view.current.clone().or_else(|| if view.wanted_auto { None } else { view.wanted.clone() })
}

fn toolbar_label(view: &PlayerView) -> &'static str {
    if view.current.is_none() && view.wanted.is_none() {
        "Play music (shuffle)"
    } else if view.bar_hidden {
        "Show player"
    } else {
        "Hide player"
    }
}

/// What the bar is waiting on, if anything - the next shuffle pick still
/// coming over the torrent, say.
fn pending_text(view: &PlayerView) -> Option<String> {
    let w = view.wanted.as_ref()?;
    let loading_only = view.current.is_none();
    // The title line already names this track while nothing else is loaded.
    let what = if loading_only { "a theme".to_string() } else { w.title.clone() };
    let state = store::music_state(w.game_id);
    Some(match state.as_ref().map(|s| s.phase.as_str()) {
        None => format!("Loading {what}…"),
        Some("fetching") => format!("Loading {what} {}%", (state.as_ref().map(|s| s.progress).unwrap_or(0.0) * 100.0).round() as i64),
        Some(PHASE_QUEUED) => format!("{} queued…", if loading_only { "Theme".to_string() } else { w.title.clone() }),
        Some(_) => format!("Looking for {what}…"),
    })
}

fn mode_note(mode: Mode) -> &'static str {
    match mode {
        Mode::Shuffle => "Shuffle · theme",
        Mode::List => "Browse list · theme",
        Mode::Theme => "Theme",
    }
}

fn paused_note(view: &PlayerView) -> Option<&'static str> {
    if view.reasons.contains(&PauseReason::Game) {
        return Some("Paused while the game runs");
    }
    if view.reasons.contains(&PauseReason::Video) {
        return Some("Paused for the preview");
    }
    None
}

/// `m:ss`, or `--:--` while the element has no duration to report.
fn format_time(micros: Option<i64>) -> String {
    match micros {
        Some(us) if us >= 0 => {
            let whole = us / 1_000_000;
            format!("{}:{:02}", whole / 60, whole % 60)
        }
        _ => "--:--".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> PlayerView {
        PlayerView {
            current: None,
            wanted: None,
            wanted_auto: false,
            mode: Mode::Theme,
            playing: false,
            user_paused: false,
            reasons: vec![],
            play_error: None,
            bar_hidden: false,
            unsupported: false,
            continuous: true,
            volume: 0.8,
        }
    }

    fn track(id: i64) -> Track {
        Track { game_id: id, title: format!("Game {id}"), collection: None, thumbnail_key: None }
    }

    #[test]
    fn time_is_m_ss_or_unknown() {
        assert_eq!(format_time(None), "--:--");
        assert_eq!(format_time(Some(-5)), "--:--");
        assert_eq!(format_time(Some(0)), "0:00");
        assert_eq!(format_time(Some(65_400_000)), "1:05");
        assert_eq!(format_time(Some(3_600_000_000)), "60:00");
    }

    #[test]
    fn the_bar_ignores_an_autoplay_wait_but_shows_a_click_wait() {
        let mut v = view();
        v.wanted = Some(track(1));
        v.wanted_auto = true;
        assert!(bar_track(&v).is_none());
        v.wanted_auto = false;
        assert_eq!(bar_track(&v).map(|t| t.game_id), Some(1));
        v.current = Some(track(2));
        assert_eq!(bar_track(&v).map(|t| t.game_id), Some(2));
    }

    #[test]
    fn toolbar_label_follows_the_player() {
        let mut v = view();
        assert_eq!(toolbar_label(&v), "Play music (shuffle)");
        v.current = Some(track(1));
        assert_eq!(toolbar_label(&v), "Hide player");
        v.bar_hidden = true;
        assert_eq!(toolbar_label(&v), "Show player");
    }

    #[test]
    fn notes_prefer_the_game_over_the_preview() {
        let mut v = view();
        assert_eq!(paused_note(&v), None);
        v.reasons = vec![PauseReason::Video];
        assert_eq!(paused_note(&v), Some("Paused for the preview"));
        v.reasons = vec![PauseReason::Video, PauseReason::Game];
        assert_eq!(paused_note(&v), Some("Paused while the game runs"));
        assert_eq!(mode_note(Mode::Shuffle), "Shuffle · theme");
    }
}
