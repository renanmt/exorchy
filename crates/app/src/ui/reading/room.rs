//! The Reading Room tab: kind and language chips, the publication menu,
//! sort, search, the sectioned grid (a `ListView` of sections, each a
//! `FlowBox` of cards, so only the sections in view are built) or the
//! sortable list, the jump bar, the offline and poster-pack notices, the
//! right-click "Remove from disk", and the entry points to the reader and
//! to DOSBox for disk magazines.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use exorchy_core::commands::{assets, content_packs, games};
use exorchy_core::models::Issue;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::card::{self, Ctx, Flags, IssueCard, IssueRow};
use super::logic::{self, Column, Filter, Kind, Language, Sort, GRID_SORTS, LIST_COLUMNS};
use super::reader::{self, notice};
use super::store::{self, Change};
use crate::app;
use crate::ui::util::format_bytes;
use crate::ui::{bus, covers, dialogs};

// ── Models ───────────────────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct IssueObject {
        pub issue: RefCell<Option<Issue>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IssueObject {
        const NAME: &'static str = "ExorchyIssueObject";
        type Type = super::IssueObject;
    }

    impl ObjectImpl for IssueObject {}

    #[derive(Default)]
    pub struct SectionObject {
        pub label: RefCell<String>,
        pub issues: RefCell<Vec<Issue>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SectionObject {
        const NAME: &'static str = "ExorchyReadingSection";
        type Type = super::SectionObject;
    }

    impl ObjectImpl for SectionObject {}
}

glib::wrapper! {
    pub struct IssueObject(ObjectSubclass<imp::IssueObject>);
}

impl IssueObject {
    fn new(issue: Issue) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().issue.replace(Some(issue));
        obj
    }

    fn issue(&self) -> Issue {
        self.imp().issue.borrow().clone().expect("IssueObject always carries an issue")
    }

    fn set_issue(&self, issue: Issue) {
        self.imp().issue.replace(Some(issue));
    }
}

glib::wrapper! {
    pub struct SectionObject(ObjectSubclass<imp::SectionObject>);
}

impl SectionObject {
    fn new(label: String, issues: Vec<Issue>) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().label.replace(label);
        obj.imp().issues.replace(issues);
        obj
    }
}

thread_local! {
    /// Cards behind each bound section row, so a status tick can find the
    /// visible ones; rows behind each bound list row likewise.
    static CARDS: RefCell<HashMap<gtk::Widget, Vec<Rc<IssueCard>>>> = RefCell::new(HashMap::new());
    static ROWS: RefCell<HashMap<gtk::Widget, Rc<IssueRow>>> = RefCell::new(HashMap::new());
}

// ── The room ─────────────────────────────────────────────────────────────────

pub struct Room {
    pub widget: gtk::Box,
    window: gtk::Window,
    kind: Cell<Kind>,
    language: Cell<Language>,
    publication_id: Cell<Option<i64>>,
    favorites: Cell<bool>,
    query: RefCell<String>,
    sort: Cell<Sort>,
    grid_mode: Cell<bool>,
    kind_buttons: Vec<(Kind, gtk::ToggleButton)>,
    lang_buttons: Vec<(Language, gtk::ToggleButton)>,
    fav_button: gtk::ToggleButton,
    pub_button: gtk::MenuButton,
    pub_popover: gtk::Popover,
    pub_list: gtk::Box,
    sort_drop: gtk::DropDown,
    count: gtk::Label,
    search: gtk::SearchEntry,
    view_grid: gtk::ToggleButton,
    view_list: gtk::ToggleButton,
    offline_note: gtk::Label,
    pack_hint: gtk::Box,
    pack_hint_desc: gtk::Label,
    content: gtk::Stack,
    empty_text: gtk::Label,
    empty_btn: gtk::Button,
    error_label: gtk::Label,
    sections_store: gio::ListStore,
    sections_list: gtk::ListView,
    issues_store: gio::ListStore,
    list: gtk::ListView,
    list_header: gtk::Box,
    jump_bar: gtk::Box,
    jump_scroller: gtk::ScrolledWindow,
    flags: Cell<Flags>,
    ctx: Rc<Ctx>,
    refresh_pending: Cell<bool>,
    /// Bumped per sort change so a stale DropDown callback is ignored.
    syncing: Cell<bool>,
}

fn chip(label: &str) -> gtk::ToggleButton {
    gtk::ToggleButton::builder().label(label).css_classes(["chip"]).build()
}

impl Room {
    pub fn new(window: &gtk::Window) -> Rc<Self> {
        let widget = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).vexpand(true).css_classes(["reading-room"]).build();

        // ── Row 1: chips, search, view ──
        let row1 = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).css_classes(["filter-row"]).build();
        let kinds = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let mut kind_buttons = Vec::new();
        for k in Kind::ALL {
            let b = chip(k.label());
            kinds.append(&b);
            kind_buttons.push((k, b));
        }
        row1.append(&kinds);
        row1.append(&gtk::Separator::builder().orientation(gtk::Orientation::Vertical).margin_top(4).margin_bottom(4).build());
        let langs = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let mut lang_buttons = Vec::new();
        for l in Language::ALL {
            let b = chip(l.label());
            langs.append(&b);
            lang_buttons.push((l, b));
        }
        row1.append(&langs);
        row1.append(&gtk::Separator::builder().orientation(gtk::Orientation::Vertical).margin_top(4).margin_bottom(4).build());
        let fav_button = chip("★ Favorites");
        fav_button.set_tooltip_text(Some("Only favourited issues"));
        row1.append(&fav_button);
        let sp1 = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sp1.set_hexpand(true);
        row1.append(&sp1);
        let search = gtk::SearchEntry::builder().placeholder_text("Search issues…").css_classes(["search"]).build();
        row1.append(&search);
        let view_grid = gtk::ToggleButton::builder().icon_name("view-grid-symbolic").active(true).css_classes(["btn", "icon"]).tooltip_text("Grid").build();
        let view_list = gtk::ToggleButton::builder().icon_name("view-list-symbolic").group(&view_grid).css_classes(["btn", "icon"]).tooltip_text("List").build();
        let view_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        view_box.add_css_class("linked");
        view_box.append(&view_grid);
        view_box.append(&view_list);
        row1.append(&view_box);
        widget.append(&row1);

        // ── Row 2: publication, sort, count ──
        let row2 = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).css_classes(["filter-row", "reading-row2"]).build();
        let pub_list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(0).build();
        let pub_scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).max_content_height(460).propagate_natural_height(true).propagate_natural_width(true).child(&pub_list).build();
        let pub_popover = gtk::Popover::builder().child(&pub_scroller).css_classes(["context-menu", "pub-menu"]).has_arrow(false).build();
        let pub_button = gtk::MenuButton::builder().label("All publications").popover(&pub_popover).css_classes(["drop", "pub-button"]).build();
        row2.append(&pub_button);
        let sort_labels: Vec<&str> = GRID_SORTS.iter().map(|(_, l)| *l).collect();
        let sort_drop = gtk::DropDown::from_strings(&sort_labels);
        sort_drop.add_css_class("drop");
        row2.append(&sort_drop);
        let count = gtk::Label::builder().css_classes(["muted", "small", "results-count"]).build();
        row2.append(&count);
        let sp2 = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sp2.set_hexpand(true);
        row2.append(&sp2);
        let offline_note = gtk::Label::builder()
            .label("Offline - the catalogue is here to browse; opening an issue needs a connection.")
            .css_classes(["reading-note"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .visible(false)
            .build();
        row2.append(&offline_note);
        widget.append(&row2);

        // ── Poster pack hint ──
        let pack_hint = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).css_classes(["pack-hint"]).visible(false).build();
        let hint_text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
        hint_text.append(&gtk::Label::builder().label("Better covers available").xalign(0.0).css_classes(["title-3"]).build());
        let pack_hint_desc = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["secondary", "small"]).build();
        hint_text.append(&pack_hint_desc);
        pack_hint.append(&hint_text);
        let hint_download = gtk::Button::builder().label("Download").css_classes(["btn", "primary"]).valign(gtk::Align::Center).build();
        let hint_dismiss = gtk::Button::builder().label("Not now").css_classes(["btn", "ghost"]).valign(gtk::Align::Center).build();
        pack_hint.append(&hint_download);
        pack_hint.append(&hint_dismiss);
        widget.append(&pack_hint);

        // ── Content ──
        let content = gtk::Stack::builder().vexpand(true).hexpand(true).build();
        let loading = gtk::Label::builder().label("Loading the reading room…").css_classes(["muted"]).vexpand(true).build();
        content.add_named(&loading, Some("loading"));
        let error_label = gtk::Label::builder().wrap(true).css_classes(["danger"]).vexpand(true).build();
        content.add_named(&error_label, Some("error"));

        let empty = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).valign(gtk::Align::Center).halign(gtk::Align::Center).vexpand(true).css_classes(["lib-empty"]).build();
        let empty_icon = gtk::Image::builder().icon_name("x-office-document-symbolic").pixel_size(56).css_classes(["muted"]).margin_bottom(8).build();
        let empty_text = gtk::Label::builder().css_classes(["title-2"]).wrap(true).justify(gtk::Justification::Center).build();
        let empty_sub = gtk::Label::builder().label("Try a different search or another publication").css_classes(["muted"]).build();
        let empty_btn = gtk::Button::builder().label("Show everything").css_classes(["btn"]).halign(gtk::Align::Center).margin_top(10).visible(false).build();
        empty.append(&empty_icon);
        empty.append(&empty_text);
        empty.append(&empty_sub);
        empty.append(&empty_btn);
        content.add_named(&empty, Some("empty"));

        // Grid: one row per section.
        let sections_store = gio::ListStore::new::<SectionObject>();
        let sections_list = gtk::ListView::builder()
            .model(&gtk::NoSelection::new(Some(sections_store.clone())))
            .css_classes(["reading-sections"])
            .single_click_activate(false)
            .build();
        let grid_scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).hexpand(true).child(&sections_list).build();
        let jump_bar = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(0).css_classes(["jump-bar"]).valign(gtk::Align::Start).build();
        let jump_scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vscrollbar_policy(gtk::PolicyType::External).child(&jump_bar).visible(false).build();
        let grid_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        grid_row.append(&grid_scroller);
        grid_row.append(&jump_scroller);
        content.add_named(&grid_row, Some("grid"));

        // List: flat rows under a sortable header.
        let issues_store = gio::ListStore::new::<IssueObject>();
        let list = gtk::ListView::builder().model(&gtk::NoSelection::new(Some(issues_store.clone()))).css_classes(["game-list", "issue-list"]).build();
        let list_scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).hexpand(true).child(&list).build();
        let list_header = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(10).css_classes(["list-header", "issue-list-header"]).build();
        let list_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list_page.append(&list_header);
        list_page.append(&list_scroller);
        content.add_named(&list_page, Some("list"));
        widget.append(&content);

        let room = Rc::new_cyclic(|weak: &std::rc::Weak<Room>| {
            let (w1, w2, w3) = (weak.clone(), weak.clone(), weak.clone());
            let ctx = Rc::new(Ctx {
                on_open: Box::new(move |issue| {
                    if let Some(r) = w1.upgrade() {
                        r.open_issue(issue.clone());
                    }
                }),
                on_play: Box::new(move |issue| {
                    if let Some(r) = w2.upgrade() {
                        r.play_issue(issue.clone());
                    }
                }),
                on_menu: Box::new(move |issue, w, x, y| {
                    if let Some(r) = w3.upgrade() {
                        r.show_menu(issue.clone(), w, x, y);
                    }
                }),
            });
            Room {
                widget,
                window: window.clone(),
                kind: Cell::new(Kind::All),
                language: Cell::new(Language::All),
                publication_id: Cell::new(None),
                favorites: Cell::new(false),
                query: RefCell::new(String::new()),
                sort: Cell::new(Sort::Publication),
                grid_mode: Cell::new(true),
                kind_buttons,
                lang_buttons,
                fav_button,
                pub_button,
                pub_popover,
                pub_list,
                sort_drop,
                count,
                search,
                view_grid: view_grid.clone(),
                view_list: view_list.clone(),
                offline_note,
                pack_hint,
                pack_hint_desc,
                content,
                empty_text,
                empty_btn,
                error_label,
                sections_store,
                sections_list,
                issues_store,
                list,
                list_header,
                jump_bar,
                jump_scroller,
                flags: Cell::new(Flags::default()),
                ctx,
                refresh_pending: Cell::new(false),
                syncing: Cell::new(false),
            }
        });

        room.setup_factories();
        room.wire(view_grid, view_list, hint_download, hint_dismiss);
        room.sync_chips();
        room.build_list_header();
        room.load_pack_hint();
        notice::load();
        store::load_catalog(false);
        room
    }

    // ── Factories ──

    fn setup_factories(self: &Rc<Self>) {
        // Sections.
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(0).css_classes(["reading-section"]).build();
            let header = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).css_classes(["grid-separator"]).build();
            header.append(&gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["grid-separator-label"]).build());
            header.append(&gtk::Label::builder().css_classes(["section-count", "muted", "small"]).build());
            root.append(&header);
            let flow = gtk::FlowBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .homogeneous(true)
                .min_children_per_line(2)
                .max_children_per_line(14)
                .column_spacing(0)
                .row_spacing(0)
                .halign(gtk::Align::Start)
                .activate_on_single_click(false)
                .css_classes(["game-grid", "issue-flow"])
                .build();
            root.append(&flow);
            item.set_child(Some(&root));
            item.set_activatable(false);
            item.set_selectable(false);
        });
        factory.connect_bind(glib::clone!(#[weak(rename_to = room)] self, move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let Some(section) = item.item().and_downcast::<SectionObject>() else { return };
            let Some(root) = item.child().and_downcast::<gtk::Box>() else { return };
            let header = root.first_child().and_downcast::<gtk::Box>().expect("header");
            let flow = root.last_child().and_downcast::<gtk::FlowBox>().expect("flow");
            let label = section.imp().label.borrow().clone();
            let issues = section.imp().issues.borrow().clone();
            header.set_visible(!label.is_empty());
            if let Some(l) = header.first_child().and_downcast::<gtk::Label>() {
                l.set_label(&label);
            }
            if let Some(c) = header.last_child().and_downcast::<gtk::Label>() {
                c.set_label(&issues.len().to_string());
            }
            while let Some(c) = flow.first_child() {
                flow.remove(&c);
            }
            let flags = room.flags.get();
            let mut cards = Vec::with_capacity(issues.len());
            for issue in issues {
                let card = IssueCard::new(issue, flags, room.ctx.clone());
                flow.append(&card.widget);
                if let Some(child) = card.widget.parent().and_downcast::<gtk::FlowBoxChild>() {
                    child.set_focusable(false);
                }
                cards.push(card);
            }
            CARDS.with(|c| c.borrow_mut().insert(root.upcast(), cards));
        }));
        factory.connect_unbind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let Some(root) = item.child().and_downcast::<gtk::Box>() else { return };
            if let Some(flow) = root.last_child().and_downcast::<gtk::FlowBox>() {
                while let Some(c) = flow.first_child() {
                    flow.remove(&c);
                }
            }
            CARDS.with(|c| c.borrow_mut().remove(root.upcast_ref::<gtk::Widget>()));
        });
        self.sections_list.set_factory(Some(&factory));

        // List rows.
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(glib::clone!(#[weak(rename_to = room)] self, move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let row = IssueRow::new(room.ctx.clone());
            item.set_child(Some(&row.widget));
            item.set_activatable(false);
            item.set_selectable(false);
            ROWS.with(|r| r.borrow_mut().insert(row.widget.clone().upcast(), row));
        }));
        factory.connect_bind(glib::clone!(#[weak(rename_to = room)] self, move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let (Some(obj), Some(w)) = (item.item().and_downcast::<IssueObject>(), item.child()) else { return };
            if let Some(row) = ROWS.with(|r| r.borrow().get(&w).cloned()) {
                row.bind(obj.issue(), room.flags.get());
            }
        }));
        factory.connect_teardown(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            if let Some(w) = item.child() {
                ROWS.with(|r| r.borrow_mut().remove(&w));
            }
        });
        self.list.set_factory(Some(&factory));
    }

    // ── Wiring ──

    fn wire(self: &Rc<Self>, view_grid: gtk::ToggleButton, view_list: gtk::ToggleButton, hint_download: gtk::Button, hint_dismiss: gtk::Button) {
        for (k, b) in &self.kind_buttons {
            let k = *k;
            b.connect_toggled(glib::clone!(#[weak(rename_to = room)] self, move |b| {
                if b.is_active() && room.kind.get() != k {
                    room.kind.set(k);
                    room.after_chip_change();
                }
                room.sync_chips();
            }));
        }
        for (l, b) in &self.lang_buttons {
            let l = *l;
            b.connect_toggled(glib::clone!(#[weak(rename_to = room)] self, move |b| {
                if b.is_active() && room.language.get() != l {
                    room.language.set(l);
                    room.after_chip_change();
                }
                room.sync_chips();
            }));
        }
        self.fav_button.connect_toggled(glib::clone!(#[weak(rename_to = room)] self, move |b| {
            room.favorites.set(b.is_active());
            room.refresh();
        }));
        self.search.connect_search_changed(glib::clone!(#[weak(rename_to = room)] self, move |e| {
            room.query.replace(e.text().to_string());
            room.refresh();
        }));
        self.sort_drop.connect_selected_notify(glib::clone!(#[weak(rename_to = room)] self, move |d| {
            if room.syncing.get() {
                return;
            }
            if let Some((sort, _)) = GRID_SORTS.get(d.selected() as usize) {
                room.set_sort(*sort);
            }
        }));
        view_grid.connect_toggled(glib::clone!(#[weak(rename_to = room)] self, move |b| {
            if b.is_active() {
                room.switch_view(true);
            }
        }));
        view_list.connect_toggled(glib::clone!(#[weak(rename_to = room)] self, move |b| {
            if b.is_active() {
                room.switch_view(false);
            }
        }));
        self.empty_btn.connect_clicked(glib::clone!(#[weak(rename_to = room)] self, move |_| {
            room.kind.set(Kind::All);
            room.language.set(Language::All);
            room.publication_id.set(None);
            room.favorites.set(false);
            room.sync_chips();
            room.refresh();
        }));
        hint_download.connect_clicked(glib::clone!(#[weak(rename_to = room)] self, move |_| room.pack_hint_answer(true)));
        hint_dismiss.connect_clicked(glib::clone!(#[weak(rename_to = room)] self, move |_| room.pack_hint_answer(false)));

        store::on_change(glib::clone!(#[weak(rename_to = room)] self, move |change| match change {
            Change::Catalog => room.refresh(),
            Change::Issue(key) => room.issue_changed(key),
        }));
        covers::on_dirs_changed(glib::clone!(#[weak(rename_to = room)] self, move || {
            room.refresh();
            room.load_pack_hint();
        }));
        self.offline_note.set_visible(bus::offline());
    }

    fn sync_chips(&self) {
        for (k, b) in &self.kind_buttons {
            let on = *k == self.kind.get();
            if b.is_active() != on {
                b.set_active(on);
            }
        }
        for (l, b) in &self.lang_buttons {
            let on = *l == self.language.get();
            if b.is_active() != on {
                b.set_active(on);
            }
        }
        if self.fav_button.is_active() != self.favorites.get() {
            self.fav_button.set_active(self.favorites.get());
        }
    }

    /// A kind or language switch invalidates a publication picked under the
    /// old one.
    fn after_chip_change(self: &Rc<Self>) {
        if let Some(id) = self.publication_id.get() {
            if !logic::publication_still_in_view(&store::publications(), id, self.kind.get(), self.language.get()) {
                self.publication_id.set(None);
            }
        }
        self.refresh();
    }

    fn set_sort(self: &Rc<Self>, sort: Sort) {
        if self.sort.get() == sort {
            return;
        }
        self.sort.set(sort);
        self.sync_sort_drop();
        self.build_list_header();
        self.refresh();
    }

    fn sync_sort_drop(&self) {
        if let Some(i) = GRID_SORTS.iter().position(|(s, _)| *s == self.sort.get()) {
            self.syncing.set(true);
            self.sort_drop.set_selected(i as u32);
            self.syncing.set(false);
        }
    }

    /// Grid or list, as the toggle would.
    pub fn set_view(self: &Rc<Self>, grid: bool) {
        self.view_grid.set_active(grid);
        self.view_list.set_active(!grid);
    }

    /// Column sorts the grid's menu cannot show fall back to its default.
    fn switch_view(self: &Rc<Self>, grid: bool) {
        self.grid_mode.set(grid);
        self.sort_drop.set_visible(grid);
        if grid && !GRID_SORTS.iter().any(|(s, _)| *s == self.sort.get()) {
            self.sort.set(Sort::Publication);
            self.sync_sort_drop();
            self.build_list_header();
        }
        self.refresh();
    }

    fn build_list_header(self: &Rc<Self>) {
        while let Some(c) = self.list_header.first_child() {
            self.list_header.remove(&c);
        }
        let pad = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        pad.set_size_request(card::ROW_FAV + 10 + card::ROW_COVER, -1);
        self.list_header.append(&pad);
        let current = self.sort.get();
        for (i, col) in LIST_COLUMNS.iter().enumerate() {
            let b = gtk::Button::builder()
                .label(format!("{}{}", col.label, col.indicator(current)))
                .css_classes(["list-col"])
                .sensitive(col.asc.is_some())
                .build();
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            let width = match i {
                0 => -1,
                1 => card::ROW_PUB,
                2 => card::ROW_YEAR,
                3 => card::ROW_KIND,
                4 => card::ROW_SIZE,
                _ => card::ROW_STATUS,
            };
            if width > 0 {
                b.set_size_request(width, -1);
            } else {
                b.set_hexpand(true);
            }
            let col: &'static Column = col;
            b.connect_clicked(glib::clone!(#[weak(rename_to = room)] self, move |_| {
                if let Some(next) = col.next_sort(room.sort.get()) {
                    room.set_sort(next);
                }
            }));
            self.list_header.append(&b);
        }
    }

    // ── Rendering ──

    /// Coalesce bursts (a keystroke, a chip and its sync) into one pass.
    pub fn refresh(self: &Rc<Self>) {
        if self.refresh_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(room) = weak.upgrade() {
                room.refresh_pending.set(false);
                room.render();
            }
        });
    }

    fn render(self: &Rc<Self>) {
        self.offline_note.set_visible(bus::offline());
        self.rebuild_publication_menu();
        if !store::loaded() {
            self.content.set_visible_child_name("loading");
            return;
        }
        if let Some(e) = store::load_error() {
            self.error_label.set_label(&format!("The reading room could not be loaded.\n{e}"));
            self.content.set_visible_child_name("error");
            return;
        }
        let (kind, language, publication_id, favorites) = (self.kind.get(), self.language.get(), self.publication_id.get(), self.favorites.get());
        let query = self.query.borrow().clone();
        let sort = self.sort.get();
        let all = store::issues();
        let mut shown = logic::filter_issues(&all, &Filter { kind, language, publication_id, favorites, query: &query });
        logic::sort_issues(&mut shown, sort);
        self.count.set_label(&logic::results_label(shown.len(), kind));
        self.flags.set(Flags {
            publication_in_view: sort == Sort::Publication || publication_id.is_some(),
            show_kind: kind == Kind::All,
            show_language: language == Language::All,
        });

        if shown.is_empty() {
            let q = query.trim();
            self.empty_text.set_label(&if q.is_empty() { "No documents match these filters".to_string() } else { format!("No documents match \"{q}\"") });
            self.empty_btn.set_visible(kind != Kind::All || language != Language::All || publication_id.is_some() || favorites);
            self.content.set_visible_child_name("empty");
            self.jump_scroller.set_visible(false);
            return;
        }
        if self.grid_mode.get() {
            let sections = logic::sections(shown, sort);
            let labels: Vec<String> = sections.iter().map(|s| s.label.clone()).collect();
            let objects: Vec<SectionObject> = sections.into_iter().map(|s| SectionObject::new(s.label, s.issues)).collect();
            self.sections_store.splice(0, self.sections_store.n_items(), &objects);
            self.content.set_visible_child_name("grid");
            self.build_jump_bar(&labels);
        } else {
            let objects: Vec<IssueObject> = shown.into_iter().map(IssueObject::new).collect();
            self.issues_store.splice(0, self.issues_store.n_items(), &objects);
            self.content.set_visible_child_name("list");
            self.jump_scroller.set_visible(false);
        }
    }

    fn build_jump_bar(self: &Rc<Self>, labels: &[String]) {
        while let Some(c) = self.jump_bar.first_child() {
            self.jump_bar.remove(&c);
        }
        let show = labels.len() > 1 && !labels[0].is_empty();
        self.jump_scroller.set_visible(show);
        if !show {
            return;
        }
        let wide = logic::jump_bar_is_wide(labels);
        if wide {
            self.jump_bar.add_css_class("wide");
        } else {
            self.jump_bar.remove_css_class("wide");
        }
        for (i, label) in labels.iter().enumerate() {
            let b = gtk::Button::builder().label(label).css_classes(["jump-btn"]).tooltip_text(label).build();
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(if wide { 0.0 } else { 0.5 });
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                l.set_max_width_chars(if wide { 16 } else { 4 });
            }
            b.connect_clicked(glib::clone!(#[weak(rename_to = room)] self, move |_| {
                room.sections_list.scroll_to(i as u32, gtk::ListScrollFlags::NONE, None);
            }));
            self.jump_bar.append(&b);
        }
    }

    fn rebuild_publication_menu(self: &Rc<Self>) {
        while let Some(c) = self.pub_list.first_child() {
            self.pub_list.remove(&c);
        }
        let pubs = store::publications();
        let selected = self.publication_id.get();
        let mut trigger = "All publications".to_string();
        let add = |label: &str, id: Option<i64>, active: bool| {
            let b = gtk::Button::builder().label(label).css_classes(["menu-item"]).build();
            if active {
                b.add_css_class("active");
            }
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                l.set_max_width_chars(36);
            }
            b.connect_clicked(glib::clone!(#[weak(rename_to = room)] self, move |_| {
                room.pub_popover.popdown();
                room.publication_id.set(id);
                room.refresh();
            }));
            self.pub_list.append(&b);
        };
        add("All publications", None, selected.is_none());
        for group in logic::publication_groups(&pubs, self.kind.get(), self.language.get()) {
            if let Some(label) = &group.label {
                self.pub_list.append(&gtk::Label::builder().label(label).xalign(0.0).css_classes(["menu-header", "muted", "tiny"]).build());
            }
            for p in &group.rows {
                let active = selected == Some(p.id);
                if active {
                    trigger = p.name.clone();
                }
                add(&format!("{} · {}", p.name, p.issue_count), Some(p.id), active);
            }
        }
        // A selection outside the current groups still names itself.
        if let Some(id) = selected {
            if let Some(p) = pubs.iter().find(|p| p.id == id) {
                trigger = p.name.clone();
            }
        }
        self.pub_button.set_label(&trigger);
        self.pub_button.set_tooltip_text(Some(&trigger));
    }

    /// One issue's row or status changed: repaint the cards showing it.
    fn issue_changed(self: &Rc<Self>, key: &str) {
        let Some(issue) = store::issue(key) else { return };
        if self.favorites.get() {
            // Its favourite flag decides whether it is shown at all.
            self.refresh();
        }
        let cards: Vec<Rc<IssueCard>> = CARDS.with(|c| c.borrow().values().flatten().filter(|c| c.key() == key).cloned().collect());
        for c in cards {
            c.set_issue(issue.clone());
        }
        let rows: Vec<Rc<IssueRow>> = ROWS.with(|r| r.borrow().values().filter(|r| r.key() == key).cloned().collect());
        for r in rows {
            r.set_issue(issue.clone());
        }
        for i in 0..self.issues_store.n_items() {
            if let Some(o) = self.issues_store.item(i).and_downcast::<IssueObject>() {
                if o.issue().key == key {
                    o.set_issue(issue.clone());
                }
            }
        }
    }

    /// The top bar's search box filters titles and publication names too.
    #[allow(dead_code)]
    pub fn set_query(self: &Rc<Self>, q: &str) {
        if self.search.text().as_str() != q {
            self.search.set_text(q);
        }
    }

    // ── Actions ──

    fn open_issue(self: &Rc<Self>, issue: Issue) {
        if issue.runnable {
            return;
        }
        if notice::needs() {
            let window = self.window.clone();
            notice::show(&self.widget, "Open the issue", move || reader::open(&window, issue, None));
            return;
        }
        reader::open(&self.window, issue, None);
    }

    /// A disk magazine's one action: fetch it, then run it.
    fn play_issue(self: &Rc<Self>, issue: Issue) {
        if store::is_busy(&issue.key) {
            return;
        }
        if !issue.installed {
            if bus::offline() {
                return;
            }
            if notice::needs() {
                let key = issue.key.clone();
                notice::show(&self.widget, "Download the issue", move || store::install_issue(&key));
                return;
            }
            store::install_issue(&issue.key);
            return;
        }
        let title = issue.title.clone();
        store::launch(&issue.key, move |res| {
            if let Err(e) = res {
                bus::toast_with(&format!("Couldn't launch {title}"), Some(e.trim_start_matches("Error: ")), None);
            }
        });
    }

    /// Same shape as the grid's right-click menu on a game: the destructive
    /// action lives there, not on the card.
    fn show_menu(self: &Rc<Self>, issue: Issue, widget: &gtk::Widget, x: f64, y: f64) {
        if !store::issue_on_disk(&issue) {
            return;
        }
        let menu = gtk::Popover::builder().has_arrow(false).position(gtk::PositionType::Bottom).css_classes(["context-menu"]).build();
        menu.set_parent(widget);
        menu.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        let items = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let b = gtk::Button::builder().label("Remove from disk").css_classes(["menu-item", "danger"]).tooltip_text("Delete the downloaded files; the issue stays in the catalogue").build();
        if let Some(l) = b.child().and_downcast::<gtk::Label>() {
            l.set_xalign(0.0);
        }
        let m = menu.clone();
        b.connect_clicked(glib::clone!(#[weak(rename_to = room)] self, move |_| {
            m.popdown();
            room.confirm_remove(issue.clone());
        }));
        items.append(&b);
        menu.set_child(Some(&items));
        menu.connect_closed(|m| {
            let m = m.clone();
            glib::idle_add_local_once(move || m.unparent());
        });
        menu.popup();
    }

    fn confirm_remove(self: &Rc<Self>, issue: Issue) {
        dialogs::confirm(
            &self.widget,
            "Remove from disk",
            &format!("Delete the downloaded copy of \"{}\"? You can download it again whenever you like.", issue.title),
            "Remove",
            true,
            move || {
                let title = issue.title.clone();
                store::remove(&issue, move |res| {
                    if let Err(e) = res {
                        bus::toast_with(&format!("Couldn't remove {title}"), Some(e.trim_start_matches("Error: ")), None);
                    }
                });
            },
        );
    }

    // ── Poster pack hint ──

    /// One-time nudge that a poster pack would sharpen the covers: shown
    /// while the grid is on the bundled tier, the pack is available and the
    /// hint was not dismissed for the Media Pack.
    fn load_pack_hint(self: &Rc<Self>) {
        if bus::offline() {
            self.pack_hint.set_visible(false);
            return;
        }
        let core = app::core();
        app::spawn(
            async move {
                let dismissed = games::get_config(core.state(), "pack_hint_dismissed".into()).await.unwrap_or(None).unwrap_or_default();
                if dismissed.split(',').any(|c| c == covers::MEDIA_SOURCE || c == "*") {
                    return None;
                }
                if assets::get_poster_dir(core.state(), covers::MEDIA_SOURCE.into()).await.is_ok() {
                    return None;
                }
                let packs = content_packs::list_content_packs(core.state(), covers::MEDIA_SOURCE.into()).await.ok()?;
                packs.into_iter().find(|p| p.id == "posters" && p.available && !p.installed)
            },
            glib::clone!(#[weak(rename_to = room)] self, move |pack| {
                match pack {
                    Some(p) => {
                        room.pack_hint_desc.set_label(&format!(
                            "{} is an optional {} download. You can also manage it later in Settings.",
                            p.display_name,
                            format_bytes(p.size_bytes)
                        ));
                        room.pack_hint.set_visible(true);
                    }
                    None => room.pack_hint.set_visible(false),
                }
            }),
        );
    }

    /// Remember first: a failed start is still an answered question.
    fn pack_hint_answer(self: &Rc<Self>, install: bool) {
        self.pack_hint.set_visible(false);
        let core = app::core();
        app::spawn(
            async move {
                let current = games::get_config(core.state(), "pack_hint_dismissed".into()).await.unwrap_or(None).unwrap_or_default();
                let mut list: Vec<String> = current.split(',').filter(|s| !s.is_empty()).map(String::from).collect();
                if !list.iter().any(|c| c == covers::MEDIA_SOURCE) {
                    list.push(covers::MEDIA_SOURCE.into());
                }
                let _ = games::set_config(core.clone(), core.state(), "pack_hint_dismissed".into(), list.join(",")).await;
                if install {
                    content_packs::install_content_pack(core.clone(), covers::MEDIA_SOURCE.into(), "posters".into()).await
                } else {
                    Ok(())
                }
            },
            move |res| match res {
                Ok(()) if install => bus::toast("Downloading the Media Pack covers"),
                Err(e) => bus::toast_with("Couldn't start the cover download", Some(&e), None),
                _ => {}
            },
        );
    }
}

/// The articles ("Covered in") section for a game's detail panel.
#[allow(dead_code)]
pub fn game_articles_widget(game_id: i64, window: &gtk::Window) -> gtk::Widget {
    let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).css_classes(["detail-articles"]).visible(false).build();
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    head.append(&gtk::Label::builder().label("Covered in").xalign(0.0).css_classes(["title-3"]).build());
    let count = gtk::Label::builder().css_classes(["section-count", "muted", "small"]).valign(gtk::Align::Baseline).build();
    head.append(&count);
    root.append(&head);
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
    root.append(&list);

    let core = app::core();
    let window = window.clone();
    let weak = root.downgrade();
    app::spawn(
        async move {
            let language = games::get_game(core.state(), game_id).await.ok().flatten().map(|g| g.language);
            let articles = exorchy_core::commands::reading::game_articles(core.state(), game_id).await;
            (language, articles)
        },
        move |(language, articles)| {
            let Some(root) = weak.upgrade() else { return };
            let mut rows = match articles {
                Ok(a) => a,
                Err(e) => {
                    log::warn!("game_articles({game_id}): {e}");
                    return;
                }
            };
            if rows.is_empty() {
                return;
            }
            // The selected variant's language leads; the rest keep eXo's order.
            if let Some(lang) = language {
                rows.sort_by_key(|a| a.language != lang);
            }
            count.set_label(&rows.len().to_string());
            notice::load();
            for a in rows {
                let b = gtk::Button::builder().css_classes(["article-row"]).tooltip_text(format!("{}, page {}", a.issue_title, a.page)).build();
                let inner = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                inner.append(&gtk::Label::builder().label(&a.kind).css_classes(["badge", "badge-platform"]).valign(gtk::Align::Center).build());
                inner.append(&gtk::Label::builder().label(&a.issue_title).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
                if a.language == "DE" {
                    inner.append(&gtk::Label::builder().label("DE").css_classes(["badge", "badge-lang"]).valign(gtk::Align::Center).build());
                }
                inner.append(&gtk::Label::builder().label(format!("p. {}", a.page)).css_classes(["muted", "small"]).build());
                b.set_child(Some(&inner));
                let (window, key, page) = (window.clone(), a.issue_key.clone(), a.page);
                b.connect_clicked(move |b| open_article(&window, b.upcast_ref(), key.clone(), page));
                list.append(&b);
            }
            root.set_visible(true);
        },
    );
    root.upcast()
}

/// The same gate the reading room applies: the first issue opened anywhere
/// says once that reading joins a second torrent.
fn open_article(window: &gtk::Window, parent: &gtk::Widget, key: String, page: i64) {
    let core = app::core();
    let (window, parent) = (window.clone(), parent.clone());
    app::spawn(async move { exorchy_core::commands::reading::get_issue(core.state(), key).await }, move |res| {
        let issue = match res {
            Ok(Some(i)) => i,
            Ok(None) => return,
            Err(e) => {
                log::error!("could not open the article: {e}");
                return;
            }
        };
        if notice::needs() {
            let w = window.clone();
            notice::show(&parent, "Open the issue", move || reader::open(&w, issue, Some(page)));
        } else {
            reader::open(&window, issue, Some(page));
        }
    });
}
