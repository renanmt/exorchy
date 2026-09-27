//! One issue in the grid (`IssueCard`, the game card's look) or in the list
//! (`IssueRow`): cover, title, year and publication, the badges (disk
//! magazine, DE, kind, on disk), the one-line action state, the size, the
//! favourite star and the fetch overlay with its cancel button.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use exorchy_core::models::Issue;
use gtk::glib;
use gtk::prelude::*;

use super::logic::{display_title, kind_label};
use super::store::{self, PHASE_QUEUED};
use crate::ui::card::{ART_HEIGHT, CARD_WIDTH};
use crate::ui::util::format_bytes;
use crate::ui::{bus, covers};

/// Right click at (x, y) on a card's widget.
pub type MenuCb = Box<dyn Fn(&Issue, &gtk::Widget, f64, f64)>;

/// What the room does with a card's clicks.
pub struct Ctx {
    pub on_open: Box<dyn Fn(&Issue)>,
    pub on_play: Box<dyn Fn(&Issue)>,
    pub on_menu: MenuCb,
}

/// What the surrounding view already shows, so the card does not repeat it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Flags {
    /// The publication is in view (section or filter), so titles drop it.
    pub publication_in_view: bool,
    pub show_kind: bool,
    /// Both languages are in view, so a German issue says so.
    pub show_language: bool,
}

/// The one-line state under a title, in the grid's action-label vocabulary.
pub fn action_of(issue: &Issue) -> (&'static str, String) {
    if let Some(state) = store::status(&issue.key) {
        if state.phase == PHASE_QUEUED {
            return ("action-downloading", "Queued".into());
        }
        if state.phase == "fetching" {
            return (
                "action-downloading",
                if state.progress > 0.0 { format!("{}%", (state.progress * 100.0).round() as i32) } else { "Fetching…".into() },
            );
        }
    }
    if issue.runnable {
        if issue.installed {
            return ("action-installed", "▶ Play".into());
        }
        return if bus::offline() { ("action-offline", "Not installed".into()) } else { ("action-download", "↓ Download".into()) };
    }
    let here = store::is_on_disk(&issue.key);
    if let Some(p) = issue.last_page.filter(|p| *p > 1) {
        return (if here { "action-installed" } else { "action-resume" }, format!("Continue · p. {p}"));
    }
    if here {
        return ("action-installed", "Read".into());
    }
    if bus::offline() {
        return ("action-offline", "Offline".into());
    }
    ("action-download", "↓ Read".into())
}

const ACTION_CLASSES: [&str; 6] =
    ["action-downloading", "action-installed", "action-resume", "action-offline", "action-download", "action-incomplete"];

fn set_action(label: &gtk::Label, cls: &str, text: &str) {
    label.set_label(text);
    for c in ACTION_CLASSES {
        label.remove_css_class(c);
    }
    label.add_css_class(cls);
}

fn badge(text: &str, cls: &str) -> gtk::Label {
    gtk::Label::builder().label(text).css_classes(["badge", cls]).build()
}

fn fav_tooltip(issue: &Issue) -> &'static str {
    if issue.favorited {
        "Remove from favorites"
    } else {
        "Add to favorites"
    }
}

/// A 400-page colour scan really is 200 MB, so the number gets a sentence.
fn size_tooltip(issue: &Issue) -> String {
    format!("{} — downloaded once, then kept until you remove it", format_bytes(issue.size_bytes.max(0) as u64))
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn set_favorited(btn: &gtk::Button, issue: &Issue) {
    if issue.favorited {
        btn.add_css_class("is-favorited");
    } else {
        btn.remove_css_class("is-favorited");
    }
    btn.set_tooltip_text(Some(fav_tooltip(issue)));
}

fn fetch_state(issue: &Issue) -> (bool, f64, bool) {
    match store::status(&issue.key) {
        Some(s) if s.phase == "fetching" => (true, s.progress, false),
        Some(s) if s.phase == PHASE_QUEUED => (true, 0.0, true),
        _ => (false, 0.0, false),
    }
}

// ── Grid card ────────────────────────────────────────────────────────────────

pub struct IssueCard {
    pub widget: gtk::Box,
    issue: RefCell<Issue>,
    flags: Cell<Flags>,
    generation: Cell<u64>,
    cover: gtk::Picture,
    cover_empty: gtk::Label,
    title: gtk::Label,
    meta: gtk::Label,
    badges: gtk::Box,
    action: gtk::Label,
    size: gtk::Label,
    fav: gtk::Button,
    dl_overlay: gtk::Box,
    dl_bar: gtk::ProgressBar,
    dl_pct: gtk::Label,
}

impl IssueCard {
    pub fn new(issue: Issue, flags: Flags, ctx: Rc<Ctx>) -> Rc<Self> {
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["game-card", "issue-card"])
            .width_request(CARD_WIDTH)
            .margin_start(5)
            .margin_end(5)
            .margin_top(5)
            .margin_bottom(5)
            .halign(gtk::Align::Center)
            .overflow(gtk::Overflow::Hidden)
            .build();
        let cover = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .width_request(CARD_WIDTH)
            .height_request(ART_HEIGHT)
            .can_shrink(true)
            .css_classes(["game-card-art"])
            .build();
        let cover_empty = gtk::Label::builder()
            .css_classes(["game-card-art-empty"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .max_width_chars(12)
            .visible(false)
            .build();
        let dl_bar = gtk::ProgressBar::builder().hexpand(true).valign(gtk::Align::Center).build();
        let dl_pct = gtk::Label::builder().css_classes(["game-card-pct"]).max_width_chars(6).build();
        let dl_cancel = gtk::Button::builder().icon_name("window-close-symbolic").css_classes(["btn", "icon", "ghost"]).tooltip_text("Cancel download").build();
        let dl_overlay = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(["game-card-download"]).visible(false).build();
        let dl_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        dl_row.set_valign(gtk::Align::Center);
        dl_row.set_vexpand(true);
        dl_row.set_margin_start(10);
        dl_row.set_margin_end(10);
        dl_row.append(&dl_bar);
        dl_row.append(&dl_cancel);
        dl_overlay.append(&dl_row);
        dl_overlay.append(&dl_pct);
        let fav = gtk::Button::builder().label("★").css_classes(["fav-btn"]).halign(gtk::Align::End).valign(gtk::Align::Start).build();
        let art = gtk::Overlay::new();
        art.set_child(Some(&cover));
        art.add_overlay(&cover_empty);
        art.add_overlay(&dl_overlay);
        art.add_overlay(&fav);
        widget.append(&art);

        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(3).css_classes(["game-card-body"]).build();
        let title = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).lines(2).wrap(true).max_width_chars(12).css_classes(["game-card-title"]).build();
        let meta = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(12).css_classes(["game-card-meta"]).build();
        let badges = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(4).build();
        let action_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let action = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).hexpand(true).max_width_chars(10).css_classes(["game-card-action"]).build();
        let size = gtk::Label::builder().xalign(1.0).css_classes(["muted", "tiny", "issue-size"]).build();
        action_row.append(&action);
        action_row.append(&size);
        body.append(&title);
        body.append(&meta);
        body.append(&badges);
        body.append(&action_row);
        widget.append(&body);

        let card = Rc::new(IssueCard {
            widget,
            issue: RefCell::new(issue),
            flags: Cell::new(flags),
            generation: Cell::new(0),
            cover,
            cover_empty,
            title,
            meta,
            badges,
            action,
            size,
            fav,
            dl_overlay,
            dl_bar,
            dl_pct,
        });

        let click = gtk::GestureClick::builder().button(1).build();
        let c = ctx.clone();
        click.connect_released(glib::clone!(#[weak] card, move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            let issue = card.issue.borrow().clone();
            if issue.runnable {
                (c.on_play)(&issue);
            } else {
                (c.on_open)(&issue);
            }
        }));
        card.widget.add_controller(click);
        let right = gtk::GestureClick::builder().button(3).build();
        let c = ctx.clone();
        right.connect_pressed(glib::clone!(#[weak] card, move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            let issue = card.issue.borrow().clone();
            (c.on_menu)(&issue, card.widget.upcast_ref(), x, y);
        }));
        card.widget.add_controller(right);
        card.fav.connect_clicked(glib::clone!(#[weak] card, move |_| {
            let issue = card.issue.borrow().clone();
            store::toggle_favorite(&issue);
        }));
        dl_cancel.connect_clicked(glib::clone!(#[weak] card, move |_| {
            let key = card.issue.borrow().key.clone();
            store::abort(&key);
        }));

        card.paint();
        card.load_cover();
        card
    }

    pub fn key(&self) -> String {
        self.issue.borrow().key.clone()
    }

    /// A fresh row for the same issue (status, favourite, installed).
    pub fn set_issue(&self, issue: Issue) {
        let same_cover = self.issue.borrow().cover_key == issue.cover_key;
        self.issue.replace(issue);
        self.paint();
        if !same_cover {
            self.load_cover_weak();
        }
    }

    fn paint(&self) {
        let issue = self.issue.borrow();
        let flags = self.flags.get();
        self.title.set_label(&display_title(&issue, flags.publication_in_view));
        self.title.set_tooltip_text(Some(&issue.title));
        let mut meta = Vec::new();
        if let Some(y) = issue.year {
            meta.push(y.to_string());
        }
        if !flags.publication_in_view {
            meta.push(issue.publication.clone());
        }
        self.meta.set_label(&meta.join(" · "));
        self.meta.set_visible(!meta.is_empty());

        clear(&self.badges);
        if issue.runnable {
            self.badges.append(&badge("Disk mag", "badge-disk"));
        }
        if flags.show_language && issue.language == "DE" {
            self.badges.append(&badge("DE", "badge-lang"));
        }
        if flags.show_kind && issue.kind != "magazine" {
            self.badges.append(&badge(&issue.kind, "badge-platform"));
        }
        if !issue.runnable && store::is_on_disk(&issue.key) {
            let b = badge("On disk", "badge-ondisk");
            b.set_tooltip_text(Some("Downloaded - opens without waiting"));
            self.badges.append(&b);
        }
        self.badges.set_visible(self.badges.first_child().is_some());

        let (cls, text) = action_of(&issue);
        set_action(&self.action, cls, &text);
        self.action.set_tooltip_text(if issue.runnable {
            Some(if issue.installed { "Run this disk magazine in DOSBox" } else { "Download this disk magazine" })
        } else {
            None
        });
        self.size.set_label(&format_bytes(issue.size_bytes.max(0) as u64));
        self.size.set_tooltip_text(Some(&size_tooltip(&issue)));
        set_favorited(&self.fav, &issue);
        if issue.installed || (!issue.runnable && store::is_on_disk(&issue.key)) {
            self.widget.add_css_class("installed");
        } else {
            self.widget.remove_css_class("installed");
        }

        let (busy, progress, queued) = fetch_state(&issue);
        self.dl_overlay.set_visible(busy);
        if busy {
            self.dl_bar.set_fraction(progress.clamp(0.0, 1.0));
            if queued {
                self.dl_bar.pulse();
            }
            self.dl_pct.set_label(&if progress > 0.0 { format!("{:.0}%", progress * 100.0) } else { "…".into() });
        }
    }

    fn load_cover_weak(&self) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let (key, title) = {
            let i = self.issue.borrow();
            (i.cover_key.clone(), i.title.clone())
        };
        self.cover.set_paintable(gtk::gdk::Paintable::NONE);
        self.cover_empty.set_visible(false);
        let cover = self.cover.downgrade();
        let empty = self.cover_empty.downgrade();
        covers::request(Some(covers::MEDIA_SOURCE), key.as_deref(), covers::Size::Fill(CARD_WIDTH as u32, ART_HEIGHT as u32), move |texture| {
            let (Some(cover), Some(empty)) = (cover.upgrade(), empty.upgrade()) else { return };
            match texture {
                Some(t) => cover.set_paintable(Some(&t)),
                None => {
                    empty.set_label(&title);
                    empty.set_visible(true);
                }
            }
        });
    }

    fn load_cover(self: &Rc<Self>) {
        self.load_cover_weak();
    }
}

// ── List row ─────────────────────────────────────────────────────────────────

/// Column widths shared by the header and the rows.
pub const ROW_FAV: i32 = 28;
pub const ROW_COVER: i32 = 36;
pub const ROW_PUB: i32 = 220;
pub const ROW_YEAR: i32 = 56;
pub const ROW_KIND: i32 = 110;
pub const ROW_SIZE: i32 = 80;
pub const ROW_STATUS: i32 = 150;

pub struct IssueRow {
    pub widget: gtk::Box,
    issue: RefCell<Issue>,
    flags: Cell<Flags>,
    generation: Rc<Cell<u64>>,
    fav: gtk::Button,
    cover: gtk::Picture,
    title: gtk::Label,
    title_badges: gtk::Box,
    publication: gtk::Label,
    year: gtk::Label,
    kind: gtk::Label,
    size: gtk::Label,
    status: gtk::Label,
    cancel: gtk::Button,
}

impl IssueRow {
    pub fn new(ctx: Rc<Ctx>) -> Rc<Self> {
        let widget = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(10).css_classes(["game-row", "issue-row"]).build();
        let fav = gtk::Button::builder().label("★").css_classes(["fav-btn", "row-fav"]).valign(gtk::Align::Center).width_request(ROW_FAV).build();
        let cover = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).width_request(ROW_COVER).height_request(48).can_shrink(true).css_classes(["row-cover"]).build();
        let title_box = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(6).hexpand(true).valign(gtk::Align::Center).build();
        let title = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(40).css_classes(["row-title"]).build();
        let title_badges = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(4).valign(gtk::Align::Center).build();
        title_box.append(&title);
        title_box.append(&title_badges);
        let publication = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).width_request(ROW_PUB).max_width_chars(24).css_classes(["secondary"]).build();
        let year = gtk::Label::builder().xalign(0.0).width_request(ROW_YEAR).css_classes(["secondary"]).build();
        let kind = gtk::Label::builder().xalign(0.0).width_request(ROW_KIND).css_classes(["secondary"]).build();
        let size = gtk::Label::builder().xalign(0.0).width_request(ROW_SIZE).css_classes(["secondary"]).build();
        // hexpand propagates up from children, so the status label must not
        // expand or the row's spare width is split with the title column.
        let status_box = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(4).width_request(ROW_STATUS).hexpand(false).build();
        let status = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(14).css_classes(["game-card-action"]).build();
        let cancel = gtk::Button::builder().icon_name("window-close-symbolic").css_classes(["btn", "icon", "ghost"]).tooltip_text("Cancel download").visible(false).build();
        status_box.append(&status);
        status_box.append(&cancel);
        widget.append(&fav);
        widget.append(&cover);
        widget.append(&title_box);
        widget.append(&publication);
        widget.append(&year);
        widget.append(&kind);
        widget.append(&size);
        widget.append(&status_box);

        let row = Rc::new(IssueRow {
            widget,
            issue: RefCell::new(placeholder_issue()),
            flags: Cell::new(Flags::default()),
            generation: Rc::new(Cell::new(0)),
            fav,
            cover,
            title,
            title_badges,
            publication,
            year,
            kind,
            size,
            status,
            cancel,
        });
        let click = gtk::GestureClick::builder().button(1).build();
        let c = ctx.clone();
        click.connect_released(glib::clone!(#[weak] row, move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            let issue = row.issue.borrow().clone();
            if issue.runnable {
                (c.on_play)(&issue);
            } else {
                (c.on_open)(&issue);
            }
        }));
        row.widget.add_controller(click);
        let right = gtk::GestureClick::builder().button(3).build();
        let c = ctx.clone();
        right.connect_pressed(glib::clone!(#[weak] row, move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            let issue = row.issue.borrow().clone();
            (c.on_menu)(&issue, row.widget.upcast_ref(), x, y);
        }));
        row.widget.add_controller(right);
        row.fav.connect_clicked(glib::clone!(#[weak] row, move |_| {
            let issue = row.issue.borrow().clone();
            store::toggle_favorite(&issue);
        }));
        row.cancel.connect_clicked(glib::clone!(#[weak] row, move |_| {
            let key = row.issue.borrow().key.clone();
            store::abort(&key);
        }));
        row
    }

    pub fn key(&self) -> String {
        self.issue.borrow().key.clone()
    }

    pub fn bind(&self, issue: Issue, flags: Flags) {
        let same_cover = {
            let old = self.issue.borrow();
            old.cover_key == issue.cover_key && !old.key.is_empty()
        };
        self.issue.replace(issue);
        self.flags.set(flags);
        self.paint();
        if !same_cover {
            self.load_cover();
        }
    }

    pub fn set_issue(&self, issue: Issue) {
        self.bind(issue, self.flags.get());
    }

    fn paint(&self) {
        let issue = self.issue.borrow();
        let flags = self.flags.get();
        self.title.set_label(&display_title(&issue, flags.publication_in_view));
        self.title.set_tooltip_text(Some(&issue.title));
        clear(&self.title_badges);
        if issue.runnable {
            self.title_badges.append(&badge("Disk", "badge-disk"));
        }
        if flags.show_language && issue.language == "DE" {
            self.title_badges.append(&badge("DE", "badge-lang"));
        }
        self.publication.set_label(&issue.publication);
        self.publication.set_tooltip_text(Some(&issue.publication));
        self.year.set_label(&issue.year.map(|y| y.to_string()).unwrap_or_default());
        self.kind.set_label(&kind_label(&issue));
        self.size.set_label(&format_bytes(issue.size_bytes.max(0) as u64));
        self.size.set_tooltip_text(Some(&size_tooltip(&issue)));
        let (cls, text) = action_of(&issue);
        set_action(&self.status, cls, &text);
        let (busy, _, _) = fetch_state(&issue);
        self.cancel.set_visible(busy);
        set_favorited(&self.fav, &issue);
        if issue.installed {
            self.widget.add_css_class("installed");
        } else {
            self.widget.remove_css_class("installed");
        }
    }

    /// The row is recycled, so a late texture checks the generation it was
    /// requested under.
    fn load_cover(&self) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let key = self.issue.borrow().cover_key.clone();
        self.cover.set_paintable(gtk::gdk::Paintable::NONE);
        let cover = self.cover.downgrade();
        let gen_cell = self.generation_weak();
        covers::request(Some(covers::MEDIA_SOURCE), key.as_deref(), covers::Size::Fill(ROW_COVER as u32, 48), move |texture| {
            let Some(cover) = cover.upgrade() else { return };
            if gen_cell.get() != generation {
                return;
            }
            if let Some(t) = texture {
                cover.set_paintable(Some(&t));
            }
        });
    }

    fn generation_weak(&self) -> Rc<Cell<u64>> {
        self.generation.clone()
    }
}

/// An empty row before the first bind (a `ListView` row is built once).
fn placeholder_issue() -> Issue {
    Issue {
        id: 0,
        key: String::new(),
        publication_id: 0,
        publication: String::new(),
        kind: String::new(),
        title: String::new(),
        sort_title: None,
        year: None,
        release_date: None,
        publisher: None,
        developer: None,
        notes: None,
        zip_file: String::new(),
        entry_path: None,
        entry_kind: None,
        size_bytes: 0,
        cover_key: None,
        runnable: false,
        launch_dir: None,
        issue_dir: None,
        launch_bat: None,
        command_line: None,
        substitutions: None,
        favorited: false,
        installed: false,
        last_page: None,
        last_opened: None,
        source: String::new(),
        inner_zip: None,
        language: String::new(),
        extras_count: 0,
    }
}
