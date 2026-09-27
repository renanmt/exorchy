//! The reading store: the catalogue (loaded once, filtered in the client),
//! which issues are on disk, and one fetch job per issue key with its 1 Hz
//! status poll. The port of the web UI's `stores/reading.ts`; the media
//! queue's three slots are not part of the native app, so a fetch starts the
//! moment it is asked for. Main-thread only.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use exorchy_core::commands::reading::{self as cmd, ReadingStatus};
use exorchy_core::models::{Issue, Publication};
use gtk::glib;

use crate::app;
use crate::ui::bus;

/// Store-only phase: the command was sent and the backend has not answered
/// yet. Without it the click leaves the card looking untouched for seconds.
pub const PHASE_QUEUED: &str = "queued";

/// What changed: the whole catalogue, or one issue's row or fetch status.
pub enum Change<'a> {
    Catalog,
    Issue(&'a str),
}

type Listener = Rc<dyn Fn(&Change)>;

#[derive(Default)]
struct Store {
    publications: Vec<Publication>,
    issues: Vec<Issue>,
    loaded: bool,
    loading: bool,
    /// The last error the catalogue load produced, shown in the room.
    load_error: Option<String>,
    fetches: HashMap<String, ReadingStatus>,
    on_disk: HashSet<String>,
    polls: HashMap<String, glib::SourceId>,
    /// Bumped whenever a job stops being the one the user asked for; an
    /// answer still in flight compares against it and is dropped.
    epochs: HashMap<String, u64>,
    page_timers: HashMap<String, glib::SourceId>,
    listeners: Vec<(u64, Listener)>,
    next_listener: u64,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store::default());
}

fn with<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    STORE.with(|s| f(&mut s.borrow_mut()))
}

// ── Listeners ────────────────────────────────────────────────────────────────

/// Subscribe; the id removes the listener again. Callbacks run on the GTK
/// thread with no store borrow held.
pub fn on_change(f: impl Fn(&Change) + 'static) -> u64 {
    with(|s| {
        s.next_listener += 1;
        let id = s.next_listener;
        s.listeners.push((id, Rc::new(f)));
        id
    })
}

pub fn remove_listener(id: u64) {
    with(|s| s.listeners.retain(|(i, _)| *i != id));
}

fn notify(change: Change) {
    let listeners: Vec<Listener> = with(|s| s.listeners.iter().map(|(_, l)| l.clone()).collect());
    for l in listeners {
        l(&change);
    }
}

// ── Catalogue ────────────────────────────────────────────────────────────────

pub fn loaded() -> bool {
    with(|s| s.loaded)
}

pub fn load_error() -> Option<String> {
    with(|s| s.load_error.clone())
}

pub fn publications() -> Vec<Publication> {
    with(|s| s.publications.clone())
}

pub fn issues() -> Vec<Issue> {
    with(|s| s.issues.clone())
}

pub fn issue(key: &str) -> Option<Issue> {
    with(|s| s.issues.iter().find(|i| i.key == key).cloned())
}

/// The whole catalogue is ~2,200 rows, so it is loaded once and filtered in
/// the client - a keystroke must not become a round trip.
pub fn load_catalog(force: bool) {
    let start = with(|s| {
        if (s.loaded && !force) || s.loading {
            return false;
        }
        s.loading = true;
        true
    });
    if !start {
        return;
    }
    let core = app::core();
    app::spawn(
        async move {
            let pubs = cmd::list_publications(core.state(), None).await;
            let rows = cmd::list_issues(core.state(), None, None, None).await;
            let cached = cmd::reading_cache_index(core.state()).await.unwrap_or_default();
            (pubs, rows, cached)
        },
        |(pubs, rows, cached)| {
            with(|s| {
                s.loading = false;
                match (pubs, rows) {
                    (Ok(p), Ok(r)) => {
                        s.publications = p;
                        s.issues = r;
                        s.on_disk = cached.into_iter().collect();
                        s.loaded = true;
                        s.load_error = None;
                    }
                    (Err(e), _) | (_, Err(e)) => {
                        log::error!("Reading Room: catalogue failed to load: {e}");
                        s.load_error = Some(e);
                        s.loaded = true;
                    }
                }
            });
            notify(Change::Catalog);
        },
    );
}

fn patch_issue(key: &str, f: impl FnOnce(&mut Issue)) {
    with(|s| {
        if let Some(row) = s.issues.iter_mut().find(|i| i.key == key) {
            f(row);
        }
    });
}

// ── Fetch state ──────────────────────────────────────────────────────────────

pub fn status(key: &str) -> Option<ReadingStatus> {
    with(|s| s.fetches.get(key).cloned())
}

pub fn is_on_disk(key: &str) -> bool {
    with(|s| s.on_disk.contains(key))
}

/// Downloaded, in either shape: a cached document or an extracted disk
/// magazine. What "Remove from disk" is offered for.
pub fn issue_on_disk(issue: &Issue) -> bool {
    if issue.runnable {
        issue.installed
    } else {
        is_on_disk(&issue.key)
    }
}

pub fn is_busy(key: &str) -> bool {
    matches!(status(key), Some(s) if s.phase == "fetching" || s.phase == PHASE_QUEUED)
}

fn plain(phase: &str) -> ReadingStatus {
    ReadingStatus { phase: phase.into(), progress: 0.0, total_bytes: 0, path: None, error: None }
}

fn error_status(e: String) -> ReadingStatus {
    ReadingStatus { error: Some(e), ..plain("error") }
}

/// Record a status. "none" with an error is provisional (offline): shown
/// once, never kept, or one offline visit would mark the issue unavailable
/// for the session.
fn put(key: &str, status: ReadingStatus) {
    let provisional = status.phase == "none" && status.error.is_some();
    let ready = status.phase == "ready";
    with(|s| {
        s.fetches.insert(key.to_string(), status);
    });
    if ready {
        landed(key);
    }
    notify(Change::Issue(key));
    if provisional {
        with(|s| s.fetches.remove(key));
    }
}

/// The document is on disk: the card flips without a catalogue reload, and
/// a runnable issue becomes playable. Here, not in the poll, because a
/// command can answer "ready" outright and never be polled at all.
fn landed(key: &str) {
    with(|s| {
        s.on_disk.insert(key.to_string());
        if let Some(row) = s.issues.iter_mut().find(|i| i.key == key) {
            if row.runnable {
                row.installed = true;
            }
        }
    });
}

fn forget(key: &str) {
    with(|s| s.fetches.remove(key));
    notify(Change::Issue(key));
}

fn epoch(key: &str) -> u64 {
    with(|s| s.epochs.get(key).copied().unwrap_or(0))
}

fn bump_epoch(key: &str) -> u64 {
    with(|s| {
        let e = s.epochs.entry(key.to_string()).or_insert(0);
        *e += 1;
        *e
    })
}

fn stop_polling(key: &str) {
    if let Some(id) = with(|s| s.polls.remove(key)) {
        id.remove();
    }
}

/// Poll the job at 1 Hz until it ends. The poll is the only thing that ever
/// ends a fetch, so an answer of "no job" or an error must end it too, or
/// the card stays pinned at "fetching" and refuses every retry.
fn poll(key: &str, epoch_at_start: u64) {
    stop_polling(key);
    let k = key.to_string();
    let id = glib::timeout_add_local(Duration::from_secs(1), move || {
        let key = k.clone();
        let core = app::core();
        app::spawn(async move { cmd::get_issue_status(core.state(), key.clone()).await.map(|s| (key, s)) }, move |res| {
            let (key, status) = match res {
                Ok(v) => v,
                Err(e) => {
                    log::error!("Reading Room: status poll failed: {e}");
                    return;
                }
            };
            if epoch(&key) != epoch_at_start {
                return;
            }
            match status {
                None => {
                    // The backend dropped the job (a cancel landed).
                    stop_polling(&key);
                    forget(&key);
                }
                Some(status) => {
                    let terminal = matches!(status.phase.as_str(), "ready" | "error" | "none");
                    put(&key, status);
                    if terminal {
                        stop_polling(&key);
                    }
                }
            }
        });
        glib::ControlFlow::Continue
    });
    with(|s| s.polls.insert(key.to_string(), id));
}

/// Fetch an issue for reading, or have the one already on disk answered as
/// ready. Callers render from `status(key)`; nothing here blocks.
pub fn request_issue(key: &str) {
    start_fetch(key, false);
}

/// Download a disk magazine so it can be launched. Same job and the same
/// polling as reading one - only the backend command differs.
pub fn install_issue(key: &str) {
    start_fetch(key, true);
}

fn start_fetch(key: &str, install: bool) {
    if let Some(s) = status(key) {
        if s.phase == "ready" || s.phase == "fetching" || s.phase == PHASE_QUEUED {
            return;
        }
    }
    if bus::offline() {
        put(key, ReadingStatus { error: Some("offline".into()), ..plain("none") });
        return;
    }
    let e = bump_epoch(key);
    put(key, plain(PHASE_QUEUED));
    let core = app::core();
    let k = key.to_string();
    app::spawn(
        async move {
            let r = if install {
                cmd::install_issue(core.clone(), core.state(), core.state(), core.state(), core.state(), k.clone()).await
            } else {
                cmd::open_issue(core.state(), core.state(), core.state(), core.state(), k.clone()).await
            };
            (k, r)
        },
        move |(key, res)| {
            if epoch(&key) != e {
                return;
            }
            match res {
                Ok(status) => {
                    let fetching = status.phase == "fetching";
                    put(&key, status);
                    if fetching {
                        poll(&key, e);
                    }
                }
                Err(err) => {
                    log::error!("Reading Room: fetch of {key} failed: {err}");
                    put(&key, error_status(err));
                }
            }
        },
    );
}

/// Abandon a fetch: the poll, the status and the backend job.
pub fn abort(key: &str) {
    bump_epoch(key);
    stop_polling(key);
    forget(key);
    let core = app::core();
    let k = key.to_string();
    app::spawn(async move { cmd::cancel_issue_fetch(core.state(), k).await }, |res| {
        if let Err(e) = res {
            log::error!("Reading Room: cancel failed: {e}");
        }
    });
}

/// Give an issue's disk space back. Nothing sweeps the reading room, so this
/// is the only way a download leaves the disk - and the only thing that
/// takes a key out of the on-disk set.
pub fn remove(issue: &Issue, done: impl FnOnce(Result<(), String>) + 'static) {
    let core = app::core();
    let key = issue.key.clone();
    let runnable = issue.runnable;
    app::spawn(async move { cmd::remove_issue(core.state(), core.state(), key.clone()).await.map(|_| key) }, move |res| match res {
        Ok(key) => {
            bump_epoch(&key);
            stop_polling(&key);
            with(|s| {
                s.on_disk.remove(&key);
                // A "ready" status outlives the file it points at: the
                // reader would open a path that is gone.
                s.fetches.remove(&key);
            });
            if runnable {
                patch_issue(&key, |i| i.installed = false);
            }
            notify(Change::Issue(&key));
            done(Ok(()));
        }
        Err(e) => done(Err(e)),
    });
}

pub fn toggle_favorite(issue: &Issue) {
    let favorited = !issue.favorited;
    let key = issue.key.clone();
    patch_issue(&key, |i| i.favorited = favorited);
    notify(Change::Issue(&key));
    let core = app::core();
    let k = key.clone();
    app::spawn(async move { cmd::set_issue_favorited(core.state(), k, favorited).await }, move |res| {
        if let Err(e) = res {
            log::error!("Reading Room: favourite failed: {e}");
            patch_issue(&key, |i| i.favorited = !favorited);
            notify(Change::Issue(&key));
        }
    });
}

/// Remember the page the reader is on. Debounced per issue: page turns are
/// frequent and each one is a DB write.
pub fn remember_page(key: &str, page: i64) {
    patch_issue(key, |i| i.last_page = Some(page));
    if let Some(id) = with(|s| s.page_timers.remove(key)) {
        id.remove();
    }
    let k = key.to_string();
    let id = glib::timeout_add_local_once(Duration::from_secs(1), move || {
        with(|s| s.page_timers.remove(&k));
        let core = app::core();
        let key = k.clone();
        app::spawn(async move { cmd::set_issue_page(core.state(), key, page).await }, |res| {
            if let Err(e) = res {
                log::error!("Reading Room: could not store the page: {e}");
            }
        });
    });
    with(|s| s.page_timers.insert(key.to_string(), id));
}

/// Run a disk magazine under DOSBox.
pub fn launch(key: &str, done: impl FnOnce(Result<String, String>) + 'static) {
    let core = app::core();
    let k = key.to_string();
    app::spawn(async move { cmd::launch_issue(core.clone(), core.state(), k).await }, done);
}
