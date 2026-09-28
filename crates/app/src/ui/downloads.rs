//! Download trackers: one per in-flight game, polling `get_download_progress`
//! at 1 Hz. The poll is what shows progress AND what extracts on completion
//! (the backend installs from inside the poll), so every download the
//! session knows about needs a tracker: started here, resumed after a
//! restart, or begun by the backend itself (an English base for a
//! translation). Mirrors the web UI's `stores/downloads.ts`, invariants
//! included.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use exorchy_core::commands::{games, install};
use gtk::glib;

use crate::app;
use crate::ui::bus;

#[derive(Debug, Clone, Default)]
pub struct DownloadState {
    pub status: String,
    pub progress: f64,
    pub downloading: bool,
    /// True from the moment the game itself is playable (extras may still
    /// be downloading).
    pub installed: bool,
    pub title: Option<String>,
}

struct Tracker {
    game_id: i64,
    title: Option<String>,
    cancelled: bool,
    command_pending: bool,
    null_polls: u32,
    stuck_since: Option<Instant>,
    max_progress: f64,
    announced_installed: bool,
    last_progress_val: f64,
    last_progress_at: Instant,
    last_torrent_val: f64,
    last_torrent_at: Instant,
    /// A poll is in flight; the timer skips a beat rather than overlap.
    polling: bool,
}

type Change = Rc<dyn Fn(i64)>;

struct Store {
    states: HashMap<i64, DownloadState>,
    trackers: HashMap<i64, Rc<RefCell<Tracker>>>,
    listeners: Vec<Change>,
    /// Session download rate, for the stall heuristic.
    download_bps: u64,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store {
        states: HashMap::new(),
        trackers: HashMap::new(),
        listeners: Vec::new(),
        download_bps: 0,
    });
}

const NULL_POLL_THRESHOLD: u32 = 5;
const STALL_HINT_SECS: f64 = 15.0;
const STALL_WARN_SECS: f64 = 90.0;

pub fn state(id: i64) -> Option<DownloadState> {
    STORE.with(|s| s.borrow().states.get(&id).cloned())
}

pub fn is_downloading(id: i64) -> bool {
    state(id).map(|s| s.downloading).unwrap_or(false)
}

pub fn active_ids() -> Vec<i64> {
    STORE.with(|s| s.borrow().states.iter().filter(|(_, v)| v.downloading).map(|(k, _)| *k).collect())
}

/// Called with a game id whenever its state changes or is cleared.
pub fn on_change(f: impl Fn(i64) + 'static) {
    STORE.with(|s| s.borrow_mut().listeners.push(Rc::new(f)));
}

pub fn set_download_bps(bps: u64) {
    STORE.with(|s| s.borrow_mut().download_bps = bps);
}

fn emit(id: i64) {
    let cbs: Vec<Change> = STORE.with(|s| s.borrow().listeners.clone());
    for cb in cbs {
        cb(id);
    }
}

fn set_state(t: &Rc<RefCell<Tracker>>, status: impl Into<String>, progress: f64, downloading: bool, installed: bool) {
    let (id, title) = {
        let t = t.borrow();
        (t.game_id, t.title.clone())
    };
    STORE.with(|s| {
        s.borrow_mut().states.insert(id, DownloadState { status: status.into(), progress, downloading, installed, title });
    });
    emit(id);
}

fn clear_state(id: i64) {
    let had = STORE.with(|s| s.borrow_mut().states.remove(&id).is_some());
    if had {
        emit(id);
    }
}

/// End a run and drop it from the registry; safe on a tracker a newer
/// attempt already replaced.
fn end_tracker(t: &Rc<RefCell<Tracker>>) {
    t.borrow_mut().cancelled = true;
    let id = t.borrow().game_id;
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        if s.trackers.get(&id).map(|cur| Rc::ptr_eq(cur, t)).unwrap_or(false) {
            s.trackers.remove(&id);
        }
    });
}

fn new_tracker(id: i64, title: Option<String>, command_pending: bool) -> Rc<RefCell<Tracker>> {
    let now = Instant::now();
    let t = Rc::new(RefCell::new(Tracker {
        game_id: id,
        title,
        cancelled: false,
        command_pending,
        null_polls: 0,
        stuck_since: None,
        max_progress: 0.0,
        announced_installed: false,
        last_progress_val: -1.0,
        last_progress_at: now,
        last_torrent_val: -1.0,
        last_torrent_at: now,
        polling: false,
    }));
    STORE.with(|s| s.borrow_mut().trackers.insert(id, t.clone()));
    t
}

fn has_tracker(id: i64) -> bool {
    STORE.with(|s| s.borrow().trackers.contains_key(&id))
}

/// The self-scheduling poll: a 1 s timer that stops once its tracker is
/// cancelled.
fn poll(t: Rc<RefCell<Tracker>>) {
    glib::timeout_add_local(Duration::from_secs(1), move || {
        if t.borrow().cancelled {
            return glib::ControlFlow::Break;
        }
        if t.borrow().polling {
            return glib::ControlFlow::Continue;
        }
        t.borrow_mut().polling = true;
        let id = t.borrow().game_id;
        let core = app::core();
        let t2 = t.clone();
        app::spawn(
            async move { install::get_download_progress(core.state(), core.state(), id).await },
            move |res| {
                t2.borrow_mut().polling = false;
                if t2.borrow().cancelled {
                    return;
                }
                match res {
                    Ok(p) => tick(&t2, p),
                    Err(e) => log::error!("download poll for game {id}: {e}"),
                }
            },
        );
        glib::ControlFlow::Continue
    });
}

fn tick(t: &Rc<RefCell<Tracker>>, p: Option<exorchy_core::torrent::manager::DownloadProgress>) {
    let id = t.borrow().game_id;
    let Some(p) = p else {
        let pending = t.borrow().command_pending;
        if pending {
            t.borrow_mut().null_polls = 0;
            if t.borrow().last_progress_at.elapsed().as_secs_f64() > 8.0 {
                set_state(t, "Preparing the collection (one-time setup, can take a few minutes)…", 0.0, true, false);
            }
            return;
        }
        let n = {
            let mut tt = t.borrow_mut();
            tt.null_polls += 1;
            tt.null_polls
        };
        if n >= NULL_POLL_THRESHOLD {
            end_tracker(t);
            set_state(t, "Download didn't start - open Settings → Diagnostics to view exorchy.log.", 0.0, false, false);
        }
        return;
    };

    t.borrow_mut().null_polls = 0;
    let safe = {
        let mut tt = t.borrow_mut();
        tt.max_progress = tt.max_progress.max(p.progress);
        tt.max_progress
    };

    if let Some(err) = p.error {
        end_tracker(t);
        set_state(t, err.clone(), 0.0, false, false);
        let title = t.borrow().title.clone();
        bus::toast_with(&title.map(|x| format!("Download failed: {x}")).unwrap_or_else(|| "Download failed".into()), Some(&err), None);
        return;
    }

    if p.installed {
        if p.extras_done == Some(false) {
            let pct = (p.extras_progress.unwrap_or(0.0) * 100.0).round() as i64;
            let first = !t.borrow().announced_installed;
            if first {
                t.borrow_mut().announced_installed = true;
                bus::notify_library_changed(id);
            }
            set_state(t, format!("Installed - downloading extras… {pct}%"), 1.0, false, true);
            return;
        }
        end_tracker(t);
        set_state(t, "Installed!", 1.0, false, true);
        bus::notify_library_changed(id);
        glib::timeout_add_local_once(Duration::from_secs(5), move || {
            if !has_tracker(id) {
                clear_state(id);
            }
        });
        return;
    }

    if p.finished {
        t.borrow_mut().stuck_since = None;
        let status = match p.waiting_for {
            Some(w) => format!("Waiting for {w}…"),
            None => "Extracting...".to_string(),
        };
        set_state(t, status, safe, true, false);
        return;
    }

    if safe >= 0.999 {
        let since = {
            let mut tt = t.borrow_mut();
            *tt.stuck_since.get_or_insert_with(Instant::now)
        };
        let status = if since.elapsed().as_secs() > 30 {
            "Waiting for last pieces… try cancelling and re-downloading if this persists"
        } else {
            "100%"
        };
        set_state(t, status, safe, true, false);
        return;
    }
    t.borrow_mut().stuck_since = None;

    if p.torrent_state.as_deref() == Some("initializing") {
        let tp = p.torrent_progress.unwrap_or(0.0);
        set_state(t, format!("Validating torrent {:.0}% (first run can take several minutes)", tp * 100.0), tp, true, false);
        return;
    }

    let now = Instant::now();
    let tp = p.torrent_progress.unwrap_or(0.0);
    let (stalled, piece_advanced) = {
        let mut tt = t.borrow_mut();
        if safe > tt.last_progress_val {
            tt.last_progress_val = safe;
            tt.last_progress_at = now;
        }
        if tp > tt.last_torrent_val {
            tt.last_torrent_val = tp;
            tt.last_torrent_at = now;
        }
        (
            now.duration_since(tt.last_progress_at).as_secs_f64(),
            now.duration_since(tt.last_torrent_at).as_secs_f64() < STALL_HINT_SECS,
        )
    };
    let bytes_flowing = STORE.with(|s| s.borrow().download_bps) >= 1024;
    let receiving = piece_advanced || bytes_flowing;
    let pct = format!("{:.0}%", safe * 100.0);
    let status = if stalled >= STALL_HINT_SECS && receiving {
        format!("{pct} - fetching a shared data block…")
    } else if stalled >= STALL_WARN_SECS {
        format!("Stalled at {pct} - no data received. Check your connection, or cancel and retry.")
    } else if stalled >= STALL_HINT_SECS {
        if safe == 0.0 { "Looking for peers…".to_string() } else { format!("{pct} - waiting for peers…") }
    } else {
        pct
    };
    set_state(t, status, safe, true, false);
}

/// Start a download and track it.
pub fn start(id: i64, title: Option<String>) {
    let previous = STORE.with(|s| s.borrow().trackers.get(&id).cloned());
    if let Some(p) = &previous {
        end_tracker(p);
    }
    let title = title.or_else(|| previous.as_ref().and_then(|p| p.borrow().title.clone())).or_else(|| state(id).and_then(|s| s.title));
    let t = new_tracker(id, title, true);
    set_state(&t, "Starting download...", 0.0, true, false);
    poll(t.clone());
    let core = app::core();
    app::spawn(
        async move { install::download_game(core.clone(), core.state(), core.state(), id).await },
        move |res| {
            if t.borrow().cancelled {
                return;
            }
            match res {
                Ok(_) => t.borrow_mut().command_pending = false,
                Err(e) => {
                    end_tracker(&t);
                    set_state(&t, format!("Error: {e}"), 0.0, false, false);
                    let title = t.borrow().title.clone();
                    bus::toast_with(
                        &title.map(|x| format!("Couldn't start download: {x}")).unwrap_or_else(|| "Couldn't start download".into()),
                        Some(&e),
                        None,
                    );
                }
            }
        },
    );
}

/// Re-arm trackers for every download the session resumed on its own.
pub fn resume_all() {
    let core = app::core();
    app::spawn(
        async move { install::list_active_downloads(core.state(), core.state()).await },
        |res| {
            let Ok(pending) = res else { return };
            for p in pending {
                if has_tracker(p.id) {
                    continue;
                }
                let t = new_tracker(p.id, Some(p.title), false);
                set_state(&t, "Resuming download…", 0.0, true, false);
                poll(t);
            }
        },
    );
}

/// Pick up downloads the backend starts by itself (`dependency-download-started`).
pub fn init_dependency_downloads() {
    app::on_event("dependency-download-started", |payload| {
        let id = payload.get("id").and_then(|v| v.as_i64());
        let title = payload.get("title").and_then(|v| v.as_str()).map(String::from);
        let Some(id) = id else { return };
        if has_tracker(id) {
            return;
        }
        let t = new_tracker(id, title, false);
        set_state(&t, "Starting download...", 0.0, true, false);
        poll(t);
    });
}

/// Stop tracking a game in any phase.
pub fn stop_tracking(id: i64) {
    if let Some(t) = STORE.with(|s| s.borrow().trackers.get(&id).cloned()) {
        end_tracker(&t);
    }
    clear_state(id);
}

/// Stop tracking everything (going offline drops the managers). Returns the count.
pub fn stop_all() -> usize {
    let active = active_ids();
    for id in &active {
        stop_tracking(*id);
    }
    active.len()
}

/// Cancel a download; translations waiting on the cancelled base go too.
pub fn cancel(id: i64) {
    stop_tracking(id);
    let core = app::core();
    app::spawn(
        async move { install::cancel_download(core.state(), core.state(), id).await },
        move |res| {
            if !has_tracker(id) {
                clear_state(id);
            }
            if let Ok(also) = res {
                for dep in &also {
                    stop_tracking(dep.id);
                }
                if also.len() == 1 {
                    bus::toast(&format!("{} was cancelled too - it needs the English version", also[0].title));
                } else if also.len() > 1 {
                    bus::toast(&format!("{} translations were cancelled too - they need the English version", also.len()));
                }
            }
            bus::notify_library_changed(id);
        },
    );
}

/// Resume polling an installed game's extras after a restart.
pub fn watch_extras_if_pending(id: i64, title: Option<String>) {
    if has_tracker(id) || state(id).is_some() {
        return;
    }
    let core = app::core();
    app::spawn(
        async move { install::get_download_progress(core.state(), core.state(), id).await },
        move |res| {
            if let Ok(Some(p)) = res {
                if p.installed && p.extras_done == Some(false) {
                    start(id, title);
                }
            }
        },
    );
}

/// Session transfer stats every 2 s: the download rate feeds the stall
/// heuristic; listeners (the activity badge) get the whole struct.
pub fn start_transfer_polling(on_stats: impl Fn(&games::TransferStats) + 'static) {
    let on_stats = Rc::new(on_stats);
    glib::timeout_add_local(Duration::from_secs(2), move || {
        let core = app::core();
        let cb = on_stats.clone();
        app::spawn(async move { games::get_transfer_stats(core.state()).await }, move |res| {
            if let Ok(s) = res {
                set_download_bps(s.download_bps);
                cb(&s);
            }
        });
        glib::ControlFlow::Continue
    });
}
