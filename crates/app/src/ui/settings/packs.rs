//! Content packs: the job store (in-flight installs, polled at 1 Hz, fed by
//! the backend's `content-pack-install-started` event) and the Content
//! Packs page. The pack row is shared with the Emulators page.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use exorchy_core::commands::content_packs::{self, ContentPackStatus};
use exorchy_core::commands::{games, setup};
use exorchy_core::media_sources::MEDIA_SOURCE;
use gtk::glib;

use super::widgets::{self, Ctx, Row};
use super::{is_emulator_pack, job_status_text, BASE_COLLECTION};
use crate::app;
use crate::ui::util::format_bytes;
use crate::ui::{bus, covers};

// ── Job store ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Job {
    pub phase: String,
    pub progress: f64,
    pub finished: bool,
    pub error: Option<String>,
}

impl Job {
    fn starting() -> Self {
        Self { phase: "starting".into(), progress: 0.0, finished: false, error: None }
    }
}

type Listener = (Rc<Cell<bool>>, Rc<dyn Fn()>);

struct Store {
    jobs: HashMap<String, Job>,
    labels: HashMap<String, String>,
    polling: HashSet<String>,
    in_flight: HashSet<String>,
    listeners: Vec<Listener>,
    /// Bumped whenever the installed set changed (a job landed, a removal).
    installed_gen: u64,
    events_armed: bool,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store {
        jobs: HashMap::new(),
        labels: HashMap::new(),
        polling: HashSet::new(),
        in_flight: HashSet::new(),
        listeners: Vec::new(),
        installed_gen: 0,
        events_armed: false,
    });
}

pub fn key(collection: &str, pack_id: &str) -> String {
    format!("{collection}:{pack_id}")
}

pub fn job(key: &str) -> Option<Job> {
    STORE.with(|s| s.borrow().jobs.get(key).cloned())
}

/// Every job the store tracks (in flight, or finished and about to be
/// cleared) with its display name, by name: the Transfers page.
pub fn jobs() -> Vec<(String, String, Job)> {
    STORE.with(|s| {
        let s = s.borrow();
        let mut v: Vec<_> = s.jobs.iter().map(|(k, j)| (k.clone(), s.labels.get(k).cloned().unwrap_or_else(|| k.clone()), j.clone())).collect();
        v.sort_by(|a, b| a.1.cmp(&b.1));
        v
    })
}

pub fn installed_gen() -> u64 {
    STORE.with(|s| s.borrow().installed_gen)
}

/// Run `f` on every change (job progress, a landed pack, a removal) while
/// `alive` holds; dead listeners are dropped on the next dispatch.
pub fn subscribe(alive: Rc<Cell<bool>>, f: impl Fn() + 'static) {
    STORE.with(|s| s.borrow_mut().listeners.push((alive, Rc::new(f))));
}

fn notify() {
    let live: Vec<Listener> = STORE.with(|s| {
        let mut s = s.borrow_mut();
        s.listeners.retain(|(alive, _)| alive.get());
        s.listeners.clone()
    });
    for (_, f) in live {
        f();
    }
}

fn set_job(key: &str, job: Job) {
    STORE.with(|s| s.borrow_mut().jobs.insert(key.to_string(), job));
    notify();
}

fn clear_job(key: &str) {
    let had = STORE.with(|s| s.borrow_mut().jobs.remove(key).is_some());
    if had {
        notify();
    }
}

fn label_of(key: &str) -> Option<String> {
    STORE.with(|s| s.borrow().labels.get(key).cloned())
}

fn installed_changed() {
    STORE.with(|s| s.borrow_mut().installed_gen += 1);
    // Tier resolution picks up a new poster dir without a restart.
    covers::load_dirs();
    notify();
}

/// Pick up jobs the backend starts by itself (the emulator auto-queue).
/// Registered once.
pub fn init_events() {
    let armed = STORE.with(|s| std::mem::replace(&mut s.borrow_mut().events_armed, true));
    if armed {
        return;
    }
    app::on_event("content-pack-install-started", |payload| {
        let get = |k: &str| payload.get(k).and_then(|v| v.as_str()).map(String::from);
        let (Some(collection), Some(pack_id)) = (get("collection"), get("pack_id")) else { return };
        let k = key(&collection, &pack_id);
        if let Some(name) = get("display_name") {
            STORE.with(|s| s.borrow_mut().labels.insert(k.clone(), name));
        }
        if job(&k).is_none() {
            set_job(&k, Job::starting());
        }
        start_polling(&collection, &pack_id);
    });
}

/// Probe the backend for a job the store does not know (started before the
/// page first opened) and follow it if one runs.
pub fn adopt_running(collection: &str, pack_id: &str) {
    let k = key(collection, pack_id);
    if job(&k).is_some() || STORE.with(|s| s.borrow().polling.contains(&k)) {
        return;
    }
    let (c, p) = (collection.to_string(), pack_id.to_string());
    let core = app::core();
    let (c2, p2) = (c.clone(), p.clone());
    app::spawn(
        async move { content_packs::get_content_pack_progress(core.state(), c2, p2).await },
        move |res| {
            if let Ok(Some(pr)) = res {
                if !pr.finished {
                    set_job(&k, Job { phase: pr.phase, progress: pr.progress, finished: false, error: pr.error });
                    start_polling(&c, &p);
                }
            }
        },
    );
}

fn start_polling(collection: &str, pack_id: &str) {
    let k = key(collection, pack_id);
    let fresh = STORE.with(|s| s.borrow_mut().polling.insert(k.clone()));
    if !fresh {
        return;
    }
    let (c, p) = (collection.to_string(), pack_id.to_string());
    glib::timeout_add_local(Duration::from_secs(1), move || {
        let (polling, busy) = STORE.with(|s| {
            let s = s.borrow();
            (s.polling.contains(&k), s.in_flight.contains(&k))
        });
        if !polling {
            return glib::ControlFlow::Break;
        }
        if busy {
            return glib::ControlFlow::Continue;
        }
        STORE.with(|s| s.borrow_mut().in_flight.insert(k.clone()));
        let core = app::core();
        let (c2, p2, k2) = (c.clone(), p.clone(), k.clone());
        app::spawn(
            async move { content_packs::get_content_pack_progress(core.state(), c2, p2).await },
            move |res| {
                STORE.with(|s| s.borrow_mut().in_flight.remove(&k2));
                on_progress(&k2, res);
            },
        );
        glib::ControlFlow::Continue
    });
}

fn stop_polling(key: &str) {
    STORE.with(|s| s.borrow_mut().polling.remove(key));
}

fn on_progress(k: &str, res: Result<Option<content_packs::ContentPackProgress>, String>) {
    let pr = match res {
        Ok(Some(pr)) => pr,
        // No job on the backend: drop the optimistic entry too, or the row
        // keeps showing a download nobody is running.
        Ok(None) | Err(_) => {
            stop_polling(k);
            clear_job(k);
            return;
        }
    };
    let finished = pr.finished;
    let installed = pr.installed;
    let error = pr.error.clone();
    set_job(k, Job { phase: pr.phase, progress: pr.progress, finished, error: pr.error });
    if finished {
        stop_polling(k);
        if installed {
            installed_changed();
        } else if let Some(e) = error.filter(|e| e != "Cancelled") {
            let name = label_of(k).unwrap_or_else(|| k.rsplit(':').next().unwrap_or(k).to_string());
            bus::toast_with(&format!("Couldn't install {name}"), Some(&e), None);
        }
        // The row shows "Installed!" / the failure for a moment.
        let k = k.to_string();
        glib::timeout_add_local_once(Duration::from_secs(5), move || clear_job(&k));
    }
}

/// Start an install; the row is claimed at once (the first poll is a second
/// out). `done` gets the backend's answer.
pub fn start_install(collection: &str, pack_id: &str, label: &str, done: impl FnOnce(Result<(), String>) + 'static) {
    let k = key(collection, pack_id);
    STORE.with(|s| s.borrow_mut().labels.insert(k.clone(), label.to_string()));
    set_job(&k, Job::starting());
    let core = app::core();
    let (c, p) = (collection.to_string(), pack_id.to_string());
    let (c2, p2) = (c.clone(), p.clone());
    app::spawn(async move { content_packs::install_content_pack(core.clone(), c2, p2).await }, move |res| {
        match &res {
            Ok(()) => start_polling(&c, &p),
            // Nothing runs, so the optimistic entry goes.
            Err(_) => clear_job(&k),
        }
        done(res);
    });
}

/// Cancel: the row returns to Install at once; the backend marks the job
/// cancelled on its own time.
pub fn cancel(collection: &str, pack_id: &str, done: impl FnOnce(Result<(), String>) + 'static) {
    let k = key(collection, pack_id);
    stop_polling(&k);
    clear_job(&k);
    let core = app::core();
    let (c, p) = (collection.to_string(), pack_id.to_string());
    app::spawn(async move { content_packs::cancel_content_pack_install(core.state(), c, p).await }, done);
}

/// Cancel every in-flight install (going offline) and say how many there were.
pub fn cancel_all() -> usize {
    let running: Vec<String> = STORE.with(|s| s.borrow().jobs.iter().filter(|(_, j)| !j.finished).map(|(k, _)| k.clone()).collect());
    for k in &running {
        if let Some((c, p)) = k.split_once(':') {
            cancel(c, p, |_| {});
        }
    }
    running.len()
}

pub fn remove(collection: &str, pack_id: &str, done: impl FnOnce(Result<(), String>) + 'static) {
    let core = app::core();
    let (c, p) = (collection.to_string(), pack_id.to_string());
    app::spawn(async move { content_packs::uninstall_content_pack(core.state(), core.state(), c, p).await }, move |res| {
        if res.is_ok() {
            installed_changed();
        }
        done(res);
    });
}

// ── The pack row ────────────────────────────────────────────────────────────

/// A pack's row: name, description · size, then its state (a running job,
/// installed with update/remove, superseded, install, coming soon). Returns
/// the row and a refresher the page calls on store changes.
pub fn pack_row(ctx: &Ctx, collection: &str, pack: Rc<ContentPackStatus>, superseded: bool) -> (Row, Rc<dyn Fn()>) {
    let row = Row::new(&pack.display_name).hint(&format!("{} · {}", pack.description, format_bytes(pack.size_bytes)));
    let status = widgets::status_label();
    let install = widgets::BusyButton::new("Install", "Starting…");
    let update = widgets::BusyButton::new("Update", "Starting…");
    let remove_btn = widgets::BusyButton::new("Remove", "Removing…");
    remove_btn.widget.add_css_class("danger");
    let cancel_btn = widgets::danger_button("Cancel");
    row.add_action(&status);
    row.add_action(&install.widget);
    row.add_action(&update.widget);
    row.add_action(&cancel_btn);
    row.add_action(&remove_btn.widget);
    let pending: Rc<RefCell<Option<&'static str>>> = Rc::new(RefCell::new(None));
    let k = key(collection, &pack.id);
    let collection = collection.to_string();
    adopt_running(&collection, &pack.id);

    let refresh: Rc<dyn Fn()> = {
        let (row, status, install, update, remove_btn, cancel_btn, pending, pack, k, ctx) =
            (row.clone(), status.clone(), install.clone(), update.clone(), remove_btn.clone(), cancel_btn.clone(), pending.clone(), pack.clone(), k.clone(), ctx.clone());
        Rc::new(move || {
            let job = job(&k);
            let offline = bus::offline();
            let pending = *pending.borrow();
            let has_update = pack.installed && pack.installed_version.map(|iv| iv < pack.version).unwrap_or(false);
            for b in [&install.widget, &update.widget, &remove_btn.widget, &cancel_btn] {
                b.set_visible(false);
            }
            status.set_visible(false);
            row.clear_below();
            let mut text = String::new();
            match job {
                Some(j) if !j.finished => {
                    text = job_status_text(&j.phase, j.progress, j.error.as_deref());
                    cancel_btn.set_visible(true);
                    row.set_below(&widgets::progress_bar(&ctx, j.progress, j.phase != "downloading"));
                }
                Some(j) => text = job_status_text(&j.phase, j.progress, j.error.as_deref()),
                None if pending == Some("remove") => text = "Removing…".into(),
                None if pack.installed => {
                    let iv = pack.installed_version.unwrap_or(pack.version);
                    text = if has_update { format!("v{iv} · v{} available", pack.version) } else { format!("Installed · v{iv}") };
                    update.widget.set_visible(has_update);
                    remove_btn.widget.set_visible(true);
                }
                None if superseded => text = "Included in another pack".into(),
                None if !pack.available => text = "Coming soon".into(),
                None => install.widget.set_visible(true),
            }
            let offline_tip = "Offline mode - nothing is downloaded. Enable downloads in Settings → Network.";
            for b in [&install, &update] {
                b.set_busy(pending == Some("install"));
                if !offline {
                    b.widget.set_sensitive(pending != Some("install"));
                }
                b.widget.set_tooltip_text(if offline { Some(offline_tip) } else { None });
            }
            update.widget.set_tooltip_text(Some(if offline { offline_tip.to_string() } else { format!("Re-downloads the whole pack ({})", format_bytes(pack.size_bytes)) }.as_str()));
            remove_btn.set_busy(pending == Some("remove"));
            status.set_label(&text);
            status.set_visible(!text.is_empty());
        })
    };

    let on_install = {
        let (pending, refresh, pack, collection) = (pending.clone(), refresh.clone(), pack.clone(), collection.clone());
        move || {
            *pending.borrow_mut() = Some("install");
            refresh();
            let (pending, refresh, name) = (pending.clone(), refresh.clone(), pack.display_name.clone());
            start_install(&collection, &pack.id, &pack.display_name, move |res| {
                *pending.borrow_mut() = None;
                if let Err(e) = res {
                    bus::toast_with(&format!("Couldn't install {name}"), Some(&e), None);
                }
                refresh();
            });
        }
    };
    let oi = on_install.clone();
    install.widget.connect_clicked(move |_| oi());
    update.widget.connect_clicked(move |_| on_install());

    remove_btn.widget.connect_clicked({
        let (pending, refresh, pack, collection) = (pending.clone(), refresh.clone(), pack.clone(), collection.clone());
        move |_| {
            *pending.borrow_mut() = Some("remove");
            refresh();
            let (pending, refresh, name) = (pending.clone(), refresh.clone(), pack.display_name.clone());
            remove(&collection, &pack.id, move |res| {
                *pending.borrow_mut() = None;
                match res {
                    Ok(()) => bus::toast(&format!("Removed {name}")),
                    Err(e) => bus::toast_with(&format!("Couldn't remove {name}"), Some(&e), None),
                }
                refresh();
            });
        }
    });
    cancel_btn.connect_clicked({
        let (pack, collection) = (pack.clone(), collection.clone());
        move |_| {
            let name = pack.display_name.clone();
            cancel(&collection, &pack.id, move |res| {
                if let Err(e) = res {
                    bus::toast_with(&format!("Couldn't cancel {name}"), Some(&e), None);
                }
            });
        }
    });

    refresh();
    (row, refresh)
}

/// One collection's packs, from the backend. A source without packs (or
/// a missing manifest) lists none; the reason is logged.
pub async fn list_packs(core: exorchy_core::host::AppHandle, collection: String) -> Vec<ContentPackStatus> {
    match content_packs::list_content_packs(core.state(), collection.clone()).await {
        Ok(list) => list,
        Err(e) => {
            log::warn!("content packs for {collection}: {e}");
            Vec::new()
        }
    }
}

// ── The page ────────────────────────────────────────────────────────────────

struct CollectionPacks {
    id: String,
    label: String,
    packs: Vec<ContentPackStatus>,
}

pub fn build(ctx: &Ctx) -> gtk::Widget {
    let page = widgets::page("Content Packs");
    let groups: Rc<RefCell<Vec<adw::PreferencesGroup>>> = Rc::new(RefCell::new(Vec::new()));
    let refreshers: widgets::Refreshers = Rc::new(RefCell::new(Vec::new()));
    let seen_gen = Rc::new(Cell::new(installed_gen()));

    let render: Rc<dyn Fn(Vec<CollectionPacks>)> = {
        let (page, groups, refreshers, ctx) = (page.clone(), groups.clone(), refreshers.clone(), ctx.clone());
        Rc::new(move |cols: Vec<CollectionPacks>| {
            for g in groups.borrow_mut().drain(..) {
                page.remove(&g);
            }
            refreshers.borrow_mut().clear();
            let intro = "Optional downloads that enhance your library with box art and media. Each language pack has its own metadata set.";
            if cols.is_empty() {
                let g = widgets::group("Content Packs", Some(intro));
                g.add(&widgets::note("No content packs available."));
                page.add(&g);
                groups.borrow_mut().push(g);
                return;
            }
            let many = cols.len() > 1;
            if many {
                let head = widgets::group("Content Packs", Some(intro));
                page.add(&head);
                groups.borrow_mut().push(head);
            }
            for col in cols {
                let title = if many { col.label.as_str() } else { "Content Packs" };
                let g = widgets::group(title, if many { None } else { Some(intro) });
                let packs: Vec<Rc<ContentPackStatus>> = col.packs.into_iter().map(Rc::new).collect();
                for pack in &packs {
                    let superseded = packs.iter().any(|o| o.installed && o.supersedes.contains(&pack.id));
                    let (row, refresh) = pack_row(&ctx, &col.id, pack.clone(), superseded);
                    g.add(&row.widget);
                    refreshers.borrow_mut().push(refresh);
                }
                page.add(&g);
                groups.borrow_mut().push(g);
            }
        })
    };

    let load: Rc<dyn Fn()> = {
        let render = render.clone();
        let ctx = ctx.clone();
        Rc::new(move || {
            let core = app::core();
            let render = render.clone();
            let ctx = ctx.clone();
            app::spawn(
                async move {
                    let raw = games::get_config(core.state(), "collections".into()).await.ok().flatten();
                    let available = setup::get_available_collections(core.state()).await.unwrap_or_default();
                    let labels: HashMap<String, String> = available.into_iter().map(|c| (c.id, c.display_name)).collect();
                    let mut ids = super::parse_collections(raw.as_deref());
                    ids.sort_by(|a, b| {
                        if a == BASE_COLLECTION { std::cmp::Ordering::Less } else if b == BASE_COLLECTION { std::cmp::Ordering::Greater } else { a.cmp(b) }
                    });
                    let mut out = Vec::new();
                    for id in ids {
                        let packs: Vec<ContentPackStatus> =
                            list_packs(core.clone(), id.clone()).await.into_iter().filter(|p| !is_emulator_pack(&p.id)).collect();
                        out.push(CollectionPacks { label: labels.get(&id).cloned().unwrap_or_else(|| id.clone()), id, packs });
                    }
                    // The reading room's covers are a pack too, but their
                    // source is not a collection: appended, not configured.
                    let media = list_packs(core.clone(), MEDIA_SOURCE.into()).await;
                    out.push(CollectionPacks { id: MEDIA_SOURCE.into(), label: "Reading Room".into(), packs: media });
                    out.retain(|c| !c.packs.is_empty());
                    out
                },
                move |cols| {
                    if ctx.is_alive() {
                        render(cols);
                    }
                },
            );
        })
    };
    load();

    subscribe(ctx.alive.clone(), {
        let (load, refreshers, seen_gen) = (load.clone(), refreshers.clone(), seen_gen.clone());
        move || {
            if installed_gen() != seen_gen.get() {
                seen_gen.set(installed_gen());
                load();
            } else {
                for r in refreshers.borrow().iter() {
                    r();
                }
            }
        }
    });
    page.upcast()
}
