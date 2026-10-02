//! The detail panel's media: the preview video under the cover (fetch
//! status, the frames, play/pause/mute/enlarge, the lightbox) and the Theme
//! row. The hero controller is the web UI's `heroVideo.ts`: one media
//! element attached for the panel's lifetime, every transition cancelling
//! the previous one (timer, sequence, source), the "video" pause reason
//! taken and given back here only.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use exorchy_core::commands::games;
use exorchy_core::models::Game;
use gtk::glib;
use gtk::prelude::*;
use adw::prelude::*;

use crate::app;
use crate::ui::detail::panel_width;

use super::store::{self, Change, PauseReason, Track, PHASE_PROBING, PHASE_QUEUED};
use super::playbin::PlaybinStream;

/// The panel settles on a game before any torrent read starts: clicking
/// through the grid would otherwise queue a read per card.
const SETTLE_MS: u64 = 400;
/// How often the theme row's position line updates.
const THEME_TICK_MS: u64 = 250;
/// How long the cover keeps the panel to itself before the preview starts.
const VIDEO_START_DELAY_MS: u64 = 2000;
const FADE_MS: u64 = 600;
const ERROR_RETRY_MS: u64 = 800;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Loading,
    Playing,
    Paused,
    Ended,
    Failed,
}

pub struct Preview {
    window: gtk::Window,
    pub root: gtk::Box,
    frame: gtk::Overlay,
    picture: gtk::Picture,
    controls: gtk::Box,
    play_btn: gtk::Button,
    mute_btn: gtk::Button,
    replay: gtk::Button,
    status_row: gtk::Box,
    status: gtk::Label,
    retry: gtk::Button,
    play_ready: gtk::Button,
    hero_error: gtk::Label,
    theme_row: gtk::Box,
    theme_btn: gtk::Button,
    theme_spinner: gtk::Spinner,
    theme_name: gtk::Label,
    theme_retry: gtk::Button,
    // Hero controller.
    stream: RefCell<PlaybinStream>,
    game_id: Cell<Option<i64>>,
    phase: Cell<Phase>,
    error: RefCell<Option<String>>,
    src: RefCell<Option<String>>,
    start_timer: Cell<Option<glib::SourceId>>,
    /// Bumped by every transition; the outcome of an earlier one is dropped.
    seq: Cell<u64>,
    holding_speakers: Cell<bool>,
    lightbox_hold: Cell<bool>,
    lightbox_open: Cell<bool>,
    fade_timer: Cell<Option<glib::SourceId>>,
    /// The game whose media error already got its one silent retry.
    error_retried_for: Cell<Option<i64>>,
    // Panel state.
    shown: RefCell<Option<Game>>,
    /// The theme (and the archive) belong to the GROUP's EN row: extras live
    /// in the EN archive only, so switching the language chip must neither
    /// restart nor re-request them.
    owner: RefCell<Option<Game>>,
    settle_timer: Cell<Option<glib::SourceId>>,
    theme_progress: gtk::Box,
    theme_pos: gtk::Label,
    theme_seek: gtk::Scale,
    theme_dur: gtk::Label,
    theme_tick: RefCell<Option<glib::SourceId>>,
    /// A fetch phase was observed for this game: the user spent the wait
    /// looking at the cover, and the ready video starts at once.
    video_just_fetched: Cell<bool>,
    video_phase_game: Cell<Option<i64>>,
    lightbox: RefCell<Option<adw::Dialog>>,
}

impl Preview {
    pub fn new(window: &gtk::Window) -> Rc<Self> {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).visible(false).css_classes(["media-slot"]).build();

        let stream = PlaybinStream::new_video();
        let picture = gtk::Picture::builder().paintable(&stream).content_fit(gtk::ContentFit::Contain).height_request(200).can_shrink(true).css_classes(["media-picture"]).build();
        let frame = gtk::Overlay::builder().child(&picture).visible(false).css_classes(["media-frame"]).build();
        let controls = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(2).halign(gtk::Align::End).valign(gtk::Align::End).margin_end(8).margin_bottom(8).css_classes(["media-controls"]).build();
        let play_btn = icon_btn("media-playback-pause-symbolic", "Pause preview");
        let mute_btn = icon_btn("audio-volume-muted-symbolic", "Unmute previews");
        let expand = icon_btn("view-fullscreen-symbolic", "Enlarge");
        controls.append(&play_btn);
        controls.append(&mute_btn);
        controls.append(&expand);
        frame.add_overlay(&controls);
        let replay = gtk::Button::builder().icon_name("media-playback-start-symbolic").halign(gtk::Align::Center).valign(gtk::Align::Center).tooltip_text("Replay preview").css_classes(["btn", "icon", "media-replay"]).visible(false).build();
        frame.add_overlay(&replay);
        root.append(&frame);

        let status_row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).visible(false).build();
        let status = gtk::Label::builder().xalign(0.0).css_classes(["media-status"]).visible(false).build();
        let retry = gtk::Button::builder().css_classes(["btn", "small", "media-retry"]).visible(false).build();
        let play_ready = gtk::Button::builder().label("▶ Play preview").css_classes(["btn", "small"]).visible(false).build();
        status_row.append(&status);
        status_row.append(&retry);
        status_row.append(&play_ready);
        root.append(&status_row);
        let hero_error = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["danger", "small", "media-error"]).visible(false).build();
        root.append(&hero_error);

        let theme_row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(10).visible(false).css_classes(["theme-row"]).build();
        theme_row.append(&gtk::Label::builder().label("Theme").css_classes(["muted", "small"]).build());
        let theme_btn = gtk::Button::builder().css_classes(["btn", "small"]).visible(false).build();
        let theme_spinner = gtk::Spinner::builder().spinning(true).visible(false).build();
        let theme_name = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).hexpand(true).css_classes(["secondary", "theme-name"]).build();
        let theme_retry = gtk::Button::builder().css_classes(["btn", "small", "media-retry"]).visible(false).build();
        theme_row.append(&theme_btn);
        theme_row.append(&theme_spinner);
        theme_row.append(&theme_name);
        theme_row.append(&theme_retry);
        root.append(&theme_row);
        // The playing theme's position, in place under its row.
        let theme_progress = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).visible(false).css_classes(["theme-progress", "player-seek"]).build();
        let theme_pos = gtk::Label::builder().label("--:--").css_classes(["player-time"]).build();
        let theme_seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.1);
        theme_seek.set_hexpand(true);
        theme_seek.set_draw_value(false);
        theme_seek.set_sensitive(false);
        let theme_dur = gtk::Label::builder().label("--:--").css_classes(["player-time"]).build();
        theme_progress.append(&theme_pos);
        theme_progress.append(&theme_seek);
        theme_progress.append(&theme_dur);
        root.append(&theme_progress);

        let p = Rc::new(Preview {
            window: window.clone(),
            root,
            frame,
            picture,
            controls,
            play_btn,
            mute_btn,
            replay,
            status_row,
            status,
            retry,
            play_ready,
            hero_error,
            theme_row,
            theme_btn,
            theme_spinner,
            theme_name,
            theme_retry,
            stream: RefCell::new(stream.clone()),
            game_id: Cell::new(None),
            phase: Cell::new(Phase::Idle),
            error: RefCell::new(None),
            src: RefCell::new(None),
            start_timer: Cell::new(None),
            seq: Cell::new(0),
            holding_speakers: Cell::new(false),
            lightbox_hold: Cell::new(false),
            lightbox_open: Cell::new(false),
            fade_timer: Cell::new(None),
            error_retried_for: Cell::new(None),
            shown: RefCell::new(None),
            owner: RefCell::new(None),
            settle_timer: Cell::new(None),
            theme_progress,
            theme_pos,
            theme_seek,
            theme_dur,
            theme_tick: RefCell::new(None),
            video_just_fetched: Cell::new(false),
            video_phase_game: Cell::new(None),
            lightbox: RefCell::new(None),
        });
        p.wire_stream(&stream);

        p.play_btn.connect_clicked(glib::clone!(#[weak] p, move |_| {
            if p.phase.get() == Phase::Playing {
                p.stream().pause();
            } else {
                p.replay(store::preview_muted());
            }
        }));
        p.replay.connect_clicked(glib::clone!(#[weak] p, move |_| p.replay(store::preview_muted())));
        p.play_ready.connect_clicked(glib::clone!(#[weak] p, move |_| p.replay(store::preview_muted())));
        p.mute_btn.connect_clicked(glib::clone!(#[weak] p, move |_| {
            let next = !store::preview_muted();
            store::set_preview_muted(next);
            p.set_muted_now(next);
            p.render();
        }));
        expand.connect_clicked(glib::clone!(#[weak] p, move |_| p.open_lightbox()));
        p.retry.connect_clicked(glib::clone!(#[weak] p, move |_| {
            if let Some(id) = p.owner_id() {
                store::request_video(id);
            }
        }));
        p.theme_retry.connect_clicked(glib::clone!(#[weak] p, move |_| {
            if let Some(id) = p.owner_id() {
                store::request_theme(id);
            }
        }));
        p.theme_seek.connect_change_value(|_, _, v| {
            if let Some(s) = store::stream() {
                if s.is_seekable() {
                    s.seek((v * 1_000_000.0) as i64);
                }
            }
            glib::Propagation::Proceed
        });
        p.theme_btn.connect_clicked(glib::clone!(#[weak] p, move |_| {
            let Some(owner) = p.owner.borrow().clone() else { return };
            let Some(track) = Track::of_game(&owner) else { return };
            if p.is_playing_this_theme() {
                store::toggle_play();
            } else {
                store::play_theme(track, false);
            }
        }));

        let weak = Rc::downgrade(&p);
        store::on_change(move |c| {
            let Some(p) = weak.upgrade() else { return };
            match c {
                Change::Video(id) if Some(*id) == p.owner_id() => p.sync_video(),
                Change::Music(id) if Some(*id) == p.owner_id() => p.render(),
                Change::Player | Change::Support => p.render(),
                Change::GameLaunched => p.stop_for_game(),
                _ => {}
            }
        });
        p
    }

    fn owner_id(&self) -> Option<i64> {
        self.owner.borrow().as_ref().and_then(|g| g.id)
    }

    fn stream(&self) -> PlaybinStream {
        self.stream.borrow().clone()
    }

    // ── Panel: opening and leaving a game ────────────────────────────────

    /// The panel opened on a (different) game, or closed.
    pub fn on_shown(self: &Rc<Self>, game: Option<&Game>) {
        let Some(game) = game else {
            self.leave();
            self.shown.replace(None);
            self.owner.replace(None);
            self.render();
            return;
        };
        if self.shown.borrow().as_ref().and_then(|g| g.id) == game.id && game.id.is_some() {
            return;
        }
        self.leave();
        self.shown.replace(Some(game.clone()));
        self.owner.replace(Some(game.clone()));
        self.settle();
        self.render();
        // A merged group opened on a language row: the theme and the archive
        // are the EN row's.
        let multi = game.available_languages.as_deref().map(|l| l.contains(',')).unwrap_or(false);
        if multi && game.language != "EN" {
            if let (Some(sc), Some(src), Some(gid)) = (game.shortcode.clone(), game.torrent_source.clone(), game.id) {
                let core = app::core();
                app::spawn(
                    async move { games::get_game_variants(core.state(), core.state(), sc, src).await },
                    glib::clone!(#[weak(rename_to = p)] self, move |res| {
                        if p.shown.borrow().as_ref().and_then(|g| g.id) != Some(gid) {
                            return;
                        }
                        let Ok(rows) = res else { return };
                        let Some(en) = rows.into_iter().find(|r| r.language == "EN" && r.id.is_some()) else { return };
                        if en.id == p.owner_id() {
                            return;
                        }
                        p.leave();
                        p.owner.replace(Some(en));
                        p.settle();
                        p.render();
                    }),
                );
            }
        }
    }

    /// Stand everything down for the game on the way out: the settle timer,
    /// the foreground claim, the auto theme, the preview, the lightbox.
    fn leave(self: &Rc<Self>) {
        if let Some(t) = self.settle_timer.take() {
            t.remove();
        }
        if let Some(id) = self.owner_id() {
            store::release_video(id);
        }
        self.drop_theme();
        if let Some(d) = self.lightbox.borrow_mut().take() {
            d.force_close();
        }
        self.clear_preview();
        self.video_just_fetched.set(false);
    }

    /// The game's theme stops with its dossier.
    fn drop_theme(&self) {
        if let Some(id) = self.owner_id() {
            store::leave_theme(id);
        }
    }

    /// A beat after the panel settles: the video fetch, and the theme beat.
    fn settle(self: &Rc<Self>) {
        let Some(id) = self.owner_id() else { return };
        store::set_foreground_video(Some(id));
        self.settle_timer.set(Some(glib::timeout_add_local_once(Duration::from_millis(SETTLE_MS), glib::clone!(#[weak(rename_to = p)] self, move || {
            p.settle_timer.set(None);
            if p.owner_id() != Some(id) {
                return;
            }
            store::request_video(id);
            // A video fetched on an earlier visit raises no change event:
            // load it here, as a fresh fetch would.
            if store::video_state(id).is_some_and(|v| v.phase == "ready") {
                p.sync_video();
            }
            // The theme is fetched so the row can offer it; it plays only
            // when the Play button is pressed.
            store::refresh_prefs();
            if !store::music_unsupported() {
                store::request_theme(id);
            }
        }))));
    }

    /// The owner's video status moved: start, or clear, the hero.
    fn sync_video(self: &Rc<Self>) {
        let Some(id) = self.owner_id() else { return };
        let state = store::video_state(id);
        if self.video_phase_game.get() != Some(id) {
            self.video_phase_game.set(Some(id));
            self.video_just_fetched.set(false);
        }
        let phase = state.as_ref().map(|s| s.phase.as_str()).unwrap_or("");
        if phase == "fetching" || phase == PHASE_PROBING || phase == PHASE_QUEUED {
            self.video_just_fetched.set(true);
        }
        match state.as_ref().filter(|s| s.phase == "ready").and_then(|s| s.path.clone()) {
            Some(path) => {
                let delay = if self.video_just_fetched.get() { 0 } else { VIDEO_START_DELAY_MS };
                self.show_preview(id, path, store::preview_muted(), delay);
            }
            None => self.clear_preview(),
        }
        self.render();
    }

    /// This game's theme is the loaded track (playing or paused).
    fn holds_this_theme(&self) -> bool {
        let view = store::view();
        self.owner_id().is_some() && view.mode == store::Mode::Theme && view.current.as_ref().map(|t| t.game_id) == self.owner_id()
    }

    fn render_theme_progress(&self, ready: bool) {
        let show = ready && self.holds_this_theme();
        self.theme_progress.set_visible(show);
        if !show {
            if let Some(t) = self.theme_tick.take() {
                t.remove();
            }
            return;
        }
        let (pos, seek, dur) = (self.theme_pos.clone(), self.theme_seek.clone(), self.theme_dur.clone());
        tick_theme(&pos, &seek, &dur);
        if self.theme_tick.borrow().is_none() {
            self.theme_tick.replace(Some(glib::timeout_add_local(Duration::from_millis(THEME_TICK_MS), move || {
                tick_theme(&pos, &seek, &dur);
                glib::ControlFlow::Continue
            })));
        }
    }

    fn is_playing_this_theme(&self) -> bool {
        let view = store::view();
        self.owner_id().is_some() && view.current.as_ref().map(|t| t.game_id) == self.owner_id() && view.playing
    }

    // ── Hero controller (heroVideo.ts) ───────────────────────────────────

    fn wire_stream(self: &Rc<Self>, stream: &PlaybinStream) {
        let weak = Rc::downgrade(self);
        stream.connect_playing_notify(move |s| {
            let Some(p) = weak.upgrade() else { return };
            if s.is_playing() {
                p.on_play();
            } else {
                p.on_pause();
            }
        });
        let weak = Rc::downgrade(self);
        stream.connect_ended_notify(move |s| {
            if s.is_ended() {
                if let Some(p) = weak.upgrade() {
                    p.on_ended();
                }
            }
        });
        let weak = Rc::downgrade(self);
        stream.connect_error_notify(move |s| {
            if let (Some(e), Some(p)) = (s.error(), weak.upgrade()) {
                p.on_error(e.message().to_string());
            }
        });
    }

    /// Every source gets a fresh element, and the picture follows: GTK keeps
    /// a failed stream in its error state for good.
    fn fresh_stream(self: &Rc<Self>) -> PlaybinStream {
        let current = self.stream();
        if current.file().is_none() && current.error().is_none() {
            return current;
        }
        current.pause();
        current.clear();
        let next = PlaybinStream::new_video();
        self.wire_stream(&next);
        self.picture.set_paintable(Some(&next));
        *self.stream.borrow_mut() = next.clone();
        next
    }

    fn set_src(self: &Rc<Self>, path: Option<&str>) {
        let stream = self.fresh_stream();
        match path {
            Some(p) => stream.set_filename(Some(std::path::Path::new(p))),
            None => stream.clear(),
        }
        self.src.replace(path.map(String::from));
    }

    fn claim_speakers(&self) {
        if self.holding_speakers.replace(true) {
            return;
        }
        store::pause_for(PauseReason::Video);
    }

    fn release_speakers(&self) {
        if !self.holding_speakers.get() || self.lightbox_hold.get() {
            return;
        }
        self.holding_speakers.set(false);
        store::resume_from(PauseReason::Video);
    }

    fn clear_timer(&self) {
        if let Some(t) = self.start_timer.take() {
            t.remove();
        }
    }

    fn clear_fade(&self) {
        if let Some(t) = self.fade_timer.take() {
            t.remove();
        }
    }

    /// The preview's sound eases in over the music's fade-out.
    fn fade_in(self: &Rc<Self>) {
        self.clear_fade();
        let stream = self.stream();
        stream.set_volume(0.0);
        let t0 = std::time::Instant::now();
        let weak = Rc::downgrade(self);
        self.fade_timer.set(Some(glib::timeout_add_local(Duration::from_millis(40), move || {
            let k = (t0.elapsed().as_millis() as f64 / FADE_MS as f64).min(1.0);
            stream.set_volume(k);
            if k < 1.0 {
                return glib::ControlFlow::Continue;
            }
            if let Some(p) = weak.upgrade() {
                p.fade_timer.set(None);
            }
            glib::ControlFlow::Break
        })));
    }

    fn start(self: &Rc<Self>, my_seq: u64, muted: bool) {
        if my_seq != self.seq.get() {
            return;
        }
        let stream = self.stream();
        stream.set_muted(muted);
        if muted {
            self.release_speakers();
        } else {
            self.claim_speakers();
            self.fade_in();
        }
        // The outcome arrives through the element's `playing` and `error`
        // properties; `on_play` / `on_error` apply it.
        stream.play();
    }

    /// Show a game's preview: after `delay_ms` of cover, start it with the
    /// given mute preference. Cancels whatever was pending or playing.
    fn show_preview(self: &Rc<Self>, id: i64, path: String, muted: bool, delay_ms: u64) {
        let same_src = self.src.borrow().as_deref() == Some(path.as_str());
        if self.game_id.get() == Some(id) && same_src && matches!(self.phase.get(), Phase::Playing | Phase::Loading) {
            return;
        }
        self.clear_timer();
        self.clear_fade();
        let my_seq = self.seq.get() + 1;
        self.seq.set(my_seq);
        if self.phase.get() != Phase::Idle && (self.game_id.get() != Some(id) || !same_src) {
            self.stream().pause();
        }
        if !same_src {
            self.set_src(Some(&path));
        }
        self.game_id.set(Some(id));
        self.phase.set(Phase::Loading);
        self.error.replace(None);
        // A preview about to play with sound claims the speakers NOW, not at
        // its first frame: the theme would start within the cover beat
        // otherwise and be cut off two seconds later.
        if muted {
            self.release_speakers();
        } else {
            self.claim_speakers();
        }
        self.start_timer.set(Some(glib::timeout_add_local_once(Duration::from_millis(delay_ms), glib::clone!(#[weak(rename_to = p)] self, move || {
            p.start_timer.set(None);
            if my_seq != p.seq.get() {
                return;
            }
            let s = p.stream();
            if s.is_seekable() {
                s.seek(0);
            }
            p.start(my_seq, muted);
        }))));
    }

    /// Nothing to show: stop, unload, give the speakers back.
    fn clear_preview(self: &Rc<Self>) {
        self.clear_timer();
        self.clear_fade();
        self.seq.set(self.seq.get() + 1);
        self.stream().pause();
        if self.src.borrow().is_some() {
            self.set_src(None);
        }
        self.game_id.set(None);
        self.phase.set(Phase::Idle);
        self.error.replace(None);
        self.release_speakers();
    }

    /// A game launched: the preview stops and stays stopped - the replay
    /// button brings it back.
    fn stop_for_game(self: &Rc<Self>) {
        if self.phase.get() == Phase::Idle {
            return;
        }
        self.clear_timer();
        self.clear_fade();
        self.seq.set(self.seq.get() + 1);
        self.stream().pause();
        if !matches!(self.phase.get(), Phase::Ended | Phase::Failed) {
            self.phase.set(Phase::Paused);
        }
        self.release_speakers();
        self.render();
    }

    /// The replay button: a gesture, so it starts with the real preference.
    fn replay(self: &Rc<Self>, muted: bool) {
        // Offered but never loaded into the hero: load it and start now.
        if self.game_id.get().is_none() || self.game_id.get() != self.owner_id() {
            let Some(id) = self.owner_id() else { return };
            let Some(path) = store::video_state(id).filter(|v| v.phase == "ready").and_then(|v| v.path) else { return };
            self.show_preview(id, path, muted, 0);
            self.render();
            return;
        }
        self.clear_timer();
        let my_seq = self.seq.get() + 1;
        self.seq.set(my_seq);
        self.error.replace(None);
        if self.phase.get() == Phase::Ended {
            let s = self.stream();
            if s.is_seekable() {
                s.seek(0);
            }
        }
        self.phase.set(Phase::Loading);
        self.start(my_seq, muted);
        self.render();
    }

    /// The mute toggle. Silent, the preview no longer needs the speakers;
    /// with sound it does - and a paused preview restarts.
    fn set_muted_now(self: &Rc<Self>, muted: bool) {
        if self.game_id.get().is_none() {
            return;
        }
        self.stream().set_muted(muted);
        if muted {
            self.release_speakers();
            return;
        }
        match self.phase.get() {
            Phase::Playing => {
                self.claim_speakers();
                self.fade_in();
            }
            Phase::Paused | Phase::Failed => self.replay(false),
            _ => {}
        }
    }

    fn on_play(self: &Rc<Self>) {
        if self.phase.get() == Phase::Idle {
            return;
        }
        self.phase.set(Phase::Playing);
        self.error.replace(None);
        // Frames are flowing, so the transient failure is spent.
        self.error_retried_for.set(None);
        if !self.stream().is_muted() {
            self.claim_speakers();
        }
        self.render();
    }

    fn on_pause(self: &Rc<Self>) {
        // Only a pause of something that played: a source change fires one too.
        if self.phase.get() != Phase::Playing {
            return;
        }
        self.phase.set(Phase::Paused);
        self.release_speakers();
        self.render();
    }

    fn on_ended(self: &Rc<Self>) {
        if self.phase.get() == Phase::Idle {
            return;
        }
        self.phase.set(Phase::Ended);
        self.release_speakers();
        self.render();
    }

    /// A media error gets one silent retry per game (a valid file failed
    /// once on the NVIDIA path and played on replay); a source that fails
    /// again gets named.
    fn on_error(self: &Rc<Self>, message: String) {
        if self.phase.get() == Phase::Idle {
            return;
        }
        let id = self.game_id.get();
        if id.is_some() && self.error_retried_for.get() != id {
            self.error_retried_for.set(id);
            self.clear_timer();
            self.clear_fade();
            let my_seq = self.seq.get() + 1;
            self.seq.set(my_seq);
            let muted = self.stream().is_muted();
            self.start_timer.set(Some(glib::timeout_add_local_once(Duration::from_millis(ERROR_RETRY_MS), glib::clone!(#[weak(rename_to = p)] self, move || {
                p.start_timer.set(None);
                if my_seq != p.seq.get() {
                    return;
                }
                // Re-run the load on a fresh element: the one answer to a
                // transient media error.
                let src = p.src.borrow().clone();
                p.set_src(src.as_deref());
                p.start(my_seq, muted);
            }))));
            return;
        }
        self.phase.set(Phase::Failed);
        self.error.replace(Some(message));
        self.release_speakers();
        self.render();
    }

    // ── Lightbox ─────────────────────────────────────────────────────────

    /// The same preview, large, in a dialog with GTK's own controls. The
    /// hero steps aside; with sound on, the speakers stay claimed.
    fn open_lightbox(self: &Rc<Self>) {
        if self.game_id.get().is_none() || self.lightbox.borrow().is_some() {
            return;
        }
        let muted = store::preview_muted();
        let title = self.owner.borrow().as_ref().map(|g| g.title.clone()).unwrap_or_default();
        let stream = self.stream();
        let video = gtk::Video::builder().media_stream(&stream).autoplay(false).hexpand(true).vexpand(true).css_classes(["media-lightbox-video"]).build();
        let dialog = adw::Dialog::builder().title(&title).content_width((panel_width() * 2).max(960)).content_height(640).child(&video).css_classes(["media-lightbox"]).build();
        dialog.connect_closed(glib::clone!(#[weak(rename_to = p)] self, move |_| {
            p.lightbox.replace(None);
            p.set_lightbox(false, false);
        }));
        self.lightbox.replace(Some(dialog.clone()));
        self.set_lightbox(true, !muted);
        dialog.present(Some(&self.window));
        let my_seq = self.seq.get();
        self.phase.set(Phase::Loading);
        self.start(my_seq, muted);
        self.render();
    }

    fn set_lightbox(self: &Rc<Self>, open: bool, holds_audio: bool) {
        let was_open = self.lightbox_open.replace(open);
        if open {
            if holds_audio {
                self.claim_speakers();
            }
            self.lightbox_hold.set(holds_audio);
            if matches!(self.phase.get(), Phase::Playing | Phase::Loading) {
                self.clear_timer();
                self.seq.set(self.seq.get() + 1);
                self.stream().pause();
                if self.phase.get() == Phase::Loading {
                    self.phase.set(Phase::Paused);
                }
            }
            if !holds_audio {
                self.release_speakers();
            }
            return;
        }
        if !was_open {
            return;
        }
        self.lightbox_hold.set(false);
        // A detached element keeps playing unless paused on the way out.
        self.stream().pause();
        if self.phase.get() != Phase::Playing {
            self.release_speakers();
        }
        self.render();
    }

    // ── Rendering ────────────────────────────────────────────────────────

    fn render(&self) {
        let Some(id) = self.owner_id() else {
            self.root.set_visible(false);
            return;
        };
        let owner = self.owner.borrow().clone();
        let video = store::video_state(id);
        let phase = video.as_ref().map(|s| s.phase.clone()).unwrap_or_default();
        let ready = phase == "ready" && video.as_ref().and_then(|s| s.path.as_ref()).is_some();
        let hero_mine = self.game_id.get() == Some(id);
        let hero = if hero_mine { self.phase.get() } else { Phase::Idle };
        let frames = matches!(hero, Phase::Playing | Phase::Paused | Phase::Ended);
        let muted = store::preview_muted();

        self.frame.set_visible(frames);
        self.controls.set_visible(frames);
        self.replay.set_visible(frames && hero != Phase::Playing);
        self.play_btn.set_icon_name(if hero == Phase::Playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" });
        self.play_btn.set_tooltip_text(Some(if hero == Phase::Playing { "Pause preview" } else { "Play preview" }));
        self.mute_btn.set_icon_name(if muted { "audio-volume-muted-symbolic" } else { "audio-volume-high-symbolic" });
        self.mute_btn.set_tooltip_text(Some(if muted { "Unmute previews" } else { "Mute previews" }));

        // Each stage of the probe says what it is; silence looks broken. No
        // "no video" line: most DOS titles have none.
        let pill = video_status_text(video.as_ref());
        self.status.set_label(pill.as_deref().unwrap_or(""));
        self.status.set_visible(pill.is_some());
        // A failed fetch must not look like "this game has no video".
        let failed = phase == "error";
        self.retry.set_visible(failed);
        if failed {
            let timeout = video.as_ref().map(store::is_timeout).unwrap_or(false);
            self.retry.set_label(if timeout { "↻ No peers yet - video retry" } else { "↻ Video retry" });
            self.retry.set_tooltip_text(video.as_ref().and_then(|s| s.error.as_deref()));
        }
        self.play_ready.set_visible(ready && !frames && hero != Phase::Loading);
        self.status_row.set_visible(pill.is_some() || failed || (ready && !frames && hero != Phase::Loading));
        let hero_error = if hero == Phase::Failed { self.error.borrow().clone() } else { None };
        self.hero_error.set_label(&hero_error.as_deref().map(|e| format!("Preview can't play - {e}")).unwrap_or_default());
        self.hero_error.set_visible(hero_error.is_some());

        // The theme row, once the archive has confirmed a track - or while
        // it is being asked.
        let music = if store::music_unsupported() { None } else { store::music_state(id) };
        let mphase = music.as_ref().map(|s| s.phase.as_str()).unwrap_or("");
        let mready = mphase == "ready" && music.as_ref().and_then(|s| s.path.as_ref()).is_some();
        let mbusy = mphase == "fetching" || mphase == PHASE_PROBING || mphase == PHASE_QUEUED;
        let mfailed = mphase == "error";
        self.theme_row.set_visible(mready || mbusy || mfailed);
        self.theme_btn.set_visible(mready);
        self.theme_spinner.set_visible(mbusy);
        self.theme_retry.set_visible(mfailed);
        if mready {
            self.theme_btn.set_label(if self.is_playing_this_theme() { "Pause" } else { "Play" });
            let name = owner.as_ref().and_then(|g| g.music_file.as_deref()).map(strip_extension).or_else(|| owner.as_ref().map(|g| g.title.clone())).unwrap_or_default();
            self.theme_name.set_label(&name);
        } else if mbusy {
            self.theme_name.set_label(&music_status_text(music.as_ref()));
        } else if mfailed {
            let timeout = music.as_ref().map(store::is_timeout).unwrap_or(false);
            self.theme_retry.set_label(if timeout { "↻ No peers yet - theme retry" } else { "↻ Theme retry" });
            self.theme_retry.set_tooltip_text(music.as_ref().and_then(|s| s.error.as_deref()));
            self.theme_name.set_label("");
        }

        self.render_theme_progress(mready);

        let anything = frames || pill.is_some() || failed || hero_error.is_some() || self.play_ready.is_visible() || self.theme_row.is_visible();
        self.root.set_visible(anything);
    }
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

/// The theme row's position line follows the music element.
fn tick_theme(pos: &gtk::Label, seek: &gtk::Scale, dur: &gtk::Label) {
    let Some(stream) = store::stream() else { return };
    let (position, duration) = (stream.timestamp(), stream.duration());
    let known = duration > 0;
    pos.set_label(&format_time(if known || position > 0 { Some(position) } else { None }));
    dur.set_label(&format_time(known.then_some(duration)));
    seek.set_sensitive(known && stream.is_seekable());
    if known {
        seek.set_range(0.0, duration as f64 / 1_000_000.0);
        seek.set_value(position as f64 / 1_000_000.0);
    }
}

fn icon_btn(icon: &str, tip: &str) -> gtk::Button {
    gtk::Button::builder().icon_name(icon).tooltip_text(tip).css_classes(["btn", "icon", "media-ctl"]).build()
}

/// The pill under the cover while the video is being looked for.
fn video_status_text(s: Option<&store::Status>) -> Option<String> {
    let s = s?;
    match s.phase.as_str() {
        PHASE_QUEUED => Some("Video queued…".into()),
        // Only a confirmed size counts as "loading": until then the archive
        // may still turn out to hold nothing.
        "fetching" if s.total_bytes > 0 => Some(format!("Loading video {}%", (s.progress * 100.0).round() as i64)),
        "fetching" | PHASE_PROBING => Some("Looking for a video…".into()),
        _ => None,
    }
}

fn music_status_text(s: Option<&store::Status>) -> String {
    match s.map(|s| s.phase.as_str()) {
        Some("fetching") => format!("Loading theme {}%", (s.map(|s| s.progress).unwrap_or(0.0) * 100.0).round() as i64),
        Some(PHASE_QUEUED) => "Theme queued…".into(),
        _ => "Looking for a theme…".into(),
    }
}

fn strip_extension(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(phase: &str, progress: f64, total: u64) -> store::Status {
        store::Status { phase: phase.into(), progress, total_bytes: total, path: None, error: None }
    }

    #[test]
    fn video_pill_names_each_stage() {
        assert_eq!(video_status_text(None), None);
        assert_eq!(video_status_text(Some(&st("probing", 0.0, 0))).as_deref(), Some("Looking for a video…"));
        assert_eq!(video_status_text(Some(&st("fetching", 0.0, 0))).as_deref(), Some("Looking for a video…"));
        assert_eq!(video_status_text(Some(&st("fetching", 0.426, 27_000_000))).as_deref(), Some("Loading video 43%"));
        assert_eq!(video_status_text(Some(&st("queued", 0.0, 0))).as_deref(), Some("Video queued…"));
        assert_eq!(video_status_text(Some(&st("ready", 1.0, 5))), None);
        assert_eq!(video_status_text(Some(&st("none", 0.0, 0))), None);
    }

    #[test]
    fn theme_wait_text() {
        assert_eq!(music_status_text(Some(&st("fetching", 0.5, 1))), "Loading theme 50%");
        assert_eq!(music_status_text(Some(&st("queued", 0.0, 0))), "Theme queued…");
        assert_eq!(music_status_text(Some(&st("probing", 0.0, 0))), "Looking for a theme…");
        assert_eq!(music_status_text(None), "Looking for a theme…");
    }

    #[test]
    fn theme_name_drops_the_extension_only() {
        assert_eq!(strip_extension("Doom (1993).mp3"), "Doom (1993)");
        assert_eq!(strip_extension("theme"), "theme");
        assert_eq!(strip_extension(".ogg"), ".ogg");
    }
}
