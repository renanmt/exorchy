//! The library's left sidebar (the concept's navigation): All Games, then
//! the categories to browse by. Most categories list their values with
//! counts on a page of their own (`values`, which the library shows in place
//! of the grid); picking a value narrows the grid to it. Favorites filters
//! directly. The sidebar knows nothing of the grid: it reports picks.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use exorchy_core::commands::{games, setup};
use gtk::glib;

use crate::app;
use crate::ui::statusbar::grouped;

/// The width of the app's left columns: this sidebar and Settings'
/// navigation are the same bar, so they share it.
pub const SIDEBAR_WIDTH: i32 = 280;

/// A category the sidebar browses by.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Platforms,
    Genres,
    Publishers,
    Series,
    Years,
    Regions,
    Tags,
    Status,
}

impl Category {
    fn facet(self) -> Option<&'static str> {
        Some(match self {
            Category::Platforms => "collection",
            Category::Genres => "genre",
            Category::Publishers => "publisher",
            Category::Series => "series",
            Category::Years => "year",
            Category::Regions => "region",
            Category::Tags => "tag",
            Category::Status => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::Platforms => "Platforms",
            Category::Genres => "Genres",
            Category::Publishers => "Publishers",
            Category::Series => "Series",
            Category::Years => "Years",
            Category::Regions => "Regions",
            Category::Tags => "Tags",
            Category::Status => "Play Status",
        }
    }

    /// The singular, for the filter bar's chip ("Publisher: Sierra").
    pub fn noun(self) -> &'static str {
        match self {
            Category::Platforms => "Platform",
            Category::Genres => "Genre",
            Category::Publishers => "Publisher",
            Category::Series => "Series",
            Category::Years => "Year",
            Category::Regions => "Region",
            Category::Tags => "Tag",
            Category::Status => "Status",
        }
    }
}

/// The play-status values: (value the backend filters on, label).
pub const STATUS: [(&str, &str); 4] = [("installed", "Installed"), ("library", "In your library"), ("available", "Not downloaded"), ("played", "Played")];

/// What the user picked.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    All,
    Favorites,
    /// `value` is what the backend filters on (a collection id, a status
    /// key); `label` what the chip shows.
    Value { category: Category, value: String, label: String },
}

pub struct Sidebar {
    pub nav: gtk::Box,
    /// The values page; the library shows it in place of the grid.
    pub values: gtk::Box,
    all_count: gtk::Label,
    buttons: RefCell<Vec<(Option<Category>, bool, gtk::Button)>>,
    title: gtk::Label,
    list: gtk::StringList,
    filter_entry: gtk::SearchEntry,
    /// Shown label -> (value, count) for the category on the values page.
    entries: RefCell<HashMap<String, (String, usize)>>,
    showing: Cell<Option<Category>>,
    generation: Cell<u64>,
    on_pick: Rc<dyn Fn(Pick)>,
    on_values: Rc<dyn Fn()>,
}

pub fn build(on_pick: impl Fn(Pick) + 'static, on_values: impl Fn() + 'static) -> Rc<Sidebar> {
    // Fixed width (Settings' navigation has the same): the rows' labels
    // expand inside it, and without an explicit
    // `hexpand(false)` that would spread to the sidebar and split a wide
    // window's spare room with the grid.
    let nav = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).width_request(SIDEBAR_WIDTH).hexpand(false).css_classes(["sidebar"]).build();

    // Values page: a heading, a filter field, the virtualised list.
    let values = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["sidebar-values"]).build();
    let head = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(10).build();
    let title = gtk::Label::builder().xalign(0.0).hexpand(true).css_classes(["sidebar-values-title"]).build();
    let filter_entry = gtk::SearchEntry::builder().placeholder_text("Filter…").css_classes(["search"]).width_chars(22).build();
    head.append(&title);
    head.append(&filter_entry);
    values.append(&head);
    let list = gtk::StringList::new(&[]);
    let expression = gtk::PropertyExpression::new(gtk::StringObject::static_type(), None::<&gtk::Expression>, "string");
    let string_filter = gtk::StringFilter::builder().expression(&expression).ignore_case(true).match_mode(gtk::StringFilterMatchMode::Substring).build();
    let filtered = gtk::FilterListModel::new(Some(list.clone()), Some(string_filter.clone()));
    let selection = gtk::NoSelection::new(Some(filtered));
    let factory = gtk::SignalListItemFactory::new();
    let view = gtk::ListView::builder().model(&selection).factory(&factory).single_click_activate(true).css_classes(["sidebar-value-list"]).build();
    let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&view).build();
    values.append(&scroller);
    filter_entry.connect_search_changed(glib::clone!(#[weak] string_filter, move |e| string_filter.set_search(Some(&e.text()))));

    let sidebar = Rc::new(Sidebar {
        nav,
        values,
        all_count: gtk::Label::builder().css_classes(["sidebar-count"]).build(),
        buttons: RefCell::new(Vec::new()),
        title,
        list,
        filter_entry,
        entries: RefCell::new(HashMap::new()),
        showing: Cell::new(None),
        generation: Cell::new(0),
        on_pick: Rc::new(on_pick),
        on_values: Rc::new(on_values),
    });

    // Rows: the value, its count on the right.
    let weak = Rc::downgrade(&sidebar);
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
        let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(10).css_classes(["sidebar-value"]).build();
        row.append(&gtk::Label::builder().xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
        row.append(&gtk::Label::builder().css_classes(["sidebar-count"]).build());
        item.set_child(Some(&row));
    });
    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
        let (Some(row), Some(obj)) = (item.child(), item.item().and_downcast::<gtk::StringObject>()) else { return };
        let label = obj.string().to_string();
        let count = weak.upgrade().and_then(|s| s.entries.borrow().get(&label).map(|e| e.1)).unwrap_or(0);
        if let Some(name) = row.first_child().and_downcast::<gtk::Label>() {
            name.set_label(&label);
        }
        if let Some(n) = row.last_child().and_downcast::<gtk::Label>() {
            n.set_label(&if count > 0 { grouped(count) } else { String::new() });
        }
    });
    let weak = Rc::downgrade(&sidebar);
    view.connect_activate(move |v, pos| {
        let Some(s) = weak.upgrade() else { return };
        let Some(label) = v.model().and_then(|m| m.item(pos)).and_downcast::<gtk::StringObject>().map(|o| o.string().to_string()) else { return };
        let (Some(category), Some((value, _))) = (s.showing.get(), s.entries.borrow().get(&label).cloned()) else { return };
        (s.on_pick)(Pick::Value { category, value, label });
    });

    // Navigation.
    let entries: [(Option<Category>, bool, &str, &str); 10] = [
        (None, false, "view-grid-symbolic", "All Games"),
        (Some(Category::Platforms), false, "computer-symbolic", "Platforms"),
        (Some(Category::Genres), false, "folder-symbolic", "Genres"),
        (Some(Category::Publishers), false, "system-users-symbolic", "Publishers"),
        (Some(Category::Series), false, "view-list-symbolic", "Series"),
        (Some(Category::Years), false, "x-office-calendar-symbolic", "Years"),
        (Some(Category::Regions), false, "mark-location-symbolic", "Regions"),
        (Some(Category::Tags), false, "bookmark-new-symbolic", "Tags"),
        (Some(Category::Status), false, "object-select-symbolic", "Play Status"),
        (None, true, "starred-symbolic", "Favorites"),
    ];
    for (category, favorites, icon, label) in entries {
        let inner = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(10).build();
        inner.append(&gtk::Image::from_icon_name(icon));
        inner.append(&gtk::Label::builder().label(label).xalign(0.0).hexpand(true).build());
        if category.is_none() && !favorites {
            inner.append(&sidebar.all_count);
        }
        let b = gtk::Button::builder().child(&inner).css_classes(["sidebar-item"]).build();
        let weak = Rc::downgrade(&sidebar);
        b.connect_clicked(move |_| {
            let Some(s) = weak.upgrade() else { return };
            match (category, favorites) {
                (Some(c), _) => s.show_category(c),
                (None, true) => {
                    s.set_active(None, true);
                    (s.on_pick)(Pick::Favorites);
                }
                (None, false) => {
                    s.set_active(None, false);
                    (s.on_pick)(Pick::All);
                }
            }
        });
        sidebar.nav.append(&b);
        sidebar.buttons.borrow_mut().push((category, favorites, b));
    }
    sidebar.set_active(None, false);
    sidebar
}

impl Sidebar {
    /// The total next to All Games.
    pub fn set_total(&self, n: usize) {
        self.all_count.set_label(&grouped(n));
    }

    /// Highlight a category (None + !favorites = All Games).
    pub fn set_active(&self, category: Option<Category>, favorites: bool) {
        for (c, f, b) in self.buttons.borrow().iter() {
            if *c == category && *f == favorites {
                b.add_css_class("active");
            } else {
                b.remove_css_class("active");
            }
        }
    }

    /// Load a category's values onto the values page and ask to show it.
    pub fn show_category(self: &Rc<Self>, category: Category) {
        self.set_active(Some(category), false);
        self.showing.set(Some(category));
        self.title.set_label(category.label());
        self.filter_entry.set_text("");
        (self.on_values)();
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        if category == Category::Status {
            let rows: Vec<(String, String, usize)> = STATUS.iter().map(|(v, l)| (l.to_string(), v.to_string(), 0)).collect();
            self.fill(rows);
            return;
        }
        let Some(facet) = category.facet() else { return };
        let core = app::core();
        let weak = Rc::downgrade(self);
        app::spawn(
            async move {
                let values = games::get_facet_values(core.state(), facet.to_string()).await.unwrap_or_default();
                // Platforms are collections: show their names, filter on ids.
                let names: HashMap<String, String> = if facet == "collection" {
                    setup::get_available_collections(core.state()).await.unwrap_or_default().into_iter().map(|c| (c.id, c.display_name)).collect()
                } else {
                    HashMap::new()
                };
                (values, names)
            },
            move |(values, names)| {
                let Some(s) = weak.upgrade() else { return };
                if s.generation.get() != generation {
                    return;
                }
                let rows = values
                    .into_iter()
                    .map(|v| (names.get(&v.value).cloned().unwrap_or_else(|| v.value.clone()), v.value, v.count))
                    .collect();
                s.fill(rows);
            },
        );
    }

    /// (label, value, count) rows onto the values page.
    fn fill(&self, rows: Vec<(String, String, usize)>) {
        let mut map = HashMap::new();
        let labels: Vec<String> = rows
            .into_iter()
            .map(|(label, value, count)| {
                map.insert(label.clone(), (value, count));
                label
            })
            .collect();
        self.title.set_label(&format!("{}  ·  {}", self.showing.get().map(Category::label).unwrap_or(""), grouped(labels.len())));
        self.entries.replace(map);
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        self.list.splice(0, self.list.n_items(), &refs);
    }
}
