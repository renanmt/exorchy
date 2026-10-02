//! The media store: preview and theme fetches (one job model per kind,
//! polled at 700 ms), the fetch scheduler both kinds share, and the music
//! player's state machine - the web UI's `videos.ts`, `mediaQueue.ts` and
//! `music.ts` in one place. Main-thread only.
//!
//! Re-entrancy contract: the state lives in one `RefCell`. A public entry
//! point borrows it, runs the transition, releases the borrow and only THEN
//! drives the audio element and the listeners (`flush`), because both
//! re-enter the store synchronously (a `play()` notifies `playing` at once).
//! Methods on `Media` take `&mut self` and never call the public wrappers.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::time::Duration;

use exorchy_core::commands::games;
use exorchy_core::commands::media::{self as backend, MusicCandidate, MusicSupport};
pub use exorchy_core::commands::media::VideoStatus as Status;
use exorchy_core::models::Game;
use gtk::glib;

use crate::app;
use crate::ui::bus;

use super::audio::AudioPort;
use super::playbin::PlaybinStream;

/// Frontend-only phase: waiting for a fetch slot.
pub const PHASE_QUEUED: &str = "queued";
/// The backend is reading the archive index; existence is still open.
pub const PHASE_PROBING: &str = "probing";

const POLL_MS: u64 = 700;
const TIMEOUT_RETRY_MS: u64 = 10_000;
/// A shuffle pick nobody seeds must not hold the player: past this it is
/// dropped for the next candidate, silently.
pub const SHUFFLE_SKIP_MS: u64 = 60_000;
const CANDIDATE_BATCH: u32 = 10;
const CANDIDATE_LOW_WATER: usize = 3;
const HISTORY_MAX: usize = 50;
/// Duds since the last playable track; past this the walk stops.
const AUTO_SKIP_MAX: u32 = 5;
/// Fetch slots shared by videos and themes: each is a torrent stream with a
/// 32 MB lookahead.
pub const MAX_CONCURRENT: usize = 3;
/// The fade-out the audio port runs before a pause; a source swap waits it out.
const SWAP_AFTER_FADE_MS: u64 = 280;
const VOLUME_SAVE_DEBOUNCE_MS: u64 = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Video,
    Music,
}

type Key = (Kind, i64);

/// "theme" plays one track and stops; "shuffle" walks the whole collection;
/// "list" walks the Browse list the user is looking at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Theme,
    Shuffle,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PauseReason {
    Video,
    Game,
}

/// What listeners are told about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Video(i64),
    Music(i64),
    Player,
    /// A playback-support probe answered.
    Support,
    /// A game started running (the preview stops for it).
    GameLaunched,
}

/// An instruction for the audio element, executed after the borrow ends.
#[derive(Clone, Debug)]
pub enum PortOp {
    SetSrc(Option<String>),
    Play,
    Pause,
    SetVolume(f64),
}

/// A track: only the GameData theme exists today.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub game_id: i64,
    pub title: String,
    pub collection: Option<String>,
    pub thumbnail_key: Option<String>,
}

impl Track {
    pub fn of_game(g: &Game) -> Option<Track> {
        Some(Track { game_id: g.id?, title: g.title.clone(), collection: g.torrent_source.clone(), thumbnail_key: g.thumbnail_key.clone() })
    }

    fn of_candidate(c: &MusicCandidate) -> Track {
        Track { game_id: c.id, title: c.title.clone(), collection: c.torrent_source.clone(), thumbnail_key: c.thumbnail_key.clone() }
    }
}

/// The player as the bar and the panel see it.
#[derive(Clone, Debug)]
pub struct PlayerView {
    pub current: Option<Track>,
    pub wanted: Option<Track>,
    pub wanted_auto: bool,
    pub mode: Mode,
    pub playing: bool,
    pub reasons: Vec<PauseReason>,
    pub play_error: Option<String>,
    pub bar_hidden: bool,
    pub unsupported: bool,
    pub continuous: bool,
    pub volume: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Accepted,
    Refused,
    /// The support probe has not answered; the request is parked.
    Pending,
}

type Listener = Rc<dyn Fn(&Change)>;

#[derive(Clone, Copy)]
enum AfterRefill {
    Advance,
    Prefetch,
}

/// Per-kind job state.
#[derive(Default)]
struct Fetch {
    jobs: HashMap<i64, Status>,
    polling: HashMap<i64, glib::SourceId>,
    /// One automatic second try per game and session.
    timeout_retried: HashSet<i64>,
}

struct Media {
    video: Fetch,
    music: Fetch,
    /// The game whose panel is open: its video always gets a slot.
    foreground: Option<i64>,
    /// Ids somebody still wants the bytes of (panel probe, wanted track,
    /// prefetch). A job reaching its slot with nothing here gives it back.
    requested: HashSet<i64>,
    active: Vec<Key>,
    queue: VecDeque<Key>,
    video_supported: Option<bool>,
    video_probe_started: bool,
    video_waiters: Vec<i64>,
    music_support: Option<MusicSupport>,
    music_probe_started: bool,
    music_waiters: Vec<i64>,
    cached: HashSet<i64>,
    none: HashSet<i64>,
    indexing: bool,
    // Player.
    current: Option<Track>,
    wanted: Option<Track>,
    wanted_auto: bool,
    mode: Mode,
    playing: bool,
    user_paused: bool,
    reasons: HashSet<PauseReason>,
    play_error: Option<String>,
    bar_hidden: bool,
    history: Vec<Track>,
    up_next: Option<Track>,
    deferred_theme: Option<Track>,
    current_auto: bool,
    candidates: VecDeque<MusicCandidate>,
    refilling: bool,
    after_refill: Option<AfterRefill>,
    list_cursor: Option<i64>,
    skip_timer: Option<glib::SourceId>,
    load_seq: u64,
    auto_skips: u32,
    tried_this_walk: HashSet<i64>,
    stepping_back_to: Option<i64>,
    ended: bool,
    play_seq: u64,
    running: HashSet<i64>,
    list_source: Option<Rc<dyn Fn() -> Vec<Game>>>,
    // Preferences (`music_autoplay` is read by the panel at each beat).
    continuous: bool,
    volume: f64,
    preview_muted: bool,
    volume_save: Option<glib::SourceId>,
    // Effects, drained by `flush`.
    ops: Vec<PortOp>,
    pending: Vec<Change>,
    listeners: Vec<Listener>,
}

thread_local! {
    static MEDIA: RefCell<Media> = RefCell::new(Media::new());
    static PORT: RefCell<Option<Rc<AudioPort>>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Media) -> R) -> R {
    let r = MEDIA.with(|m| f(&mut m.borrow_mut()));
    flush();
    r
}

fn read<R>(f: impl FnOnce(&Media) -> R) -> R {
    MEDIA.with(|m| f(&m.borrow()))
}

/// Run the effects a transition queued. Re-entrant: a port op that notifies
/// synchronously lands in a nested `with`, whose own flush drains what that
/// added; the loop here picks up the rest.
fn flush() {
    loop {
        let (ops, changes) = MEDIA.with(|m| {
            let mut m = m.borrow_mut();
            (std::mem::take(&mut m.ops), std::mem::take(&mut m.pending))
        });
        if ops.is_empty() && changes.is_empty() {
            break;
        }
        let port = PORT.with(|p| p.borrow().clone());
        for op in ops {
            if let Some(p) = &port {
                p.apply(op);
            }
        }
        let listeners = read(|m| m.listeners.clone());
        for c in &changes {
            for l in &listeners {
                l(c);
            }
        }
    }
}

fn once(ms: u64, f: impl FnOnce() + 'static) -> glib::SourceId {
    glib::timeout_add_local_once(Duration::from_millis(ms), f)
}

fn is_in_flight(phase: &str) -> bool {
    phase == "fetching" || phase == PHASE_PROBING
}

/// A deadline the archive read ran into: no peers yet, not a broken file.
pub fn is_timeout(s: &Status) -> bool {
    s.phase == "error" && s.error.as_deref().map(|e| e.to_ascii_lowercase().starts_with("timed out")).unwrap_or(false)
}

fn queued_status(previous: Option<&Status>) -> Status {
    Status {
        phase: PHASE_QUEUED.into(),
        progress: 0.0,
        // The known size survives an eviction: it is the panel's signal that a
        // video was confirmed.
        total_bytes: previous.map(|p| p.total_bytes).unwrap_or(0),
        path: None,
        error: None,
    }
}

/// The two containers GStreamer is offered, minus a codec the probe ruled
/// out. Tracker modules and the like are not playable.
fn playable_with(support: Option<MusicSupport>, file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with(".ogg") {
        return support.map(|s| s.ogg).unwrap_or(true);
    }
    if lower.ends_with(".mp3") {
        return support.map(|s| s.mp3).unwrap_or(true);
    }
    false
}

impl Media {
    fn new() -> Self {
        Media {
            video: Fetch::default(),
            music: Fetch::default(),
            foreground: None,
            requested: HashSet::new(),
            active: Vec::new(),
            queue: VecDeque::new(),
            video_supported: None,
            video_probe_started: false,
            video_waiters: Vec::new(),
            music_support: None,
            music_probe_started: false,
            music_waiters: Vec::new(),
            cached: HashSet::new(),
            none: HashSet::new(),
            indexing: false,
            current: None,
            wanted: None,
            wanted_auto: false,
            mode: Mode::Theme,
            playing: false,
            user_paused: false,
            reasons: HashSet::new(),
            play_error: None,
            bar_hidden: false,
            history: Vec::new(),
            up_next: None,
            deferred_theme: None,
            current_auto: false,
            candidates: VecDeque::new(),
            refilling: false,
            after_refill: None,
            list_cursor: None,
            skip_timer: None,
            load_seq: 0,
            auto_skips: 0,
            tried_this_walk: HashSet::new(),
            stepping_back_to: None,
            ended: false,
            play_seq: 0,
            running: HashSet::new(),
            list_source: None,
            continuous: true,
            volume: 0.8,
            preview_muted: true,
            volume_save: None,
            ops: Vec::new(),
            pending: Vec::new(),
            listeners: Vec::new(),
        }
    }

    fn changed(&mut self, c: Change) {
        if !self.pending.contains(&c) {
            self.pending.push(c);
        }
    }

    fn fetch(&mut self, kind: Kind) -> &mut Fetch {
        match kind {
            Kind::Video => &mut self.video,
            Kind::Music => &mut self.music,
        }
    }

    fn fetch_ref(&self, kind: Kind) -> &Fetch {
        match kind {
            Kind::Video => &self.video,
            Kind::Music => &self.music,
        }
    }

    fn wanted_id(&self) -> Option<i64> {
        self.wanted.as_ref().map(|t| t.game_id)
    }

    fn current_id(&self) -> Option<i64> {
        self.current.as_ref().map(|t| t.game_id)
    }

    fn up_next_id(&self) -> Option<i64> {
        self.up_next.as_ref().map(|t| t.game_id)
    }

    // ── Scheduler (mediaQueue.ts) ────────────────────────────────────────

    /// 0 = the visible game's video, 1 = the track the player waits on,
    /// 2 = background. Read at decision time - they move.
    fn priority(&self, key: Key) -> u8 {
        match key.0 {
            Kind::Video if self.foreground == Some(key.1) => 0,
            Kind::Music if self.wanted_id() == Some(key.1) => 1,
            _ => 2,
        }
    }

    fn is_active(&self, key: Key) -> bool {
        self.active.contains(&key)
    }

    fn drop_queued(&mut self, key: Key) {
        self.queue.retain(|k| *k != key);
    }

    /// The least important running job that matters less than `priority`,
    /// oldest first among equals.
    fn eviction_victim(&self, priority: u8) -> Option<Key> {
        let mut victim: Option<(Key, u8)> = None;
        for &job in &self.active {
            let p = self.priority(job);
            if p <= priority {
                continue;
            }
            if victim.map(|(_, vp)| p > vp).unwrap_or(true) {
                victim = Some((job, p));
            }
        }
        victim.map(|(k, _)| k)
    }

    /// Run now if a slot is free (or can be taken from something less
    /// important), otherwise queue.
    fn request_slot(&mut self, key: Key) {
        if self.is_active(key) {
            return;
        }
        self.drop_queued(key);
        if self.active.len() >= MAX_CONCURRENT {
            let Some(victim) = self.eviction_victim(self.priority(key)) else {
                self.on_queued(key);
                self.queue.push_back(key);
                return;
            };
            self.evict(victim);
        }
        self.start(key);
    }

    fn evict(&mut self, key: Key) {
        self.active.retain(|k| *k != key);
        self.on_evicted(key);
        if !self.queue.contains(&key) {
            self.queue.push_front(key);
        }
    }

    fn start(&mut self, key: Key) {
        self.active.push(key);
        self.begin_fetch(key);
    }

    fn release_slot(&mut self, key: Key) {
        self.active.retain(|k| *k != key);
        self.pump();
    }

    fn pump(&mut self) {
        while !self.queue.is_empty() && self.active.len() < MAX_CONCURRENT {
            let mut best = 0;
            for i in 1..self.queue.len() {
                if self.priority(self.queue[i]) < self.priority(self.queue[best]) {
                    best = i;
                }
            }
            if let Some(next) = self.queue.remove(best) {
                self.start(next);
            }
        }
    }

    fn on_queued(&mut self, key: Key) {
        let prev = self.fetch_ref(key.0).jobs.get(&key.1).cloned();
        self.put(key, queued_status(prev.as_ref()));
    }

    /// The slot went to something more important: the backend read is
    /// cancelled and the entry shows as queued - never dropped.
    fn on_evicted(&mut self, key: Key) {
        if let Some(src) = self.fetch(key.0).polling.remove(&key.1) {
            src.remove();
        }
        cancel_backend(key);
        let prev = self.fetch_ref(key.0).jobs.get(&key.1).cloned();
        self.put(key, queued_status(prev.as_ref()));
    }

    // ── Fetch jobs ───────────────────────────────────────────────────────

    fn still_wanted(&self, id: i64) -> bool {
        self.wanted_id() == Some(id) || self.up_next_id() == Some(id) || self.requested.contains(&id)
    }

    fn begin_fetch(&mut self, key: Key) {
        let (kind, id) = key;
        if self.fetch_ref(kind).polling.contains_key(&id) {
            return;
        }
        if kind == Kind::Music && !self.still_wanted(id) {
            // Queued long enough for its reason to disappear: give the slot
            // back instead of streaming bytes for a track nobody will hear.
            self.forget_job(kind, id);
            self.stop_polling(key);
            return;
        }
        let core = app::core();
        match kind {
            Kind::Video => app::spawn(
                async move { backend::start_game_video(core.state(), core.state(), core.state(), id).await },
                move |res| with(|m| m.fetch_started(key, res)),
            ),
            Kind::Music => app::spawn(
                async move { backend::start_game_music(core.state(), core.state(), core.state(), id).await },
                move |res| with(|m| m.fetch_started(key, res)),
            ),
        }
    }

    fn fetch_started(&mut self, key: Key, res: Result<Status, String>) {
        let (kind, id) = key;
        let initial = match res {
            Ok(s) => s,
            Err(e) => {
                self.put(key, Status { phase: "error".into(), progress: 0.0, total_bytes: 0, path: None, error: Some(e) });
                self.stop_polling(key);
                return;
            }
        };
        let in_flight = is_in_flight(&initial.phase);
        self.put(key, initial);
        if !in_flight {
            // Cached, absent, or failed outright - nothing to poll for.
            self.stop_polling(key);
            return;
        }
        let src = glib::timeout_add_local(Duration::from_millis(POLL_MS), move || {
            let core = app::core();
            match kind {
                Kind::Video => app::spawn(
                    async move { backend::get_video_status(core.state(), id).await },
                    move |res| with(|m| m.polled(key, res)),
                ),
                Kind::Music => app::spawn(
                    async move { backend::get_music_status(core.state(), id).await },
                    move |res| with(|m| m.polled(key, res)),
                ),
            }
            glib::ControlFlow::Continue
        });
        self.fetch(kind).polling.insert(id, src);
    }

    fn polled(&mut self, key: Key, res: Result<Option<Status>, String>) {
        if !self.fetch_ref(key.0).polling.contains_key(&key.1) {
            // A late answer for a poll that was stopped meanwhile.
            return;
        }
        match res {
            Ok(Some(status)) => {
                let done = !is_in_flight(&status.phase);
                self.put(key, status);
                if done {
                    self.stop_polling(key);
                }
            }
            // The backend forgot the job (a cancel, a restart): the entry goes
            // with it, or it would sit at "fetching" forever.
            Ok(None) => {
                self.forget_job(key.0, key.1);
                self.stop_polling(key);
            }
            Err(_) => self.stop_polling(key),
        }
    }

    /// The job is over, so the request that carried it is too.
    fn stop_polling(&mut self, key: Key) {
        if let Some(src) = self.fetch(key.0).polling.remove(&key.1) {
            src.remove();
        }
        if key.0 == Kind::Music {
            self.requested.remove(&key.1);
        }
        self.release_slot(key);
    }

    fn put(&mut self, key: Key, status: Status) {
        let (kind, id) = key;
        // A "none" with a non-null error is provisional (offline, no session):
        // shown once, then forgotten, or the game is blacklisted for the session.
        let provisional = status.phase == "none" && (status.error.is_some() || (kind == Kind::Music && bus::offline()));
        let timeout = is_timeout(&status);
        let phase = status.phase.clone();
        self.fetch(kind).jobs.insert(id, status);
        match kind {
            Kind::Video => {
                self.changed(Change::Video(id));
                if provisional {
                    self.video.jobs.remove(&id);
                }
            }
            Kind::Music => {
                self.changed(Change::Music(id));
                if !provisional {
                    self.note_in_index(id, &phase);
                }
                self.reconcile();
                if provisional {
                    self.forget_job(kind, id);
                }
            }
        }
        if timeout && !self.fetch_ref(kind).timeout_retried.contains(&id) {
            self.fetch(kind).timeout_retried.insert(id);
            once(TIMEOUT_RETRY_MS, move || {
                with(|m| {
                    let still = m.fetch_ref(kind).jobs.get(&id).map(is_timeout).unwrap_or(false);
                    if still {
                        match kind {
                            Kind::Video => m.request_video(id),
                            Kind::Music => {
                                m.request_theme(id);
                            }
                        }
                    }
                })
            });
        }
    }

    /// Drop the entry entirely, so the next request probes again rather than
    /// finding a status nothing is working on any more.
    fn forget_job(&mut self, kind: Kind, id: i64) {
        self.fetch(kind).jobs.remove(&id);
        if kind == Kind::Video {
            self.changed(Change::Video(id));
            return;
        }
        self.requested.remove(&id);
        self.changed(Change::Music(id));
        // `wanted` still set here means the player was waiting on this job
        // and nothing would ever answer: treat it as a dud.
        if self.wanted_id() == Some(id) {
            self.clear_skip_timer();
            self.wanted = None;
            self.changed(Change::Player);
            if self.auto_advances() {
                self.advance_past_dud();
            }
        }
    }

    /// Nobody is waiting for these bytes any more: stop the backend job,
    /// free the slot, take it out of the queue and forget it.
    fn abandon_fetch(&mut self, id: i64) {
        let key = (Kind::Music, id);
        self.drop_queued(key);
        cancel_backend(key);
        self.stop_polling(key);
        self.forget_job(Kind::Music, id);
    }

    /// A fetch that has not started only ever cost a place in the queue.
    fn release_queued(&mut self, id: i64) {
        if self.music.jobs.get(&id).map(|s| s.phase.as_str()) != Some(PHASE_QUEUED) {
            return;
        }
        self.drop_queued((Kind::Music, id));
        self.forget_job(Kind::Music, id);
    }

    // ── Videos ───────────────────────────────────────────────────────────

    fn request_video(&mut self, id: i64) {
        match self.video_supported {
            None => {
                if !self.video_waiters.contains(&id) {
                    self.video_waiters.push(id);
                }
                self.ensure_video_probe();
                return;
            }
            // On an affected system the video element itself is what wedges
            // the app, so the whole feature stands down - no wasted traffic.
            Some(false) => return,
            Some(true) => {}
        }
        if let Some(known) = self.video.jobs.get(&id) {
            if known.phase != "error" && known.phase != PHASE_QUEUED {
                return;
            }
        }
        let key = (Kind::Video, id);
        if self.video.polling.contains_key(&id) || self.is_active(key) {
            return;
        }
        self.request_slot(key);
    }

    fn ensure_video_probe(&mut self) {
        if self.video_probe_started {
            return;
        }
        self.video_probe_started = true;
        if assume_supported() {
            self.video_probe_done(true);
            return;
        }
        app::spawn(async move { backend::video_playback_supported().await }, |ok| with(|m| m.video_probe_done(ok)));
    }

    fn video_probe_done(&mut self, ok: bool) {
        self.video_supported = Some(ok);
        self.changed(Change::Support);
        for id in std::mem::take(&mut self.video_waiters) {
            self.request_video(id);
        }
    }

    // ── Music: support, index, requests ──────────────────────────────────

    fn ensure_music_probe(&mut self) {
        if self.music_probe_started {
            return;
        }
        self.music_probe_started = true;
        if assume_supported() {
            self.music_probe_done(MusicSupport { mp3: true, ogg: true });
            return;
        }
        app::spawn(async move { backend::music_playback_supported().await }, |s| with(|m| m.music_probe_done(s)));
    }

    /// Only an explicit "no mp3" switches the feature off.
    fn music_probe_done(&mut self, s: MusicSupport) {
        self.music_support = Some(s);
        self.changed(Change::Support);
        for id in std::mem::take(&mut self.music_waiters) {
            let accepted = s.mp3;
            if accepted {
                self.request_theme_known(id);
            }
            if self.wanted_id() == Some(id) {
                self.wanted_fetch_started(id, accepted);
            }
        }
    }

    fn music_unsupported(&self) -> bool {
        self.music_support.map(|s| !s.mp3).unwrap_or(false)
    }

    fn playable(&self, file_name: &str) -> bool {
        playable_with(self.music_support, file_name)
    }

    /// Keep the sets in step with what the fetches learn.
    fn note_in_index(&mut self, id: i64, phase: &str) {
        if phase == "ready" {
            self.cached.insert(id);
            return;
        }
        if phase != "none" {
            return;
        }
        self.none.insert(id);
        self.cached.remove(&id);
    }

    /// Fetch a theme without changing what plays.
    fn request_theme(&mut self, id: i64) -> Outcome {
        match self.music_support {
            None => {
                if !self.music_waiters.contains(&id) {
                    self.music_waiters.push(id);
                }
                self.ensure_music_probe();
                Outcome::Pending
            }
            Some(s) if !s.mp3 => Outcome::Refused,
            Some(_) => {
                self.request_theme_known(id);
                Outcome::Accepted
            }
        }
    }

    fn request_theme_known(&mut self, id: i64) {
        if let Some(known) = self.music.jobs.get(&id) {
            if known.phase != "error" && known.phase != PHASE_QUEUED {
                self.reconcile();
                return;
            }
        }
        let key = (Kind::Music, id);
        if self.music.polling.contains_key(&id) || self.is_active(key) {
            return;
        }
        self.requested.insert(id);
        self.request_slot(key);
    }

    // ── Player ───────────────────────────────────────────────────────────

    fn play(&mut self) {
        if self.current.is_none() {
            return;
        }
        self.ended = false;
        self.play_seq += 1;
        self.ops.push(PortOp::Play);
        self.playing = true;
        self.play_error = None;
        self.changed(Change::Player);
    }

    fn pause(&mut self) {
        self.play_seq += 1;
        self.ops.push(PortOp::Pause);
        self.playing = false;
        self.changed(Change::Player);
    }

    fn clear_skip_timer(&mut self) {
        if let Some(src) = self.skip_timer.take() {
            src.remove();
        }
    }

    /// Both queue modes move on by themselves; theme mode plays one track.
    fn auto_advances(&self) -> bool {
        self.mode != Mode::Theme
    }

    /// Set the track the player is waiting on. It becomes the current one
    /// the moment its bytes are on disk; until then whatever plays keeps
    /// playing.
    fn want(&mut self, track: Track, auto: bool) {
        self.clear_skip_timer();
        self.wanted_auto = auto;
        // Whatever is asked for now is newer than a parked panel theme.
        self.deferred_theme = None;
        if self.stepping_back_to != Some(track.game_id) {
            self.stepping_back_to = None;
        }
        let previous = self.wanted.take();
        // Asking for a track is an implicit "show me the player".
        self.bar_hidden = false;
        self.wanted = Some(track.clone());
        self.changed(Change::Player);
        // Release the replaced pick's slot AFTER the new wanted track is in
        // place, or `forget_job` reads the drop as a dud.
        if let Some(p) = previous {
            if p.game_id != track.game_id && Some(p.game_id) != self.up_next_id() {
                self.release_queued(p.game_id);
            }
        }
        self.tried_this_walk.insert(track.game_id);
        if self.mode == Mode::List {
            self.list_cursor = Some(track.game_id);
        }
        match self.request_theme(track.game_id) {
            Outcome::Pending => {}
            outcome => self.wanted_fetch_started(track.game_id, outcome == Outcome::Accepted),
        }
    }

    /// The skip timer is armed only once the fetch is actually under way.
    fn wanted_fetch_started(&mut self, id: i64, accepted: bool) {
        if self.wanted_id() != Some(id) {
            return;
        }
        if !accepted {
            // The platform cannot decode anything we have - no fetch was
            // started and none ever will be; the queue is stood down.
            self.wanted = None;
            self.list_cursor = None;
            if self.current.is_none() {
                self.mode = Mode::Theme;
            }
            self.changed(Change::Player);
            return;
        }
        if !self.auto_advances() {
            return;
        }
        self.clear_skip_timer();
        self.skip_timer = Some(once(SHUFFLE_SKIP_MS, move || with(|m| m.skip_timer_fired(id))));
    }

    fn skip_timer_fired(&mut self, id: i64) {
        self.skip_timer = None;
        if self.wanted_id() != Some(id) {
            return;
        }
        if self.music.jobs.get(&id).map(|s| s.phase.as_str()) == Some("ready") {
            return;
        }
        // Cleared before the abandon: `forget_job` skips for a wanted track
        // whose entry disappears, and the walk below is that skip.
        self.wanted = None;
        self.changed(Change::Player);
        self.abandon_fetch(id);
        self.advance_past_dud();
    }

    /// The wanted track's bytes arrived (or never will): act on it. Runs
    /// after every status change.
    fn reconcile(&mut self) {
        let Some(w) = self.wanted.clone() else { return };
        let Some(state) = self.music.jobs.get(&w.game_id).cloned() else { return };
        if state.phase == "ready" {
            if let Some(path) = state.path {
                self.clear_skip_timer();
                let auto = self.wanted_auto;
                self.wanted = None;
                self.changed(Change::Player);
                self.load(w, path, auto);
            }
        } else if state.phase == "none" || state.phase == "error" {
            // No theme, or a failed read: in theme mode the previous track
            // keeps playing and the panel says what happened; a queue skips.
            self.clear_skip_timer();
            self.wanted = None;
            self.changed(Change::Player);
            if self.auto_advances() {
                self.advance_past_dud();
            }
        }
    }

    /// Walk past a pick that produced nothing. Bounded: every arm of the walk
    /// can dud out at once (offline, an archive set without music).
    fn advance_past_dud(&mut self) {
        self.auto_skips += 1;
        if self.auto_skips >= AUTO_SKIP_MAX {
            self.auto_skips = 0;
            self.tried_this_walk.clear();
            self.wanted = None;
            self.pause();
            return;
        }
        self.advance(true);
    }

    fn load(&mut self, track: Track, path: String, auto: bool) {
        self.load_seq += 1;
        let seq = self.load_seq;
        // An audibly playing track gets its fade-out before the swap.
        if self.playing {
            self.pause();
            once(SWAP_AFTER_FADE_MS, move || {
                with(|m| {
                    if m.load_seq == seq {
                        m.load_now(track, path, auto);
                    }
                })
            });
            return;
        }
        self.load_now(track, path, auto);
    }

    fn load_now(&mut self, track: Track, path: String, auto: bool) {
        // A track that plays ends the dud streak and starts a fresh walk.
        self.auto_skips = 0;
        self.tried_this_walk.clear();
        let stepping_back = self.stepping_back_to == Some(track.game_id);
        self.stepping_back_to = None;
        // Stepping back, the outgoing track goes to `up_next`, not onto the
        // history we are walking down.
        if let Some(previous) = self.current.clone() {
            if !stepping_back && previous.game_id != track.game_id {
                self.history.retain(|t| t.game_id != previous.game_id);
                self.history.push(previous);
                if self.history.len() > HISTORY_MAX {
                    let excess = self.history.len() - HISTORY_MAX;
                    self.history.drain(..excess);
                }
            }
        }
        if self.up_next_id() == Some(track.game_id) {
            self.up_next = None;
        }
        self.current = Some(track);
        self.current_auto = auto;
        self.playing = false;
        self.play_error = None;
        self.ended = false;
        self.play_seq += 1;
        self.ops.push(PortOp::SetSrc(Some(path)));
        self.changed(Change::Player);
        if !self.user_paused && self.reasons.is_empty() {
            self.play();
        }
        // No prefetch when nothing will follow.
        if self.auto_advances() && self.continuous {
            self.prefetch_next(true);
        }
    }

    fn handle_ended(&mut self) {
        self.playing = false;
        self.ended = true;
        self.changed(Change::Player);
        // The theme parked behind this track has first claim on the silence.
        if let Some(track) = self.deferred_theme.take() {
            self.up_next = None;
            self.mode = Mode::Theme;
            self.want(track, true);
            return;
        }
        if self.auto_advances() && self.continuous {
            self.next();
        }
    }

    /// Play a game's theme. A click overrides whatever holds the speakers;
    /// the panel's autoplay (`auto`) does not.
    fn play_theme(&mut self, track: Track, auto: bool) {
        // A track the LISTENER chose is not cut off mid-song by browsing to
        // another game: the panel's autoplay queues up behind it.
        if auto && self.playing && !self.current_auto && self.current_id() != Some(track.game_id) {
            self.deferred_theme = Some(track.clone());
            self.request_theme(track.game_id);
            return;
        }
        self.mode = Mode::Theme;
        if auto {
            self.user_paused = false;
            self.bar_hidden = false;
        } else {
            self.listener_wants_sound();
        }
        self.up_next = None;
        self.changed(Change::Player);
        if self.current_id() == Some(track.game_id) {
            // A click on the already-loaded track adopts it like toggle_play.
            if !auto {
                self.current_auto = false;
            }
            self.wanted = None;
            if !self.playing && self.reasons.is_empty() {
                self.play();
            }
            return;
        }
        self.want(track, auto);
    }

    /// The dossier moved off a game: its theme stops with it. The shuffle
    /// and the list walk are not tied to a game and keep playing.
    fn leave_theme(&mut self, id: i64) {
        if self.mode == Mode::Theme && (self.current_id() == Some(id) || self.wanted_id() == Some(id)) {
            self.stop();
        }
    }

    /// Play a theme and keep going down the Browse list as it stands.
    #[allow(dead_code)]
    fn play_from_list(&mut self, track: Track) {
        self.mode = Mode::List;
        self.listener_wants_sound();
        self.up_next = None;
        self.list_cursor = Some(track.game_id);
        self.changed(Change::Player);
        if self.current_id() == Some(track.game_id) {
            self.current_auto = false;
            self.wanted = None;
            if !self.playing {
                self.play();
            }
            return;
        }
        self.want(track, false);
    }

    /// The next row of the list worth playing, walking in `dir`. Never wraps.
    fn list_neighbour(&self, id: i64, dir: i32) -> Option<Track> {
        let list = self.list_source.as_ref()?();
        let index = match list.iter().position(|g| g.id == Some(id)) {
            Some(i) => i as i32,
            // The list changed under the player: forward starts over from the
            // top, back has nothing to go back to.
            None if dir < 0 => return None,
            None => -1,
        };
        let mut i = index + dir;
        while i >= 0 && (i as usize) < list.len() {
            let g = &list[i as usize];
            if g.id.is_some() && self.playable_hint(g) {
                return Track::of_game(g);
            }
            i += dir;
        }
        None
    }

    /// Is this row worth offering a play button? The catalogue hint plus
    /// what the fetches learned - the archive still has the last word.
    fn playable_hint(&self, game: &Game) -> bool {
        let Some(id) = game.id else { return false };
        if let Some(f) = game.music_file.as_deref() {
            if !self.playable(f) {
                return false;
            }
        }
        if self.cached.contains(&id) {
            return true;
        }
        if self.none.contains(&id) {
            return false;
        }
        game.music_file.is_some()
    }

    // ── Shuffle ──────────────────────────────────────────────────────────

    fn refill(&mut self) {
        if self.candidates.len() >= CANDIDATE_LOW_WATER || self.refilling {
            return;
        }
        self.refilling = true;
        let core = app::core();
        app::spawn(
            async move { backend::music_shuffle_candidates(core.state(), CANDIDATE_BATCH).await },
            |res| with(|m| m.refilled(res)),
        );
    }

    fn refilled(&mut self, res: Result<Vec<MusicCandidate>, String>) {
        self.refilling = false;
        if let Ok(batch) = res {
            // The picks the walk already knows are hopeless: a probe that found
            // no music, and everything tried since the last track that played.
            let mut seen: HashSet<i64> = self.candidates.iter().map(|c| c.id).collect();
            seen.extend(self.history.iter().rev().take(10).map(|t| t.game_id));
            seen.extend(self.none.iter().copied());
            seen.extend(self.tried_this_walk.iter().copied());
            seen.extend(self.current_id());
            for c in batch {
                if !seen.contains(&c.id) && self.playable(&c.music_file) {
                    seen.insert(c.id);
                    self.candidates.push_back(c);
                }
            }
        }
        match self.after_refill.take() {
            Some(AfterRefill::Advance) => self.advance(false),
            Some(AfterRefill::Prefetch) => self.prefetch_next(false),
            None => {}
        }
    }

    /// Take the next entry of the running queue as the wanted track. `refill`
    /// allows one wait for candidates; the continuation comes back with false.
    fn advance(&mut self, refill: bool) {
        // Offline every pick answers "none" instantly; the walk would spin.
        if bus::offline() {
            return;
        }
        if self.mode == Mode::List {
            let Some(cursor) = self.list_cursor else { return };
            if let Some(ahead) = self.list_neighbour(cursor, 1) {
                self.want(ahead, false);
            }
            return;
        }
        if self.mode != Mode::Shuffle {
            return;
        }
        if self.candidates.is_empty() && refill {
            self.after_refill = Some(AfterRefill::Advance);
            self.refill();
            return;
        }
        let Some(c) = self.candidates.pop_front() else { return };
        self.want(Track::of_candidate(&c), false);
        self.refill();
    }

    /// Exactly one track ahead: enough to hide the swarm's latency between
    /// songs, not enough to compete with the videos for peers.
    fn prefetch_next(&mut self, refill: bool) {
        if !self.auto_advances() || self.up_next.is_some() {
            return;
        }
        if self.mode == Mode::List {
            let Some(cursor) = self.current_id().or(self.list_cursor) else { return };
            let Some(ahead) = self.list_neighbour(cursor, 1) else { return };
            self.up_next = Some(ahead.clone());
            self.request_theme(ahead.game_id);
            return;
        }
        if self.candidates.is_empty() && refill {
            self.after_refill = Some(AfterRefill::Prefetch);
            self.refill();
            return;
        }
        let Some(c) = self.candidates.front() else { return };
        if self.mode != Mode::Shuffle {
            return;
        }
        let track = Track::of_candidate(c);
        let id = track.game_id;
        self.up_next = Some(track);
        self.request_theme(id);
    }

    /// Start the collection-wide shuffle. Only ever called from a click.
    fn start_shuffle(&mut self) {
        if bus::offline() {
            bus::toast("Offline: the shuffle needs the torrent session");
            return;
        }
        // A pick is already on its way: a second click must not stack a
        // second fetch behind it.
        if self.wanted.is_some() {
            return;
        }
        self.mode = Mode::Shuffle;
        self.user_paused = false;
        self.bar_hidden = false;
        self.auto_skips = 0;
        self.changed(Change::Player);
        self.advance(true);
    }

    fn next(&mut self) {
        // Only theme mode has no queue to move along; there "next" starts one.
        if !self.auto_advances() {
            self.start_shuffle();
            return;
        }
        if let Some(ahead) = self.up_next.take() {
            if self.mode == Mode::Shuffle && self.candidates.front().map(|c| c.id) == Some(ahead.game_id) {
                self.candidates.pop_front();
            }
            self.want(ahead, false);
            return;
        }
        self.advance(true);
    }

    fn prev(&mut self) {
        let Some(previous) = self.history.pop() else { return };
        // The current track goes in front of the queue rather than back into
        // history, so "prev" then "next" returns to it.
        if self.mode == Mode::Shuffle {
            if let Some(cur) = self.current.clone() {
                self.up_next = Some(cur);
            }
        }
        self.stepping_back_to = Some(previous.game_id);
        self.want(previous, false);
    }

    /// A click on a play control is the listener's word: it overrides the
    /// pause of their own and whatever else claimed the speakers.
    fn listener_wants_sound(&mut self) {
        self.user_paused = false;
        self.bar_hidden = false;
        self.reasons.clear();
        self.changed(Change::Player);
    }

    fn toggle_play(&mut self) {
        if self.playing {
            self.user_paused = true;
            self.pause();
            return;
        }
        self.listener_wants_sound();
        // Pressing play on an autoplayed track adopts it.
        self.current_auto = false;
        if self.current.is_some() {
            self.play();
        }
    }

    /// Put the bar away without giving up the track. A gesture, so it counts
    /// as a pause of the listener's own - nothing resumes behind a hidden bar.
    fn hide_player(&mut self) {
        self.bar_hidden = true;
        self.user_paused = true;
        self.pause();
    }

    fn show_player(&mut self) {
        self.bar_hidden = false;
        self.changed(Change::Player);
    }

    fn stop(&mut self) {
        self.clear_skip_timer();
        self.bar_hidden = false;
        self.pause();
        self.load_seq += 1;
        let ahead = self.up_next.take();
        let abandoned = self.wanted.take();
        for track in [abandoned, ahead].into_iter().flatten() {
            if self.music.jobs.get(&track.game_id).map(|s| s.phase.as_str()) != Some("ready") {
                self.abandon_fetch(track.game_id);
            }
        }
        self.current = None;
        self.play_error = None;
        self.deferred_theme = None;
        self.current_auto = false;
        self.ended = false;
        self.ops.push(PortOp::SetSrc(None));
        self.list_cursor = None;
        self.auto_skips = 0;
        self.tried_this_walk.clear();
        self.stepping_back_to = None;
        self.mode = Mode::Theme;
        self.user_paused = false;
        self.reasons.clear();
        self.changed(Change::Player);
    }

    // ── Arbitration ──────────────────────────────────────────────────────

    /// Something else needs the speakers. A level, not an edge: recorded
    /// whether or not anything plays.
    fn pause_for(&mut self, reason: PauseReason) {
        self.reasons.insert(reason);
        self.pause();
    }

    fn resume_from(&mut self, reason: PauseReason) {
        if !self.reasons.remove(&reason) {
            return;
        }
        self.changed(Change::Player);
        if self.reasons.is_empty() && !self.user_paused && !self.ended && self.current.is_some() && !self.playing {
            self.play();
        }
    }

    /// The running set moved: each new game is a reason to stay quiet, the
    /// last exit gives the speakers back.
    fn sync_running(&mut self, now: &HashSet<i64>) {
        let added: Vec<i64> = now.difference(&self.running).copied().collect();
        let removed: Vec<i64> = self.running.difference(now).copied().collect();
        self.running = now.clone();
        if !added.is_empty() {
            self.pause_for(PauseReason::Game);
            self.changed(Change::GameLaunched);
        }
        if !removed.is_empty() && self.running.is_empty() {
            self.resume_from(PauseReason::Game);
        }
    }

    fn view(&self) -> PlayerView {
        PlayerView {
            current: self.current.clone(),
            wanted: self.wanted.clone(),
            wanted_auto: self.wanted_auto,
            mode: self.mode,
            playing: self.playing,
            reasons: self.reasons.iter().copied().collect(),
            play_error: self.play_error.clone(),
            bar_hidden: self.bar_hidden,
            unsupported: self.music_unsupported(),
            continuous: self.continuous,
            volume: self.volume,
        }
    }
}

fn cancel_backend(key: Key) {
    let (kind, id) = key;
    let core = app::core();
    match kind {
        Kind::Video => app::spawn(async move { backend::cancel_game_video(core.state(), id).await }, |_| {}),
        Kind::Music => app::spawn(async move { backend::cancel_game_music(core.state(), id).await }, |_| {}),
    }
}

/// Developer aid: `EXORCHY_MEDIA_ASSUME_SUPPORTED=1` skips the GStreamer
/// probes, so the player and the preview can be exercised on a machine
/// without the decoders (the element then reports its own error).
fn assume_supported() -> bool {
    std::env::var_os("EXORCHY_MEDIA_ASSUME_SUPPORTED").is_some_and(|v| v == "1")
}

fn persist(key: &str, value: String) {
    let core = app::core();
    let key = key.to_string();
    app::spawn(async move { games::set_config(core.clone(), core.state(), key, value).await }, |res| {
        if let Err(e) = res {
            log::warn!("media: saving a preference failed: {e}");
        }
    });
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Once, before the UI: the audio element, the preferences, the cache index
/// and the support probes.
pub fn init() {
    PORT.with(|p| {
        if p.borrow().is_none() {
            *p.borrow_mut() = Some(AudioPort::new());
        }
    });
    refresh_prefs();
    refresh_music_index();
    with(|m| {
        m.ensure_music_probe();
        m.ensure_video_probe();
    });
}

/// Re-read the preferences (the settings page writes the same keys).
pub fn refresh_prefs() {
    let core = app::core();
    app::spawn(
        async move {
            let mut out = Vec::new();
            for key in ["music_continuous", "music_volume", "preview_muted"] {
                out.push(games::get_config(core.state(), key.into()).await.ok().flatten());
            }
            out
        },
        |vals| {
            with(|m| {
                let flag = |v: &Option<String>, default: bool| v.as_deref().map(|s| s == "1").unwrap_or(default);
                m.continuous = flag(&vals[0], true);
                m.volume = vals[1].as_deref().and_then(|s| s.parse::<f64>().ok()).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0)).unwrap_or(0.8);
                m.preview_muted = flag(&vals[2], true);
                let v = m.volume;
                m.ops.push(PortOp::SetVolume(v));
                m.changed(Change::Player);
            })
        },
    );
}

/// One readdir, and every row can say whether its theme is on disk.
pub fn refresh_music_index() {
    if read(|m| m.indexing) {
        return;
    }
    with(|m| m.indexing = true);
    let core = app::core();
    app::spawn(async move { backend::music_cache_index(core.state()).await }, |res| {
        with(|m| {
            m.indexing = false;
            if let Ok(index) = res {
                m.cached = index.cached.into_iter().collect();
                m.none = index.none.into_iter().collect();
            }
        })
    });
}

pub fn on_change(f: impl Fn(&Change) + 'static) {
    MEDIA.with(|m| m.borrow_mut().listeners.push(Rc::new(f)));
}

/// Preview and theme fetches: those holding a slot, then those waiting for
/// one, as (kind, game id, status, queued). The Transfers page.
pub fn fetches() -> Vec<(Kind, i64, Option<Status>, bool)> {
    read(|m| {
        let row = |&(kind, id): &Key, queued: bool| (kind, id, m.fetch_ref(kind).jobs.get(&id).cloned(), queued);
        m.active.iter().map(|k| row(k, false)).chain(m.queue.iter().map(|k| row(k, true))).collect()
    })
}

pub fn video_state(id: i64) -> Option<Status> {
    read(|m| m.video.jobs.get(&id).cloned())
}

pub fn music_state(id: i64) -> Option<Status> {
    read(|m| m.music.jobs.get(&id).cloned())
}

/// `Some(false)` once the probe said no; the panel explains why.
#[allow(dead_code)]
pub fn video_supported() -> Option<bool> {
    read(|m| m.video_supported)
}

pub fn music_unsupported() -> bool {
    read(|m| m.music_unsupported())
}

pub fn request_video(id: i64) {
    with(|m| m.request_video(id));
}

/// Mark the game the panel shows: its fetch jumps the queue and is never
/// evicted.
pub fn set_foreground_video(id: Option<i64>) {
    with(|m| m.foreground = id);
}

/// The panel moved on. A running fetch keeps going in the background.
pub fn release_video(id: i64) {
    with(|m| {
        if m.foreground == Some(id) {
            m.foreground = None;
        }
    });
}

pub fn request_theme(id: i64) -> Outcome {
    with(|m| m.request_theme(id))
}

pub fn play_theme(track: Track, auto: bool) {
    with(|m| m.play_theme(track, auto));
}

pub fn leave_theme(id: i64) {
    with(|m| m.leave_theme(id));
}

#[allow(dead_code)]
pub fn play_from_list(track: Track) {
    with(|m| m.play_from_list(track));
}

/// The Browse list as the player's queue; the library page registers it.
#[allow(dead_code)]
pub fn set_list_source(f: impl Fn() -> Vec<Game> + 'static) {
    with(|m| m.list_source = Some(Rc::new(f)));
}

#[allow(dead_code)]
pub fn playable_hint(game: &Game) -> bool {
    read(|m| m.playable_hint(game))
}

pub fn start_shuffle() {
    with(|m| m.start_shuffle());
}

pub fn next() {
    with(|m| m.next());
}

pub fn prev() {
    with(|m| m.prev());
}

pub fn toggle_play() {
    with(|m| m.toggle_play());
}

pub fn hide_player() {
    with(|m| m.hide_player());
}

pub fn show_player() {
    with(|m| m.show_player());
}

#[allow(dead_code)]
pub fn stop() {
    with(|m| m.stop());
}

pub fn pause_for(reason: PauseReason) {
    with(|m| m.pause_for(reason));
}

pub fn resume_from(reason: PauseReason) {
    with(|m| m.resume_from(reason));
}

pub fn sync_running(now: &HashSet<i64>) {
    with(|m| m.sync_running(now));
}

pub fn view() -> PlayerView {
    read(|m| m.view())
}

pub fn set_continuous(v: bool) {
    with(|m| {
        m.continuous = v;
        m.changed(Change::Player);
    });
    persist("music_continuous", if v { "1" } else { "0" }.into());
}

pub fn set_volume(v: f64) {
    let v = v.clamp(0.0, 1.0);
    with(|m| {
        m.volume = v;
        m.ops.push(PortOp::SetVolume(v));
        m.changed(Change::Player);
        if let Some(src) = m.volume_save.take() {
            src.remove();
        }
        m.volume_save = Some(once(VOLUME_SAVE_DEBOUNCE_MS, move || {
            with(|m| m.volume_save = None);
            persist("music_volume", format!("{v:.2}"));
        }));
    });
}

pub fn preview_muted() -> bool {
    read(|m| m.preview_muted)
}

pub fn set_preview_muted(v: bool) {
    with(|m| m.preview_muted = v);
    persist("preview_muted", if v { "1" } else { "0" }.into());
}

/// The audio element, for the seek bar.
pub fn stream() -> Option<PlaybinStream> {
    PORT.with(|p| p.borrow().as_ref().map(|p| p.stream()))
}

// Events from the audio port.

pub fn port_ended() {
    with(|m| m.handle_ended());
}

pub fn port_error(message: String) {
    with(|m| {
        m.play_error = Some(message);
        m.playing = false;
        m.changed(Change::Player);
    });
}

pub fn port_playing() {
    with(|m| {
        m.playing = true;
        m.play_error = None;
        m.changed(Change::Player);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(phase: &str, error: Option<&str>) -> Status {
        Status { phase: phase.into(), progress: 0.0, total_bytes: 0, path: None, error: error.map(String::from) }
    }

    #[test]
    fn timeout_errors_are_recognised_case_insensitively() {
        assert!(is_timeout(&status("error", Some("Timed out fetching the video"))));
        assert!(is_timeout(&status("error", Some("timed out"))));
        assert!(!is_timeout(&status("error", Some("archive unreadable"))));
        assert!(!is_timeout(&status("ready", Some("timed out"))));
    }

    #[test]
    fn queued_status_keeps_the_confirmed_size() {
        let prev = Status { phase: "fetching".into(), progress: 0.4, total_bytes: 27_000_000, path: None, error: None };
        let q = queued_status(Some(&prev));
        assert_eq!(q.phase, PHASE_QUEUED);
        assert_eq!(q.total_bytes, 27_000_000);
        assert_eq!(q.progress, 0.0);
        assert_eq!(queued_status(None).total_bytes, 0);
    }

    #[test]
    fn playable_only_rules_out_what_the_probe_refused() {
        assert!(playable_with(None, "Theme.MP3"));
        assert!(playable_with(None, "theme.ogg"));
        assert!(!playable_with(None, "theme.xm"));
        let no_ogg = MusicSupport { mp3: true, ogg: false };
        assert!(playable_with(Some(no_ogg), "a.mp3"));
        assert!(!playable_with(Some(no_ogg), "a.ogg"));
    }

    #[test]
    fn eviction_prefers_the_least_important_oldest_job() {
        let mut m = Media::new();
        m.foreground = Some(1);
        m.wanted = Some(Track { game_id: 2, title: "t".into(), collection: None, thumbnail_key: None });
        m.active = vec![(Kind::Video, 1), (Kind::Music, 2), (Kind::Video, 3), (Kind::Video, 4)];
        // A background request (2) may not evict anything: nothing is less important.
        assert_eq!(m.eviction_victim(2), None);
        // The wanted track (1) evicts the oldest background job.
        assert_eq!(m.eviction_victim(1), Some((Kind::Video, 3)));
        // The visible game (0) also skips the wanted track for a background one.
        assert_eq!(m.eviction_victim(0), Some((Kind::Video, 3)));
        m.active = vec![(Kind::Video, 1), (Kind::Music, 2)];
        assert_eq!(m.eviction_victim(0), Some((Kind::Music, 2)));
    }

    #[test]
    fn playable_hint_follows_the_probe_then_the_cache_then_the_catalogue() {
        let mut m = Media::new();
        let mut g = Game { id: Some(7), music_file: Some("x.mp3".into()), ..Game::default() };
        assert!(m.playable_hint(&g));
        m.none.insert(7);
        assert!(!m.playable_hint(&g));
        m.cached.insert(7);
        assert!(m.playable_hint(&g));
        m.music_support = Some(MusicSupport { mp3: false, ogg: true });
        assert!(!m.playable_hint(&g));
        g.music_file = None;
        m.cached.insert(7);
        assert!(m.playable_hint(&g), "a cached track plays whatever the catalogue claims");
    }

    #[test]
    fn list_neighbour_walks_past_rows_without_a_theme_and_never_wraps() {
        let mut m = Media::new();
        let row = |id: i64, music: Option<&str>| {
            Game { id: Some(id), title: format!("g{id}"), music_file: music.map(String::from), ..Game::default() }
        };
        let rows = vec![row(1, Some("a.mp3")), row(2, None), row(3, Some("c.ogg")), row(4, None)];
        m.list_source = Some(Rc::new(move || rows.clone()));
        assert_eq!(m.list_neighbour(1, 1).map(|t| t.game_id), Some(3));
        assert_eq!(m.list_neighbour(3, 1), None);
        assert_eq!(m.list_neighbour(3, -1).map(|t| t.game_id), Some(1));
        assert_eq!(m.list_neighbour(99, -1), None);
        assert_eq!(m.list_neighbour(99, 1).map(|t| t.game_id), Some(1), "a vanished cursor restarts from the top");
    }
}
