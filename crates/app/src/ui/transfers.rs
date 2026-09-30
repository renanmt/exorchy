//! The Transfers page: what the torrent session and the fetchers are doing,
//! opened from the toolbar's connection badge in place of the library (like
//! Settings; back arrow or Esc returns). Three lists:
//! - Downloading: games, content packs, Reading Room documents, preview and
//!   theme fetches holding a slot, each with its progress and a Cancel where
//!   the store has one;
//! - Queued: preview and theme fetches waiting for one of the media slots
//!   (the only queue there is: game files all download at once);
//! - Torrents: every torrent in the shared session with its own rates, peers
//!   and upload this session, which is what shows what is being seeded.
//!
//! A 1 s poll updates rows in place (keyed), so hover and clicks survive it.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use exorchy_core::commands::games::{self, TorrentRow, TransferStats};
use gtk::glib;

use crate::app;
use crate::ui::util::format_bytes;
use crate::ui::{bus, downloads};

/// The window stack's name for the page.
const PAGE: &str = "transfers";

/// Open the page in place of the library.
pub fn open(parent: &impl IsA<gtk::Widget>) {
    let Some(window) = parent.root().and_then(|r| r.downcast::<gtk::Window>().ok()) else {
        log::warn!("transfers: parent has no window yet");
        return;
    };
    let view = build(&window);
    crate::ui::window::show_page(PAGE, &view);
}

/// One line of a list: what it is, its state, how far, and how to stop it.
struct Item {
    key: String,
    title: String,
    detail: String,
    progress: Option<f64>,
    cancel: Option<Rc<dyn Fn()>>,
}

struct Row {
    widget: gtk::ListBoxRow,
    title: gtk::Label,
    detail: gtk::Label,
    bar: gtk::ProgressBar,
}

/// A titled list whose rows are kept by key between polls.
struct Section {
    group: gtk::Box,
    list: gtk::ListBox,
    empty: gtk::Label,
    rows: RefCell<HashMap<String, Row>>,
    /// Hide the whole section while it has nothing (Queued), instead of
    /// showing its empty note.
    hide_when_empty: bool,
}

impl Section {
    fn new(title: &str, empty: &str, hide_when_empty: bool) -> Self {
        let group = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
        group.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["transfers-heading"]).build());
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list", "transfers-list"]).build();
        let empty = gtk::Label::builder().label(empty).xalign(0.0).wrap(true).css_classes(["muted", "small"]).build();
        group.append(&list);
        group.append(&empty);
        Section { group, list, empty, rows: RefCell::new(HashMap::new()), hide_when_empty }
    }

    fn sync(&self, items: Vec<Item>) {
        let mut rows = self.rows.borrow_mut();
        let keep: HashSet<&str> = items.iter().map(|i| i.key.as_str()).collect();
        rows.retain(|k, r| {
            let stay = keep.contains(k.as_str());
            if !stay {
                self.list.remove(&r.widget);
            }
            stay
        });
        for item in items {
            let row = rows.entry(item.key.clone()).or_insert_with(|| {
                let row = new_row(item.cancel.clone());
                self.list.append(&row.widget);
                row
            });
            row.title.set_label(&item.title);
            row.detail.set_label(&item.detail);
            row.detail.set_visible(!item.detail.is_empty());
            match item.progress {
                Some(p) => {
                    row.bar.set_fraction(p.clamp(0.0, 1.0));
                    row.bar.set_visible(true);
                }
                None => row.bar.set_visible(false),
            }
        }
        let has = !rows.is_empty();
        self.list.set_visible(has);
        self.empty.set_visible(!has && !self.hide_when_empty);
        if self.hide_when_empty {
            self.group.set_visible(has);
        }
    }
}

fn new_row(cancel: Option<Rc<dyn Fn()>>) -> Row {
    let title = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["transfer-title"]).build();
    let detail = gtk::Label::builder().xalign(0.0).wrap(true).wrap_mode(gtk::pango::WrapMode::WordChar).css_classes(["muted", "small"]).build();
    let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
    text.append(&title);
    text.append(&detail);
    let line = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).build();
    line.append(&text);
    if let Some(cancel) = cancel {
        let button = gtk::Button::builder().label("Cancel").valign(gtk::Align::Center).css_classes(["btn"]).build();
        button.connect_clicked(move |b| {
            b.set_sensitive(false);
            cancel();
        });
        line.append(&button);
    }
    let bar = gtk::ProgressBar::builder().css_classes(["mini"]).visible(false).build();
    let outer = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
    outer.append(&line);
    outer.append(&bar);
    let widget = gtk::ListBoxRow::builder().activatable(false).selectable(false).css_classes(["transfer-row"]).child(&outer).build();
    Row { widget, title, detail, bar }
}

/// The page's state between polls.
struct View {
    alive: Rc<Cell<bool>>,
    rates: gtk::Label,
    session: gtk::Label,
    limits: gtk::Label,
    downloading: Section,
    queued: Section,
    torrents: Section,
    /// Game titles for preview and theme fetches (the media store keeps ids).
    titles: RefCell<HashMap<i64, String>>,
    asked: RefCell<HashSet<i64>>,
    polling: Cell<bool>,
    /// Per torrent: when it was last read, uploaded and on-disk bytes then.
    /// librqbit's per-torrent speed estimates read zero while the session
    /// total moves, so the page derives each torrent's rates from these.
    counters: RefCell<HashMap<String, (std::time::Instant, u64, u64)>>,
}

fn build(window: &gtk::Window) -> gtk::Widget {
    let alive = Rc::new(Cell::new(true));

    // ── header ──
    let back = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .css_classes(["btn", "icon", "ghost"])
        .tooltip_text("Back to the library (Esc)")
        .build();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .title_widget(&adw::WindowTitle::new("Transfers", ""))
        .css_classes(["transfers-header"])
        .build();
    header.pack_start(&back);

    // ── summary ──
    let rates = gtk::Label::builder().xalign(0.0).css_classes(["transfers-rates"]).build();
    let session = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["muted"]).build();
    let limits = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["muted", "small"]).build();
    let network = gtk::Button::builder().label("Network settings").halign(gtk::Align::Start).css_classes(["btn"]).build();
    let summary = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(["transfers-summary"]).build();
    summary.append(&rates);
    summary.append(&session);
    summary.append(&limits);
    summary.append(&network);

    let downloading = Section::new("Downloading", "Nothing is downloading.", false);
    let queued = Section::new("Queued", "", true);
    let torrents = Section::new(
        "Torrents",
        "No torrent is running. Torrents start with the first download, or stay off in offline mode.",
        false,
    );

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .margin_top(16)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    body.append(&summary);
    body.append(&downloading.group);
    body.append(&queued.group);
    body.append(&torrents.group);
    let clamp = adw::Clamp::builder().maximum_size(960).tightening_threshold(720).child(&body).build();
    let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&clamp).build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scroller));
    let root = adw::Bin::builder().css_classes(["transfers-view"]).child(&toolbar).build();

    let view = Rc::new(View {
        alive: alive.clone(),
        rates,
        session,
        limits,
        downloading,
        queued,
        torrents,
        titles: RefCell::new(HashMap::new()),
        asked: RefCell::new(HashSet::new()),
        polling: Cell::new(false),
        counters: RefCell::new(HashMap::new()),
    });

    // ── leaving ──
    let close: Rc<dyn Fn()> = {
        let alive = alive.clone();
        Rc::new(move || {
            if alive.replace(false) {
                crate::ui::window::close_page(PAGE);
            }
        })
    };
    root.connect_unrealize({
        let alive = alive.clone();
        move |_| alive.set(false)
    });
    back.connect_clicked({
        let close = close.clone();
        move |_| close()
    });
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let close = close.clone();
        move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                close();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    });
    root.add_controller(keys);
    network.connect_clicked({
        let (close, window) = (close.clone(), window.clone());
        move |_| {
            close();
            crate::ui::settings::open(&window, "network");
        }
    });

    load_limits(&view);
    poll(&view);
    glib::timeout_add_local(Duration::from_secs(1), {
        let view = view.clone();
        move || {
            if !view.alive.get() {
                return glib::ControlFlow::Break;
            }
            poll(&view);
            glib::ControlFlow::Continue
        }
    });
    root.upcast()
}

/// The seeding switch and rate caps, read once (they change in Settings,
/// which leaves this page).
fn load_limits(view: &Rc<View>) {
    let core = app::core();
    let view = view.clone();
    app::spawn(
        async move {
            let get = |key: &'static str| {
                let core = core.clone();
                async move { games::get_config(core.state(), key.into()).await.ok().flatten() }
            };
            (get("seeding_enabled").await, get("rate_limit_up_kbps").await, get("rate_limit_down_kbps").await)
        },
        move |(seeding, up, down)| view.limits.set_label(&limits_text(seeding.as_deref(), up.as_deref(), down.as_deref())),
    );
}

fn limits_text(seeding: Option<&str>, up: Option<&str>, down: Option<&str>) -> String {
    let seeding = if seeding == Some("0") { "Seeding off" } else { "Seeding on" };
    let cap = |v: Option<&str>| match v.and_then(|s| s.parse::<u64>().ok()).filter(|n| *n > 0) {
        Some(kbps) => format!("{}/s", format_bytes(kbps * 1024)),
        None => "unlimited".into(),
    };
    format!("{seeding} · upload {} · download {}", cap(up), cap(down))
}

fn poll(view: &Rc<View>) {
    sync_local(view);
    if view.polling.replace(true) {
        return;
    }
    let core = app::core();
    let view = view.clone();
    app::spawn(
        async move {
            let stats = games::get_transfer_stats(core.state()).await.unwrap_or_default();
            let torrents = games::get_session_torrents(core.state()).await.unwrap_or_default();
            (stats, torrents)
        },
        move |(stats, torrents)| {
            view.polling.set(false);
            if view.alive.get() {
                show_session(&view, &stats, &torrents);
            }
        },
    );
}

fn show_session(view: &View, stats: &TransferStats, torrents: &[TorrentRow]) {
    view.rates.set_label(&format!("↓ {}/s    ↑ {}/s", format_bytes(stats.download_bps), format_bytes(stats.upload_bps)));
    view.session.set_label(&if bus::offline() {
        "Offline mode: no torrents run until you switch back in Network settings.".to_string()
    } else if !stats.active {
        "Idle: no torrent is running.".to_string()
    } else {
        format!("{} peers connected · {} uploaded this session", stats.peers, format_bytes(stats.uploaded_bytes))
    });
    let now = std::time::Instant::now();
    let mut counters = view.counters.borrow_mut();
    let items = torrents
        .iter()
        .map(|row| {
            let t = &row.transfer;
            let (down, up) = match counters.get(&t.info_hash) {
                Some((at, up_then, disk_then)) => {
                    let secs = now.duration_since(*at).as_secs_f64().max(0.5);
                    let rate = |now_b: u64, then_b: u64| (now_b.saturating_sub(then_b) as f64 / secs) as u64;
                    (rate(t.progress_bytes, *disk_then), rate(t.uploaded_bytes, *up_then))
                }
                None => (0, 0),
            };
            counters.insert(t.info_hash.clone(), (now, t.uploaded_bytes, t.progress_bytes));
            torrent_item(row, down, up)
        })
        .collect();
    view.torrents.sync(items);
}

/// A torrent row: what it is doing, its rates when it moves, peers when it
/// has some, and what it has on disk. The session totals are the summary's.
fn torrent_item(row: &TorrentRow, down_bps: u64, up_bps: u64) -> Item {
    let t = &row.transfer;
    let what = match t.state.as_str() {
        _ if t.error.is_some() => format!("Error: {}", t.error.as_deref().unwrap_or_default()),
        "initializing" => "Checking files".into(),
        "paused" => "Paused".into(),
        "error" => "Error".into(),
        _ if t.finished && up_bps > 0 => "Seeding".into(),
        _ if t.finished => "Seeding, idle".into(),
        _ => "Downloading".into(),
    };
    let mut parts = vec![what];
    if down_bps > 0 {
        parts.push(format!("↓ {}/s", format_bytes(down_bps)));
    }
    if up_bps > 0 {
        parts.push(format!("↑ {}/s", format_bytes(up_bps)));
    }
    if t.peers > 0 {
        parts.push(if t.peers == 1 { "1 peer".into() } else { format!("{} peers", t.peers) });
    }
    if t.uploaded_bytes > 0 {
        parts.push(format!("{} uploaded", format_bytes(t.uploaded_bytes)));
    }
    if t.total_bytes > 0 {
        parts.push(if t.finished {
            format!("{} on disk", format_bytes(t.total_bytes))
        } else {
            format!("{} of {} on disk", format_bytes(t.progress_bytes), format_bytes(t.total_bytes))
        });
    }
    let progress = (!t.finished && t.total_bytes > 0).then(|| t.progress_bytes as f64 / t.total_bytes as f64);
    Item { key: t.info_hash.clone(), title: row.label.clone(), detail: parts.join(" · "), progress, cancel: None }
}

/// The lists the UI stores already know: game downloads, content packs,
/// Reading Room documents, preview and theme fetches.
fn sync_local(view: &Rc<View>) {
    let mut downloading = Vec::new();
    let mut queued = Vec::new();

    let mut ids = downloads::active_ids();
    ids.sort();
    for id in ids {
        let Some(s) = downloads::state(id) else { continue };
        downloading.push(Item {
            key: format!("game:{id}"),
            title: s.title.clone().unwrap_or_else(|| format!("Game {id}")),
            detail: s.status.clone(),
            progress: Some(s.progress),
            cancel: Some(Rc::new(move || downloads::cancel(id))),
        });
    }

    for (key, label, job) in crate::ui::settings::packs::jobs() {
        if job.finished {
            continue;
        }
        let (collection, pack) = key.split_once(':').map(|(c, p)| (c.to_string(), p.to_string())).unwrap_or_default();
        let detail = match &job.error {
            Some(e) => format!("Content pack · {e}"),
            None => format!("Content pack · {}", job.phase),
        };
        downloading.push(Item {
            key: format!("pack:{key}"),
            title: label,
            detail,
            progress: Some(job.progress),
            cancel: Some(Rc::new(move || crate::ui::settings::packs::cancel(&collection, &pack, |_| {}))),
        });
    }

    for (key, title, status) in crate::ui::reading::store::active_fetches() {
        let queued_now = status.phase == crate::ui::reading::store::PHASE_QUEUED;
        let k = key.clone();
        downloading.push(Item {
            key: format!("reading:{key}"),
            title,
            detail: if queued_now { "Reading Room · starting".into() } else { format!("Reading Room · {}", size_text(status.total_bytes)) },
            progress: (!queued_now).then_some(status.progress),
            cancel: Some(Rc::new(move || crate::ui::reading::store::abort(&k))),
        });
    }

    let mut missing = Vec::new();
    for (kind, id, status, waiting) in crate::ui::media::fetches() {
        let what = match kind {
            crate::ui::media::Kind::Video => "Preview video",
            crate::ui::media::Kind::Music => "Theme music",
        };
        let title = match view.titles.borrow().get(&id) {
            Some(t) => t.clone(),
            None => {
                missing.push(id);
                format!("Game {id}")
            }
        };
        let item = Item {
            key: format!("media:{what}:{id}"),
            title,
            detail: match &status {
                Some(s) if !waiting => format!("{what} · {}", s.phase),
                _ => format!("{what} · waiting for a free slot"),
            },
            progress: status.as_ref().filter(|_| !waiting).map(|s| s.progress),
            cancel: None,
        };
        if waiting {
            queued.push(item);
        } else {
            downloading.push(item);
        }
    }
    fetch_titles(view, missing);

    view.downloading.sync(downloading);
    view.queued.sync(queued);
}

fn size_text(total: u64) -> String {
    if total > 0 {
        format_bytes(total)
    } else {
        "fetching".into()
    }
}

/// Look up game titles for media fetches once each.
fn fetch_titles(view: &Rc<View>, ids: Vec<i64>) {
    for id in ids {
        if !view.asked.borrow_mut().insert(id) {
            continue;
        }
        let core = app::core();
        let view = view.clone();
        app::spawn(async move { games::get_game(core.state(), id).await }, move |res| {
            if let Ok(Some(game)) = res {
                view.titles.borrow_mut().insert(id, game.title);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_read_the_stored_config() {
        assert_eq!(limits_text(None, None, None), "Seeding on · upload unlimited · download unlimited");
        assert_eq!(limits_text(Some("0"), Some("512"), Some("0")), "Seeding off · upload 512 KB/s · download unlimited");
    }
}
