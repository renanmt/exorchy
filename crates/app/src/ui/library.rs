//! The library: Browse (filters, collection shelf, virtualised grid or list,
//! jump bar), My Library (recently played, installed, favourites,
//! playlists) and the Reading Room tab, with the detail panel beside them.
//! Paging, the fetch epoch and the "library follows intent" refresh mirror
//! the web UI's `stores/games.ts` + `pages/Library.tsx`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use exorchy_core::commands::{games, playlists, setup};
use exorchy_core::models::Game;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

use crate::app;
use crate::ui::card::{Card, CARD_WIDTH};
use crate::ui::detail::DetailPanel;
use crate::ui::model::GameObject;
use crate::ui::util::{esc, format_bytes};
use crate::ui::{bus, covers, downloads};

const PER_PAGE: usize = 100;

pub const SORT_OPTIONS: [(&str, &str); 6] = [
    ("title", "Title A–Z"),
    ("title_desc", "Title Z–A"),
    ("year_desc", "Newest first"),
    ("year_asc", "Oldest first"),
    ("rating", "Top rated"),
    ("genre", "Genre A–Z"),
];

#[derive(Default)]
struct Filters {
    query: String,
    genre: String,
    sort_by: String,
    collection: String,
    playlist: Option<i64>,
    page: usize,
    has_more: bool,
    loading: bool,
    epoch: u64,
    total: usize,
}

thread_local! {
    /// Card behind each recycled grid/list child, so store-wide refreshes
    /// (download ticks, library changes) can find the visible ones.
    static CARDS: RefCell<HashMap<gtk::Widget, Rc<Card>>> = RefCell::new(HashMap::new());
}

pub struct LibraryPage {
    pub widget: gtk::Box,
    filters: Rc<RefCell<Filters>>,
    store: gio::ListStore,
    grid: gtk::GridView,
    list: gtk::ListView,
    scroller: gtk::ScrolledWindow,
    list_scroller: gtk::ScrolledWindow,
    view_stack: gtk::Stack,
    tab_stack: gtk::Stack,
    tab_buttons: RefCell<Vec<(String, gtk::Button)>>,
    search: gtk::SearchEntry,
    genre_drop: gtk::DropDown,
    genre_values: RefCell<Vec<String>>,
    sort_drop: gtk::DropDown,
    shelf: gtk::Box,
    shelf_buttons: RefCell<Vec<(String, gtk::ToggleButton)>>,
    jump_bar: gtk::Box,
    section_keys: RefCell<Vec<String>>,
    status: gtk::Label,
    detail: Rc<DetailPanel>,
    shelves: gtk::Box,
    reading_slot: gtk::Box,
    /// `activity` shows the session's transfer rates.
    pub activity: gtk::Label,
    pub settings_button: gtk::Button,
    pub bar_slot: gtk::Box,
    pub toolbar_slot: gtk::Box,
    playlist_menu: gtk::Box,
    window: gtk::Window,
}

impl LibraryPage {
    pub fn new(window: &gtk::Window) -> Rc<Self> {
        let detail = DetailPanel::new(window);

        // ── toolbar ──
        let toolbar = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(10).css_classes(["toolbar"]).build();
        let brand = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(0).css_classes(["brand"]).build();
        brand.append(&gtk::Label::new(Some("e")));
        brand.append(&gtk::Label::builder().label("X").css_classes(["x"]).build());
        brand.append(&gtk::Label::new(Some("orchy")));
        toolbar.append(&brand);
        let tabs = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(2).margin_start(12).build();
        let mut tab_buttons = Vec::new();
        for (id, label) in [("browse", "Browse"), ("library", "My Library"), ("reading", "Reading Room")] {
            let b = gtk::Button::builder().label(label).css_classes(["tab"]).build();
            tabs.append(&b);
            tab_buttons.push((id.to_string(), b));
        }
        toolbar.append(&tabs);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        toolbar.append(&spacer);
        let search = gtk::SearchEntry::builder().placeholder_text("Search games…  (/)").css_classes(["search"]).build();
        toolbar.append(&search);
        let activity = gtk::Label::builder().css_classes(["activity", "muted", "small"]).build();
        toolbar.append(&activity);
        // Feature modules (music button, ...) mount their toolbar controls here.
        let toolbar_slot = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(6).build();
        toolbar.append(&toolbar_slot);
        let settings_button = gtk::Button::builder().icon_name("emblem-system-symbolic").css_classes(["btn", "icon", "ghost"]).tooltip_text("Settings (Ctrl+,)").build();
        toolbar.append(&settings_button);

        // ── filter row ──
        let filter_row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).css_classes(["filter-row"]).build();
        let shelf = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(4).build();
        filter_row.append(&shelf);
        let fspacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        fspacer.set_hexpand(true);
        filter_row.append(&fspacer);
        let playlist_menu = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        filter_row.append(&playlist_menu);
        let genre_drop = gtk::DropDown::from_strings(&["All genres"]);
        genre_drop.add_css_class("drop");
        filter_row.append(&genre_drop);
        let sort_labels: Vec<&str> = SORT_OPTIONS.iter().map(|(_, l)| *l).collect();
        let sort_drop = gtk::DropDown::from_strings(&sort_labels);
        sort_drop.add_css_class("drop");
        filter_row.append(&sort_drop);
        let view_grid = gtk::ToggleButton::builder().icon_name("view-grid-symbolic").active(true).css_classes(["btn", "icon"]).tooltip_text("Grid").build();
        let view_list = gtk::ToggleButton::builder().icon_name("view-list-symbolic").group(&view_grid).css_classes(["btn", "icon"]).tooltip_text("List").build();
        let view_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        view_box.add_css_class("linked");
        view_box.append(&view_grid);
        view_box.append(&view_list);
        filter_row.append(&view_box);

        // ── models and views ──
        let store = gio::ListStore::new::<GameObject>();
        let selection = gtk::NoSelection::new(Some(store.clone()));
        let grid = gtk::GridView::builder()
            .model(&selection)
            .min_columns(2)
            .max_columns(12)
            .single_click_activate(false)
            .css_classes(["game-grid"])
            // Rows take the cards' minimum height (the cover's requested
            // size plus the body), not a natural height the grid computes
            // from the widest possible cover.
            .vscroll_policy(gtk::ScrollablePolicy::Minimum)
            .build();
        let list = gtk::ListView::builder().model(&selection).css_classes(["game-list"]).build();
        // Each view is its own ScrolledWindow's direct child: a GridView
        // only virtualises rows when it is the scrollable itself.
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .child(&grid)
            .build();
        let list_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .child(&list)
            .build();
        let list_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list_page.append(&list_header());
        list_page.append(&list_scroller);
        let view_stack = gtk::Stack::new();
        view_stack.add_named(&scroller, Some("grid"));
        view_stack.add_named(&list_page, Some("list"));
        let status = gtk::Label::builder().css_classes(["muted", "small"]).xalign(0.0).margin_start(14).margin_bottom(6).build();

        let browse = gtk::Box::new(gtk::Orientation::Vertical, 0);
        browse.append(&filter_row);
        browse.append(&view_stack);
        browse.append(&status);

        // Jump bar beside the scroller.
        let jump_bar = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(0).css_classes(["jump-bar"]).valign(gtk::Align::Start).build();
        let jump_scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vscrollbar_policy(gtk::PolicyType::External).child(&jump_bar).build();
        let browse_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        browse_row.append(&browse);
        browse_row.append(&jump_scroller);

        // My Library shelves.
        let shelves = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).margin_top(12).margin_bottom(24).margin_start(14).margin_end(14).build();
        let shelves_scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).hexpand(true).child(&shelves).build();

        let reading_slot = gtk::Box::builder().orientation(gtk::Orientation::Vertical).vexpand(true).hexpand(true).build();
        reading_slot.append(&gtk::Label::builder().label("The Reading Room is being ported.").css_classes(["muted"]).vexpand(true).build());

        let tab_stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::SlideLeftRight).vexpand(true).hexpand(true).build();
        tab_stack.add_named(&browse_row, Some("browse"));
        tab_stack.add_named(&shelves_scroller, Some("library"));
        tab_stack.add_named(&reading_slot, Some("reading"));

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        content.append(&tab_stack);
        content.append(&detail.widget);

        // The now-playing bar (ui::media) mounts under the content.
        let bar_slot = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.append(&toolbar);
        widget.append(&content);
        widget.append(&bar_slot);

        let page = Rc::new(LibraryPage {
            widget,
            filters: Rc::new(RefCell::new(Filters { sort_by: "title".into(), has_more: true, ..Default::default() })),
            store,
            grid,
            list,
            scroller,
            list_scroller,
            view_stack,
            tab_stack,
            tab_buttons: RefCell::new(tab_buttons),
            search,
            genre_drop,
            genre_values: RefCell::new(vec![String::new()]),
            sort_drop,
            shelf,
            shelf_buttons: RefCell::new(Vec::new()),
            jump_bar,
            section_keys: RefCell::new(Vec::new()),
            status,
            detail,
            shelves,
            reading_slot,
            activity,
            settings_button,
            bar_slot,
            toolbar_slot,
            playlist_menu,
            window: window.clone(),
        });

        page.setup_factories();
        page.wire(view_grid, view_list);
        page.set_tab("browse");
        page
    }

    // ── factories ──

    fn setup_factories(self: &Rc<Self>) {
        let on_detail: Rc<dyn Fn(Game)> = {
            let page = Rc::downgrade(self);
            Rc::new(move |g| {
                if let Some(p) = page.upgrade() {
                    p.detail.show(g);
                }
            })
        };

        let factory = gtk::SignalListItemFactory::new();
        let od = on_detail.clone();
        let page = Rc::downgrade(self);
        factory.connect_setup(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let card = Card::new(od.clone());
            if let Some(p) = page.upgrade() {
                card.set_hide_platform(!p.filters.borrow().collection.is_empty());
            }
            item.set_child(Some(&card.widget));
            item.set_activatable(true);
            item.set_selectable(false);
            CARDS.with(|c| c.borrow_mut().insert(card.widget.clone().upcast(), card));
        });
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(obj) = item.item().and_downcast::<GameObject>() else { return };
            let Some(child) = item.child() else { return };
            if let Some(card) = CARDS.with(|c| c.borrow().get(&child).cloned()) {
                card.bind(&obj.game());
            }
        });
        factory.connect_teardown(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            if let Some(child) = item.child() {
                CARDS.with(|c| c.borrow_mut().remove(&child));
            }
        });
        self.grid.set_factory(Some(&factory));
        // Enter / Space on a focused card (keyboard navigation) opens it.
        self.grid.connect_activate(glib::clone!(#[weak(rename_to = page)] self, move |gv, pos| {
            if let Some(obj) = gv.model().and_then(|m| m.item(pos)).and_downcast::<GameObject>() {
                page.detail.show(obj.game());
            }
        }));

        let row_factory = gtk::SignalListItemFactory::new();
        row_factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            item.set_child(Some(&row_widget()));
        });
        let od = on_detail.clone();
        row_factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(obj) = item.item().and_downcast::<GameObject>() else { return };
            if let Some(row) = item.child() {
                bind_row(&row, &obj.game(), od.clone());
            }
        });
        self.list.set_factory(Some(&row_factory));
        self.list.connect_activate(glib::clone!(#[weak(rename_to = page)] self, move |lv, pos| {
            if let Some(obj) = lv.model().and_then(|m| m.item(pos)).and_downcast::<GameObject>() {
                page.detail.show(obj.game());
            }
        }));
        self.list.set_single_click_activate(true);
    }

    fn wire(self: &Rc<Self>, view_grid: gtk::ToggleButton, view_list: gtk::ToggleButton) {
        // Tabs
        for (id, b) in self.tab_buttons.borrow().iter() {
            let id = id.clone();
            b.connect_clicked(glib::clone!(#[weak(rename_to = page)] self, move |_| page.set_tab(&id)));
        }
        // Search (GTK debounces search-changed ~150 ms; a second beat keeps typing cheap).
        self.search.connect_search_changed(glib::clone!(#[weak(rename_to = page)] self, move |e| {
            let q = e.text().to_string();
            crate::ui::reading::set_query(&q);
            page.filters.borrow_mut().query = q;
            page.debounced_fetch();
        }));
        self.search.connect_stop_search(glib::clone!(#[weak(rename_to = page)] self, move |e| {
            e.set_text("");
            page.grid.grab_focus();
        }));
        // Genre / sort
        self.genre_drop.connect_selected_notify(glib::clone!(#[weak(rename_to = page)] self, move |d| {
            let v = page.genre_values.borrow().get(d.selected() as usize).cloned().unwrap_or_default();
            if page.filters.borrow().genre != v {
                page.filters.borrow_mut().genre = v;
                page.fetch();
            }
        }));
        self.sort_drop.connect_selected_notify(glib::clone!(#[weak(rename_to = page)] self, move |d| {
            let v = SORT_OPTIONS.get(d.selected() as usize).map(|(k, _)| k.to_string()).unwrap_or_default();
            if page.filters.borrow().sort_by != v {
                page.filters.borrow_mut().sort_by = v;
                page.fetch();
            }
        }));
        // View mode
        view_grid.connect_toggled(glib::clone!(#[weak(rename_to = page)] self, move |b| {
            if b.is_active() {
                page.view_stack.set_visible_child_name("grid");
                let core = app::core();
                app::spawn(async move { games::set_config(core.clone(), core.state(), "view_mode".into(), "grid".into()).await }, |_| {});
            }
        }));
        view_list.connect_toggled(glib::clone!(#[weak(rename_to = page)] self, move |b| {
            if b.is_active() {
                page.view_stack.set_visible_child_name("list");
                let core = app::core();
                app::spawn(async move { games::set_config(core.clone(), core.state(), "view_mode".into(), "list".into()).await }, |_| {});
            }
        }));
        {
            let core = app::core();
            let vl = view_list.clone();
            app::spawn(async move { games::get_config(core.state(), "view_mode".into()).await }, move |r| {
                if let Ok(Some(v)) = r {
                    if v == "list" {
                        vl.set_active(true);
                    }
                }
            });
        }
        // Infinite scroll, both views.
        for sc in [&self.scroller, &self.list_scroller] {
            sc.vadjustment().connect_value_changed(glib::clone!(#[weak(rename_to = page)] self, move |adj| {
                if adj.value() + adj.page_size() >= adj.upper() - 900.0 {
                    page.fetch_more();
                }
            }));
        }
        // Store-wide refreshes
        downloads::on_change(glib::clone!(#[weak(rename_to = page)] self, move |id| {
            refresh_cards_for(id);
            page.detail.refresh_download(id);
            // A finished download must land in the grid's flags too.
            if downloads::state(id).map(|s| !s.downloading).unwrap_or(true) {
                page.refresh_loaded();
            }
        }));
        bus::on_library_changed(glib::clone!(#[weak(rename_to = page)] self, move |id| {
            page.refresh_loaded();
            page.refresh_shelves();
            page.detail.refresh_by_id(*id);
        }));
        bus::on_running_changed(glib::clone!(#[weak(rename_to = page)] self, move |_| {
            page.detail.refresh_running();
        }));
        bus::on_collections_changed(glib::clone!(#[weak(rename_to = page)] self, move |_| {
            page.load_collections();
        }));
        bus::on_playlists_changed(glib::clone!(#[weak(rename_to = page)] self, move |_| {
            page.load_playlists();
            page.refresh_shelves();
        }));
        covers::on_dirs_changed(glib::clone!(#[weak(rename_to = page)] self, move || {
            page.rebind_cards();
        }));
        // Keyboard: "/" focuses search, Escape closes the panel.
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(#[weak(rename_to = page)] self, #[upgrade_or] glib::Propagation::Proceed, move |_, key, _, state| {
            let focused_entry = gtk::prelude::RootExt::focus(&page.window).map(|w| w.is::<gtk::Text>() || w.is::<gtk::Entry>() || w.is::<gtk::SearchEntry>()).unwrap_or(false);
            if key == gtk::gdk::Key::slash && !focused_entry {
                page.search.grab_focus();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::Escape && page.detail.is_open() {
                page.detail.close();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::comma && state.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                page.settings_button.emit_clicked();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }));
        self.window.add_controller(keys);
    }

    // ── tabs ──

    pub fn set_tab(self: &Rc<Self>, id: &str) {
        self.tab_stack.set_visible_child_name(id);
        for (tid, b) in self.tab_buttons.borrow().iter() {
            if tid == id {
                b.add_css_class("active");
            } else {
                b.remove_css_class("active");
            }
        }
        if id == "library" {
            self.refresh_shelves();
        }
    }

    /// The Reading Room mounts its own widget here.
    pub fn set_reading_widget(&self, w: &impl IsA<gtk::Widget>) {
        while let Some(c) = self.reading_slot.first_child() {
            self.reading_slot.remove(&c);
        }
        self.reading_slot.append(w);
    }

    // ── collections shelf, genres, playlists ──

    pub fn load_collections(self: &Rc<Self>) {
        let core = app::core();
        app::spawn(
            async move {
                let enabled = games::get_config(core.state(), "collections".into()).await.ok().flatten().unwrap_or_else(|| "eXoDOS".into());
                let all = setup::get_available_collections(core.state()).await.unwrap_or_default();
                (enabled, all)
            },
            glib::clone!(#[weak(rename_to = page)] self, move |(enabled, all)| {
                let enabled: Vec<&str> = enabled.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
                let cols: Vec<(String, String, i64)> = all
                    .iter()
                    .filter(|c| enabled.contains(&c.id.as_str()))
                    .map(|c| (c.id.clone(), c.display_name.clone(), c.game_count))
                    .collect();
                while let Some(c) = page.shelf.first_child() {
                    page.shelf.remove(&c);
                }
                let mut buttons = Vec::new();
                page.shelf.set_visible(cols.len() > 1);
                if cols.len() > 1 {
                    let mut entries = vec![(String::new(), "All".to_string(), 0)];
                    entries.extend(cols.iter().cloned());
                    let mut group: Option<gtk::ToggleButton> = None;
                    for (id, label, count) in entries {
                        let text = if count > 0 { format!("{label}  {count}") } else { label.clone() };
                        let b = gtk::ToggleButton::builder().label(&text).css_classes(["shelf-btn"]).build();
                        if let Some(g) = &group {
                            b.set_group(Some(g));
                        } else {
                            group = Some(b.clone());
                        }
                        let id2 = id.clone();
                        b.connect_toggled(glib::clone!(#[weak] page, move |b| {
                            if b.is_active() && page.filters.borrow().collection != id2 {
                                page.filters.borrow_mut().collection = id2.clone();
                                page.load_genres();
                                page.fetch();
                            }
                        }));
                        page.shelf.append(&b);
                        buttons.push((id, b));
                    }
                }
                // A filter on a collection that was just disabled would show nothing.
                let current = page.filters.borrow().collection.clone();
                let valid = cols.len() > 1 && (current.is_empty() || cols.iter().any(|c| c.0 == current));
                let target = if valid { current } else { String::new() };
                page.filters.borrow_mut().collection = target.clone();
                for (id, b) in &buttons {
                    if *id == target {
                        b.set_active(true);
                    }
                }
                page.shelf_buttons.replace(buttons);
                page.load_genres();
                page.load_playlists();
                page.fetch();
            }),
        );
    }

    fn load_genres(self: &Rc<Self>) {
        let core = app::core();
        let col = self.filters.borrow().collection.clone();
        app::spawn(async move { games::get_genres(core.state(), Some(col)).await }, glib::clone!(#[weak(rename_to = page)] self, move |r| {
            let flat = r.unwrap_or_default();
            // Parent/child tree from " / " entries; a parent filters by prefix.
            let mut groups: Vec<(String, Vec<String>)> = Vec::new();
            for g in &flat {
                match g.split_once(" / ") {
                    Some((p, c)) => match groups.iter_mut().find(|(k, _)| k == p) {
                        Some((_, kids)) => kids.push(c.to_string()),
                        None => groups.push((p.to_string(), vec![c.to_string()])),
                    },
                    None => {
                        if !groups.iter().any(|(k, _)| k == g) {
                            groups.push((g.clone(), vec![]));
                        }
                    }
                }
            }
            groups.sort_by(|a, b| a.0.cmp(&b.0));
            let mut values = vec![String::new()];
            let mut labels = vec!["All genres".to_string()];
            for (p, kids) in groups {
                values.push(p.clone());
                labels.push(p.clone());
                let mut kids = kids;
                kids.sort();
                for k in kids {
                    values.push(format!("{p} / {k}"));
                    labels.push(format!("   {k}"));
                }
            }
            let current = page.filters.borrow().genre.clone();
            let idx = values.iter().position(|v| *v == current).unwrap_or(0);
            let strs: Vec<&str> = labels.iter().map(String::as_str).collect();
            page.genre_drop.set_model(Some(&gtk::StringList::new(&strs)));
            page.genre_values.replace(values);
            page.genre_drop.set_selected(idx as u32);
        }));
    }

    pub fn load_playlists(self: &Rc<Self>) {
        let core = app::core();
        app::spawn(async move { playlists::get_playlists(core.state()).await }, glib::clone!(#[weak(rename_to = page)] self, move |r| {
            let lists = r.unwrap_or_default();
            while let Some(c) = page.playlist_menu.first_child() {
                page.playlist_menu.remove(&c);
            }
            // The filtered playlist was deleted: back to the whole catalogue.
            let current = page.filters.borrow().playlist;
            if current.is_some() && !lists.iter().any(|p| Some(p.id) == current) {
                page.filters.borrow_mut().playlist = None;
                page.fetch();
            }
            if lists.is_empty() {
                let new_btn = gtk::Button::builder().label("New playlist…").css_classes(["btn", "ghost"]).build();
                new_btn.connect_clicked(crate::ui::playlists::manage);
                page.playlist_menu.append(&new_btn);
                return;
            }
            let mut labels = vec!["All playlists".to_string()];
            labels.extend(lists.iter().map(|p| format!("{} ({})", p.name, p.game_count)));
            let strs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let drop = gtk::DropDown::from_strings(&strs);
            drop.add_css_class("drop");
            let ids: Vec<Option<i64>> = std::iter::once(None).chain(lists.iter().map(|p| Some(p.id))).collect();
            let current = page.filters.borrow().playlist;
            drop.set_selected(ids.iter().position(|i| *i == current).unwrap_or(0) as u32);
            drop.connect_selected_notify(glib::clone!(#[weak] page, move |d| {
                let v = ids.get(d.selected() as usize).copied().flatten();
                if page.filters.borrow().playlist != v {
                    page.filters.borrow_mut().playlist = v;
                    page.fetch();
                }
            }));
            page.playlist_menu.append(&drop);
            let manage = gtk::Button::builder().icon_name("document-edit-symbolic").css_classes(["btn", "icon", "ghost"]).tooltip_text("Manage playlists").build();
            manage.connect_clicked(crate::ui::playlists::manage);
            page.playlist_menu.append(&manage);
        }));
    }

    // ── fetching ──

    fn args(&self) -> (String, String, String, String, Option<i64>) {
        let f = self.filters.borrow();
        (f.query.clone(), f.genre.clone(), f.sort_by.clone(), f.collection.clone(), f.playlist)
    }

    fn debounced_fetch(self: &Rc<Self>) {
        let epoch = {
            let mut f = self.filters.borrow_mut();
            f.epoch += 1;
            f.epoch
        };
        glib::timeout_add_local_once(Duration::from_millis(220), glib::clone!(#[weak(rename_to = page)] self, move || {
            if page.filters.borrow().epoch == epoch {
                page.fetch();
            }
        }));
    }

    /// First page; resets the list and the section keys.
    pub fn fetch(self: &Rc<Self>) {
        let epoch = {
            let mut f = self.filters.borrow_mut();
            f.epoch += 1;
            f.loading = true;
            f.page = 1;
            f.epoch
        };
        let hide_platform = !self.filters.borrow().collection.is_empty();
        CARDS.with(|c| c.borrow().values().for_each(|card| card.set_hide_platform(hide_platform)));
        let (q, g, s, c, p) = self.args();
        let core = app::core();
        app::spawn(
            async move { games::get_games(core.state(), Some(1), Some(PER_PAGE), Some(q), Some(g), Some(s), Some(c), Some(false), p, Some(false)).await },
            glib::clone!(#[weak(rename_to = page)] self, move |res| {
                if page.filters.borrow().epoch != epoch {
                    return;
                }
                page.filters.borrow_mut().loading = false;
                match res {
                    Ok(list) => {
                        let n = list.games.len();
                        let objs: Vec<GameObject> = list.games.into_iter().map(GameObject::new).collect();
                        page.store.remove_all();
                        page.store.extend_from_slice(&objs);
                        let mut f = page.filters.borrow_mut();
                        f.total = list.total;
                        f.has_more = n < list.total;
                        drop(f);
                        page.scroller.vadjustment().set_value(0.0);
                        page.list_scroller.vadjustment().set_value(0.0);
                        page.update_status();
                    }
                    Err(e) => page.status.set_label(&format!("Couldn't load games: {e}")),
                }
            }),
        );
        self.load_section_keys(epoch);
    }

    fn fetch_more(self: &Rc<Self>) {
        let (epoch, next) = {
            let mut f = self.filters.borrow_mut();
            if f.loading || !f.has_more {
                return;
            }
            f.loading = true;
            f.epoch += 1;
            (f.epoch, f.page + 1)
        };
        let (q, g, s, c, p) = self.args();
        let core = app::core();
        app::spawn(
            async move { games::get_games(core.state(), Some(next), Some(PER_PAGE), Some(q), Some(g), Some(s), Some(c), Some(false), p, Some(false)).await },
            glib::clone!(#[weak(rename_to = page)] self, move |res| {
                if page.filters.borrow().epoch != epoch {
                    return;
                }
                page.filters.borrow_mut().loading = false;
                if let Ok(list) = res {
                    let objs: Vec<GameObject> = list.games.into_iter().map(GameObject::new).collect();
                    page.store.extend_from_slice(&objs);
                    let mut f = page.filters.borrow_mut();
                    f.page = next;
                    f.total = list.total;
                    f.has_more = (page.store.n_items() as usize) < list.total;
                    drop(f);
                    page.update_status();
                }
            }),
        );
    }

    /// Everything, for a jump to a section that is not loaded yet.
    fn fetch_all(self: &Rc<Self>, then: impl FnOnce() + 'static) {
        let epoch = {
            let mut f = self.filters.borrow_mut();
            f.loading = true;
            f.epoch += 1;
            f.epoch
        };
        let (q, g, s, c, p) = self.args();
        let total = self.filters.borrow().total.max(9999);
        let core = app::core();
        app::spawn(
            async move { games::get_games(core.state(), Some(1), Some(total), Some(q), Some(g), Some(s), Some(c), Some(false), p, Some(false)).await },
            glib::clone!(#[weak(rename_to = page)] self, move |res| {
                if page.filters.borrow().epoch != epoch {
                    return;
                }
                page.filters.borrow_mut().loading = false;
                if let Ok(list) = res {
                    let objs: Vec<GameObject> = list.games.into_iter().map(GameObject::new).collect();
                    page.store.remove_all();
                    page.store.extend_from_slice(&objs);
                    page.filters.borrow_mut().has_more = false;
                    page.update_status();
                    then();
                }
            }),
        );
    }

    /// Re-fetch every loaded row in place (a full fetch would reset the scroll).
    pub fn refresh_loaded(self: &Rc<Self>) {
        let count = self.store.n_items() as usize;
        if count == 0 {
            self.fetch();
            return;
        }
        if self.filters.borrow().loading {
            return;
        }
        let epoch = {
            let mut f = self.filters.borrow_mut();
            f.epoch += 1;
            f.epoch
        };
        let (q, g, s, c, p) = self.args();
        let core = app::core();
        app::spawn(
            async move { games::get_games(core.state(), Some(1), Some(count.max(PER_PAGE)), Some(q), Some(g), Some(s), Some(c), Some(false), p, Some(false)).await },
            glib::clone!(#[weak(rename_to = page)] self, move |res| {
                if page.filters.borrow().epoch != epoch {
                    return;
                }
                let Ok(list) = res else { return };
                // Update rows in place where the id matches, so recycled
                // cards keep their scroll position; otherwise splice.
                let n = page.store.n_items() as usize;
                if list.games.len() == n
                    && list.games.iter().enumerate().all(|(i, g)| page.store.item(i as u32).and_downcast::<GameObject>().map(|o| o.id() == g.id).unwrap_or(false))
                {
                    for (i, g) in list.games.into_iter().enumerate() {
                        if let Some(o) = page.store.item(i as u32).and_downcast::<GameObject>() {
                            o.set_game(g);
                        }
                    }
                    page.rebind_cards();
                } else {
                    let objs: Vec<GameObject> = list.games.into_iter().map(GameObject::new).collect();
                    page.store.splice(0, page.store.n_items(), &objs);
                }
                let mut f = page.filters.borrow_mut();
                f.total = list.total;
                f.has_more = (page.store.n_items() as usize) < list.total;
                drop(f);
                page.update_status();
            }),
        );
    }

    /// Re-run `bind` on every recycled card from its current row.
    fn rebind_cards(&self) {
        let n = self.store.n_items();
        let by_id: HashMap<i64, Game> = (0..n)
            .filter_map(|i| self.store.item(i).and_downcast::<GameObject>())
            .filter_map(|o| o.id().map(|id| (id, o.game())))
            .collect();
        CARDS.with(|c| {
            for card in c.borrow().values() {
                if let Some(g) = card.game_id().and_then(|id| by_id.get(&id)) {
                    card.bind(g);
                }
            }
        });
    }

    fn update_status(&self) {
        let f = self.filters.borrow();
        let shown = self.store.n_items() as usize;
        self.status.set_label(&if f.total == 0 {
            if f.query.is_empty() { "No games match these filters.".to_string() } else { format!("No games match “{}”.", f.query) }
        } else if shown < f.total {
            format!("{shown} of {} games", f.total)
        } else {
            format!("{} games", f.total)
        });
    }

    // ── jump bar ──

    fn load_section_keys(self: &Rc<Self>, epoch: u64) {
        let (q, g, s, c, p) = self.args();
        let core = app::core();
        app::spawn(
            async move { games::get_section_keys(core.state(), Some(s), Some(q), Some(g), Some(c), Some(false), p, Some(false)).await },
            glib::clone!(#[weak(rename_to = page)] self, move |res| {
                if page.filters.borrow().epoch < epoch {
                    return;
                }
                let keys = res.unwrap_or_default();
                page.section_keys.replace(keys.clone());
                while let Some(c) = page.jump_bar.first_child() {
                    page.jump_bar.remove(&c);
                }
                page.jump_bar.set_visible(keys.len() > 1);
                for key in keys {
                    let b = gtk::Button::builder().label(&key).css_classes(["jump-btn"]).build();
                    let k = key.clone();
                    b.connect_clicked(glib::clone!(#[weak] page, move |_| page.jump_to(&k)));
                    page.jump_bar.append(&b);
                }
            }),
        );
    }

    fn jump_to(self: &Rc<Self>, key: &str) {
        let sort = self.filters.borrow().sort_by.clone();
        let find = |store: &gio::ListStore| -> Option<u32> {
            (0..store.n_items()).find(|i| {
                store.item(*i).and_downcast::<GameObject>().map(|o| group_key(&o.game(), &sort) == key).unwrap_or(false)
            })
        };
        if let Some(pos) = find(&self.store) {
            self.scroll_to(pos);
            return;
        }
        if !self.filters.borrow().has_more {
            return;
        }
        let key = key.to_string();
        let page = Rc::downgrade(self);
        self.fetch_all(move || {
            let Some(page) = page.upgrade() else { return };
            let sort = page.filters.borrow().sort_by.clone();
            let pos = (0..page.store.n_items()).find(|i| {
                page.store.item(*i).and_downcast::<GameObject>().map(|o| group_key(&o.game(), &sort) == key).unwrap_or(false)
            });
            if let Some(pos) = pos {
                page.scroll_to(pos);
            }
        });
    }

    fn scroll_to(&self, pos: u32) {
        if self.view_stack.visible_child_name().as_deref() == Some("list") {
            self.list.scroll_to(pos, gtk::ListScrollFlags::NONE, None);
        } else {
            self.grid.scroll_to(pos, gtk::ListScrollFlags::NONE, None);
        }
    }

    // ── My Library shelves ──

    pub fn refresh_shelves(self: &Rc<Self>) {
        if self.tab_stack.visible_child_name().as_deref() != Some("library") {
            return;
        }
        let core = app::core();
        app::spawn(
            async move {
                let recent = games::get_recently_played(core.state(), Some(12)).await.unwrap_or_default();
                let installed = games::get_installed_games(core.state()).await.unwrap_or_default();
                let favorites = games::get_games(core.state(), Some(1), Some(500), None, None, Some("title".into()), None, Some(true), None, None)
                    .await
                    .map(|l| l.games)
                    .unwrap_or_default();
                let lists = playlists::get_playlists(core.state()).await.unwrap_or_default();
                let mut playlist_games = Vec::new();
                for p in lists.into_iter().filter(|p| p.kind == "user") {
                    let g = games::get_games(core.state(), Some(1), Some(200), None, None, Some("title".into()), None, None, Some(p.id), None)
                        .await
                        .map(|l| l.games)
                        .unwrap_or_default();
                    playlist_games.push((p.name, g));
                }
                (recent, installed, favorites, playlist_games)
            },
            glib::clone!(#[weak(rename_to = page)] self, move |(recent, installed, favorites, lists)| {
                while let Some(c) = page.shelves.first_child() {
                    page.shelves.remove(&c);
                }
                let on_detail: Rc<dyn Fn(Game)> = {
                    let p = Rc::downgrade(&page);
                    Rc::new(move |g| {
                        if let Some(p) = p.upgrade() {
                            p.detail.show(g);
                        }
                    })
                };
                let mut any = false;
                for (title, list) in [("Recently played", recent), ("Installed", installed), ("Favorites", favorites)] {
                    if !list.is_empty() {
                        any = true;
                        page.shelves.append(&shelf(title, &list, on_detail.clone()));
                    }
                }
                for (name, list) in lists {
                    if !list.is_empty() {
                        any = true;
                        page.shelves.append(&shelf(&format!("Playlist · {name}"), &list, on_detail.clone()));
                    }
                }
                if !any {
                    let empty = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).valign(gtk::Align::Center).vexpand(true).halign(gtk::Align::Center).margin_top(80).build();
                    empty.append(&gtk::Label::builder().label("Your library is empty").css_classes(["title-2"]).build());
                    empty.append(&gtk::Label::builder().label("Download a game from Browse and it shows up here.").css_classes(["muted"]).build());
                    page.shelves.append(&empty);
                }
            }),
        );
    }

    pub fn detail(&self) -> Rc<DetailPanel> {
        self.detail.clone()
    }
}

/// Section label per row, matching `get_section_keys` server-side.
pub fn group_key(g: &Game, sort: &str) -> String {
    match sort {
        "title" | "title_desc" => {
            let first = g.sort_title.as_deref().unwrap_or(&g.title).chars().next().map(|c| c.to_ascii_uppercase()).unwrap_or('#');
            if first.is_ascii_alphabetic() { first.to_string() } else { "#".into() }
        }
        "year_asc" | "year_desc" => g.year.map(|y| y.to_string()).unwrap_or_else(|| "Unknown".into()),
        "rating" => match g.rating {
            None => "Unrated".into(),
            Some(r) => {
                let n = r.round().clamp(0.0, 5.0) as usize;
                format!("{}{}", "★".repeat(n), "☆".repeat(5 - n))
            }
        },
        "genre" => {
            let raw = g.genre.as_deref().unwrap_or("");
            let first = raw.split(';').next().unwrap_or("").trim();
            let parent = first.split(" / ").next().unwrap_or("").trim();
            if parent.is_empty() { "Unknown".into() } else { parent.to_string() }
        }
        _ => String::new(),
    }
}

fn refresh_cards_for(id: i64) {
    CARDS.with(|c| {
        for card in c.borrow().values() {
            if card.game_id() == Some(id) {
                card.refresh_download();
            }
        }
    });
}

fn shelf(title: &str, list: &[Game], on_detail: Rc<dyn Fn(Game)>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let head = gtk::Label::builder().label(title).xalign(0.0).css_classes(["title-3"]).build();
    b.append(&head);
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .halign(gtk::Align::Start)
        .column_spacing(10)
        .row_spacing(10)
        .min_children_per_line(2)
        .max_children_per_line(20)
        .build();
    for g in list {
        let card = Card::new(on_detail.clone());
        card.bind(g);
        card.widget.set_size_request(CARD_WIDTH, -1);
        card.widget.set_halign(gtk::Align::Start);
        // Shelf cards are not recycled; keep them findable for refreshes.
        CARDS.with(|c| c.borrow_mut().insert(card.widget.clone().upcast(), card.clone()));
        flow.insert(&card.widget, -1);
    }
    b.append(&flow);
    b
}

// ── list view rows ──

const COLS: [(&str, i32); 8] = [("Title", 320), ("Year", 60), ("Genre", 180), ("Developer", 160), ("Publisher", 160), ("Rating", 70), ("Size", 80), ("Status", 120)];

fn list_header() -> gtk::Box {
    let header = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).css_classes(["list-header"]).build();
    for (label, w) in COLS {
        header.append(&gtk::Label::builder().label(label).xalign(0.0).width_request(w).css_classes(["list-col"]).build());
    }
    header
}

fn row_widget() -> gtk::Widget {
    let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).css_classes(["game-row"]).build();
    for (_, w) in COLS {
        row.append(&gtk::Label::builder().xalign(0.0).width_request(w).ellipsize(gtk::pango::EllipsizeMode::End).build());
    }
    row.upcast()
}

fn bind_row(row: &gtk::Widget, g: &Game, _on_detail: Rc<dyn Fn(Game)>) {
    let labels: Vec<gtk::Label> = {
        let mut v = Vec::new();
        let mut c = row.first_child();
        while let Some(w) = c {
            if let Some(l) = w.downcast_ref::<gtk::Label>() {
                v.push(l.clone());
            }
            c = w.next_sibling();
        }
        v
    };
    let status = if g.installed {
        "Installed"
    } else if g.in_library {
        "Incomplete"
    } else {
        ""
    };
    let values = [
        g.title.clone(),
        g.year.map(|y| y.to_string()).unwrap_or_default(),
        g.genre.clone().unwrap_or_default(),
        g.developer.clone().unwrap_or_default(),
        g.publisher.clone().unwrap_or_default(),
        g.rating.map(|r| format!("{r:.1}")).unwrap_or_default(),
        g.download_size.filter(|s| *s > 0).map(|s| format_bytes(s as u64)).unwrap_or_default(),
        status.to_string(),
    ];
    for (l, v) in labels.iter().zip(values.iter()) {
        l.set_label(v);
    }
    if let Some(first) = labels.first() {
        first.set_markup(&format!("<b>{}</b>", esc(&g.title)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(title: &str, year: Option<i32>, rating: Option<f64>, genre: Option<&str>) -> Game {
        Game {
            id: Some(1),
            title: title.into(),
            sort_title: None,
            platform: "MS-DOS".into(),
            developer: None,
            publisher: None,
            release_date: None,
            year,
            genre: genre.map(String::from),
            series: None,
            play_mode: None,
            rating,
            rating_votes: None,
            description: None,
            notes: None,
            source: None,
            application_path: None,
            dosbox_conf: None,
            status: None,
            region: None,
            max_players: None,
            language: "EN".into(),
            shortcode: None,
            available_languages: None,
            variant_titles: None,
            torrent_source: Some("eXoDOS".into()),
            in_library: false,
            installed: false,
            favorited: false,
            game_torrent_index: None,
            gamedata_torrent_index: None,
            download_size: None,
            has_thumbnail: false,
            dosbox_variant: None,
            thumbnail_key: None,
            manual_path: None,
            last_played: None,
            music_file: None,
            requires_base: false,
            installed_with: None,
        }
    }

    #[test]
    fn section_keys_follow_the_sort() {
        assert_eq!(group_key(&game("alpha", None, None, None), "title"), "A");
        assert_eq!(group_key(&game("1942", None, None, None), "title_desc"), "#");
        assert_eq!(group_key(&game("x", Some(1991), None, None), "year_asc"), "1991");
        assert_eq!(group_key(&game("x", None, None, None), "year_desc"), "Unknown");
        assert_eq!(group_key(&game("x", None, Some(4.4), None), "rating"), "★★★★☆");
        assert_eq!(group_key(&game("x", None, None, None), "rating"), "Unrated");
        assert_eq!(group_key(&game("x", None, None, Some("Sports / Baseball; Action")), "genre"), "Sports");
        assert_eq!(group_key(&game("x", None, None, None), "genre"), "Unknown");
        assert_eq!(group_key(&game("x", None, None, None), "developer"), "");
    }

    #[test]
    fn sort_options_match_the_backend_keys() {
        let keys: Vec<&str> = SORT_OPTIONS.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, ["title", "title_desc", "year_desc", "year_asc", "rating", "genre"]);
    }
}
