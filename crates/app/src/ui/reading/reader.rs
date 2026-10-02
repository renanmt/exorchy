//! The issue reader: shown in place of the Reading Room's body (the room
//! mounts `Reader::widget`). It shows the fetch (cover, phase, progress,
//! cancel / retry) until the document is on disk, then the document viewer
//! at the remembered page, with a full-screen toggle. Also the media notice:
//! opening an issue joins a second torrent, said once before the first fetch.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use exorchy_core::commands::games;
use exorchy_core::models::Issue;
use gtk::glib;

use super::logic::{failure_detail, kind_label};
use super::store::{self, Change, PHASE_QUEUED};
use crate::app;
use crate::ui::pdf::DocumentView;
use crate::ui::util::format_bytes;
use crate::ui::{bus, covers, dialogs};

// ── Media notice ─────────────────────────────────────────────────────────────

pub mod notice {
    use super::*;

    thread_local! {
        /// `None` until the config answered; unknown counts as seen so the
        /// dialog never flashes on a slow start.
        static SEEN: Cell<Option<bool>> = const { Cell::new(None) };
    }

    /// Read `media_notice_seen` once. Idempotent - every entry point asks.
    pub fn load() {
        if SEEN.with(|s| s.get().is_some()) {
            return;
        }
        let core = app::core();
        app::spawn(async move { games::get_config(core.state(), "media_notice_seen".into()).await }, |res| {
            let seen = match res {
                Ok(v) => v.as_deref() == Some("1"),
                Err(e) => {
                    log::warn!("could not read media_notice_seen: {e}");
                    true
                }
            };
            SEEN.with(|s| s.set(Some(seen)));
        });
    }

    /// Offline nothing is transferred and no torrent is joined, so there is
    /// nothing to announce.
    pub fn needs() -> bool {
        !SEEN.with(|s| s.get().unwrap_or(true)) && !bus::offline()
    }

    fn accept() {
        SEEN.with(|s| s.set(Some(true)));
        let core = app::core();
        app::spawn(async move { games::set_config(core.clone(), core.state(), "media_notice_seen".into(), "1".into()).await }, |res| {
            if let Err(e) = res {
                log::error!("could not store media_notice_seen: {e}");
            }
        });
    }

    /// Information, not consent: the seeding decision itself lives in
    /// Settings → Network.
    pub fn show(parent: &impl IsA<gtk::Widget>, confirm_label: &str, on_confirm: impl FnOnce() + 'static) {
        let dialog = adw::AlertDialog::builder()
            .heading("Reading uses your connection")
            .body("Magazines, books and catalogs are downloaded one at a time - only the issue you open is transferred. While seeding is enabled, eXorchy also shares what it has downloaded. You can turn seeding off in Settings → Network.")
            .build();
        dialog.add_responses(&[("cancel", "Not now"), ("ok", confirm_label)]);
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");
        let on_confirm = Cell::new(Some(on_confirm));
        dialog.connect_response(None, move |_, r| {
            if r == "ok" {
                accept();
                if let Some(f) = on_confirm.take() {
                    f();
                }
            }
        });
        dialog.present(Some(parent));
    }
}

// ── The reader ───────────────────────────────────────────────────────────────

pub struct Reader {
    pub issue: Issue,
    start_page: Option<i64>,
    on_close: Rc<dyn Fn()>,
    stack: gtk::Stack,
    headline: gtk::Label,
    detail: gtk::Label,
    progress_row: gtk::Box,
    progress: gtk::ProgressBar,
    pct: gtk::Label,
    cancel_btn: gtk::Button,
    retry_btn: gtk::Button,
    close_btn: gtk::Button,
    view: RefCell<Option<Rc<DocumentView>>>,
    listener: Cell<Option<u64>>,
}

fn subtitle_of(issue: &Issue) -> String {
    let mut parts = vec![issue.publication.clone()];
    if let Some(y) = issue.year {
        parts.push(y.to_string());
    }
    parts.push(kind_label(issue));
    parts.push(format_bytes(issue.size_bytes.max(0) as u64));
    parts.join(" · ")
}

/// The reader for `issue`: fetches it if needed, then shows it. An article
/// link names a `start_page`; otherwise the last page read. `on_close` runs
/// on its close button (and Esc, which the room routes here).
pub fn build(issue: Issue, start_page: Option<i64>, on_close: Rc<dyn Fn()>) -> Rc<Reader> {
    // The fetch panel: header like the viewer's, then cover and state.
    let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["doc-viewer"]).hexpand(true).vexpand(true).build();
    let header = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).css_classes(["doc-header"]).build();
    let titles = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(1).hexpand(true).valign(gtk::Align::Center).build();
    titles.append(&gtk::Label::builder().label(&issue.title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["doc-title"]).build());
    titles.append(&gtk::Label::builder().label(subtitle_of(&issue)).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["muted", "small"]).build());
    header.append(&titles);
    let header_close = gtk::Button::builder().icon_name("window-close-symbolic").css_classes(["btn", "icon", "ghost"]).tooltip_text("Close (Esc)").build();
    header.append(&header_close);
    root.append(&header);

    let panel = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(24).halign(gtk::Align::Center).valign(gtk::Align::Center).vexpand(true).css_classes(["issue-fetch"]).build();
    let cover = gtk::Picture::builder().content_fit(gtk::ContentFit::Contain).width_request(180).height_request(240).can_shrink(true).css_classes(["issue-fetch-cover"]).build();
    panel.append(&cover);
    let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).valign(gtk::Align::Center).width_request(360).build();
    let headline = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["title-2"]).build();
    let detail = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["secondary"]).build();
    let progress_row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).build();
    let progress = gtk::ProgressBar::builder().hexpand(true).valign(gtk::Align::Center).build();
    let pct = gtk::Label::builder().width_chars(4).xalign(1.0).css_classes(["muted", "small"]).build();
    progress_row.append(&progress);
    progress_row.append(&pct);
    let actions = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).margin_top(6).build();
    let cancel_btn = gtk::Button::builder().label("Cancel").css_classes(["btn"]).build();
    let retry_btn = gtk::Button::builder().label("Try again").css_classes(["btn", "primary"]).build();
    let close_btn = gtk::Button::builder().label("Close").css_classes(["btn"]).build();
    actions.append(&retry_btn);
    actions.append(&cancel_btn);
    actions.append(&close_btn);
    body.append(&headline);
    body.append(&detail);
    body.append(&progress_row);
    body.append(&actions);
    panel.append(&body);
    root.append(&panel);

    let stack = gtk::Stack::builder().vexpand(true).hexpand(true).transition_type(gtk::StackTransitionType::Crossfade).build();
    stack.add_named(&root, Some("fetch"));

    let reader = Rc::new(Reader {
        issue: issue.clone(),
        start_page,
        on_close,
        stack: stack.clone(),
        headline,
        detail,
        progress_row,
        progress,
        pct,
        cancel_btn,
        retry_btn,
        close_btn,
        view: RefCell::new(None),
        listener: Cell::new(None),
    });

    let cover_weak = cover.downgrade();
    covers::request(Some(covers::MEDIA_SOURCE), issue.cover_key.as_deref(), covers::Size::Fit(360, 480), move |t| {
        if let (Some(c), Some(t)) = (cover_weak.upgrade(), t) {
            c.set_paintable(Some(&t));
        }
    });

    header_close.connect_clicked(glib::clone!(#[weak] reader, move |_| reader.close()));
    reader.cancel_btn.connect_clicked(glib::clone!(#[weak] reader, move |_| reader.close()));
    reader.close_btn.connect_clicked(glib::clone!(#[weak] reader, move |_| reader.close()));
    reader.retry_btn.connect_clicked(glib::clone!(#[weak] reader, move |_| store::request_issue(&reader.issue.key)));

    let key = issue.key.clone();
    let id = store::on_change(glib::clone!(#[weak] reader, move |change| {
        if matches!(change, Change::Issue(k) if *k == key) {
            reader.update();
        }
    }));
    reader.listener.set(Some(id));

    reader.update();
    store::request_issue(&issue.key);
    reader
}

impl Reader {
    pub fn widget(&self) -> gtk::Widget {
        self.stack.clone().upcast()
    }

    fn close(&self) {
        (self.on_close)();
    }

    /// The room took the reader down: stop listening, and abandon a fetch
    /// still running - nobody is reading.
    pub fn shutdown(&self) {
        if let Some(id) = self.listener.take() {
            store::remove_listener(id);
        }
        if store::is_busy(&self.issue.key) {
            store::abort(&self.issue.key);
        }
    }

    /// Paint the fetch panel from the store, or mount the viewer once the
    /// document is here.
    fn update(self: &Rc<Self>) {
        if self.view.borrow().is_some() {
            return;
        }
        let status = store::status(&self.issue.key);
        if let Some(path) = status.as_ref().filter(|s| s.phase == "ready").and_then(|s| s.path.clone()) {
            self.mount(&path);
            return;
        }
        // An offline request is provisional and never kept, so "no status"
        // while offline is the offline answer, not "preparing".
        let phase: Option<&str> = match &status {
            Some(s) => Some(s.phase.as_str()),
            None if bus::offline() => Some("none"),
            None => None,
        };
        let in_flight = matches!(phase, Some("fetching") | Some(PHASE_QUEUED));
        let failed = matches!(phase, Some("error") | Some("none"));
        let total = status.as_ref().map(|s| s.total_bytes).filter(|t| *t > 0).unwrap_or(self.issue.size_bytes.max(0) as u64);
        let progress = status.as_ref().map(|s| s.progress).unwrap_or(0.0);
        let headline = match phase {
            None => "Preparing…",
            Some(PHASE_QUEUED) => "Starting…",
            Some("fetching") => "Downloading…",
            Some("ready") => "Opening…",
            Some("error") => "Couldn't fetch this issue",
            _ => {
                if bus::offline() {
                    "Offline"
                } else {
                    "Not available right now"
                }
            }
        };
        let detail = match phase {
            Some(PHASE_QUEUED) => "Contacting the swarm.".to_string(),
            Some("fetching") => {
                if progress > 0.0 {
                    format!("{} of {}", format_bytes((progress * total as f64) as u64), format_bytes(total))
                } else {
                    format!("Starting · {}", format_bytes(total))
                }
            }
            Some("error") => {
                if let Some(raw) = status.as_ref().and_then(|s| s.error.as_deref()) {
                    log::error!("Reading Room: fetch failed: {raw}");
                }
                failure_detail(status.as_ref().and_then(|s| s.error.as_deref())).to_string()
            }
            Some("none") => {
                if bus::offline() {
                    "eXorchy is offline. Switch to online in Settings → Network to read this.".to_string()
                } else {
                    "This issue could not be fetched. Try again in a moment.".to_string()
                }
            }
            _ => String::new(),
        };
        self.headline.set_label(headline);
        self.detail.set_label(&detail);
        self.detail.set_visible(!detail.is_empty());
        self.progress_row.set_visible(in_flight);
        if in_flight {
            self.progress.set_fraction(progress.clamp(0.0, 1.0));
            if phase == Some(PHASE_QUEUED) {
                self.progress.pulse();
            }
            self.pct.set_label(&if phase == Some("fetching") && progress > 0.0 { format!("{}%", (progress * 100.0).round() as i32) } else { String::new() });
        }
        self.cancel_btn.set_visible(in_flight);
        self.retry_btn.set_visible(failed && !bus::offline());
        self.close_btn.set_visible(failed);
    }

    fn mount(self: &Rc<Self>, path: &str) {
        let issue = &self.issue;
        let start = self.start_page.or(issue.last_page).unwrap_or(1).max(1) as u32;
        let key = issue.key.clone();
        let on_page: Box<dyn Fn(u32)> = Box::new(move |p| store::remember_page(&key, p as i64));
        let view = DocumentView::new(path, &issue.title, Some(&subtitle_of(issue)), Some(on_page), start);
        // Only for a document that is here - there is nothing to give back
        // otherwise, and the reader is closed after it: its source is gone.
        let remove = gtk::Button::builder().label("Remove from disk").css_classes(["btn", "small", "danger"]).tooltip_text("Delete the downloaded file; the issue stays in the catalogue").build();
        remove.connect_clicked(glib::clone!(#[weak(rename_to = reader)] self, move |b| {
            let issue = reader.issue.clone();
            dialogs::confirm(
                b,
                "Remove from disk",
                &format!("Delete the downloaded copy of \"{}\"? You can download it again whenever you like.", issue.title),
                "Remove",
                true,
                glib::clone!(#[weak] reader, move || {
                    let title = issue.title.clone();
                    store::remove(&issue, move |res| {
                        if let Err(e) = res {
                            bus::toast_with(&format!("Couldn't remove {title}"), Some(&e), None);
                        }
                    });
                    reader.close();
                }),
            );
        }));
        view.actions.append(&remove);
        view.actions.append(&fullscreen_toggle());
        view.connect_close(glib::clone!(#[weak(rename_to = reader)] self, move || reader.close()));
        self.stack.add_named(&view.widget, Some("doc"));
        self.stack.set_visible_child_name("doc");
        self.view.replace(Some(view));
    }
}

/// Full screen for reading: the window goes full screen and the app's
/// chrome steps aside (`LibraryPage::set_reading_fullscreen`). The toggle
/// follows the window, which can leave full screen on its own.
fn fullscreen_toggle() -> gtk::ToggleButton {
    let b = gtk::ToggleButton::builder().icon_name("view-fullscreen-symbolic").css_classes(["btn", "icon"]).tooltip_text("Full screen (F11)").build();
    b.connect_toggled(|b| {
        if let Some(lib) = crate::ui::window::library() {
            lib.set_reading_fullscreen(b.is_active());
        }
    });
    let watching = Cell::new(false);
    b.connect_map(move |b| {
        let Some(window) = b.root().and_downcast::<gtk::Window>() else { return };
        b.set_active(window.is_fullscreen());
        if watching.replace(true) {
            return;
        }
        let weak = b.downgrade();
        window.connect_fullscreened_notify(move |w| {
            if let Some(b) = weak.upgrade() {
                if b.is_active() != w.is_fullscreen() {
                    b.set_active(w.is_fullscreen());
                }
            }
        });
    });
    b
}
