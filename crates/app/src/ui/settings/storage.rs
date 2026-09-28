//! Storage: disk usage by what it is for, plus every installed game by
//! size - the Steam storage manager's shape. A measurement survives
//! switching sections and reopening Settings; Refresh, every action here
//! and a library change re-measure.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use exorchy_core::commands::archives::{self, ArchiveUsage};
use exorchy_core::commands::storage::{self as backend, Category, GameStorage, StorageOverview};
use exorchy_core::commands::{games, storage::Freed};

use super::widgets::{self, Ctx, Row};
use super::{last_played_label, plural};
use crate::app;
use crate::ui::util::{format_bytes, platform_tag};
use crate::ui::{actions, bus};

/// Order, wording and colour class per category. The bar paints them in
/// this order, so the big ones (games, packs) come first.
struct Meta {
    id: Category,
    css: &'static str,
    name: &'static str,
    hint: &'static str,
    unit: Option<&'static str>,
}

const CATEGORIES: [Meta; 11] = [
    Meta { id: Category::Games, css: "cat-games", name: "Games", hint: "Unpacked game folders", unit: Some("game") },
    Meta { id: Category::Archives, css: "cat-archives", name: "Game archives", hint: "Downloaded ZIPs kept beside unpacked games", unit: Some("archive") },
    Meta { id: Category::Extras, css: "cat-extras", name: "Extras", hint: "Videos, manuals and music that DOS games share", unit: Some("archive") },
    Meta { id: Category::Packs, css: "cat-packs", name: "Content packs", hint: "Covers, screenshots, manuals, emulators", unit: None },
    Meta { id: Category::PackArchives, css: "cat-pack-archives", name: "Pack archives", hint: "Downloaded pack ZIPs from the torrents", unit: Some("archive") },
    Meta { id: Category::Support, css: "cat-support", name: "Emulators & support", hint: "eXo's own emulator builds, Windows 9x system images, ScummVM ROMs", unit: None },
    Meta { id: Category::Reading, css: "cat-reading", name: "Reading room", hint: "Magazines, books and catalogs - remove a document from its context menu in the reading room", unit: Some("document") },
    Meta { id: Category::Saves, css: "cat-saves", name: "Save backups", hint: "What uninstalls set aside. Delete skips backups of games in your library and backups that hold a whole game folder.", unit: Some("backup") },
    Meta { id: Category::Caches, css: "cat-caches", name: "Media caches", hint: "Preview videos, theme music, gallery thumbnails", unit: None },
    Meta { id: Category::Configs, css: "cat-configs", name: "Launch configs", hint: "Per-game DOSBox and emulator configs", unit: None },
    Meta { id: Category::Other, css: "cat-other", name: "Other", hint: "Torrent placeholders and pieces shared with neighbouring games", unit: None },
];

/// An open page's re-render, dropped once Settings closed.
type Renderer = (Rc<Cell<bool>>, Rc<dyn Fn()>);

/// Resolution of the usage bar: columns of a homogeneous grid.
const BAR_COLUMNS: i32 = 400;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sort {
    Size,
    Name,
    Played,
}

struct Cache {
    overview: Option<StorageOverview>,
    games: Vec<GameStorage>,
    archives: Option<ArchiveUsage>,
    keep_archives: bool,
    error: String,
    loading: bool,
    /// The library generation the measurement was taken at (-1: never).
    measured_at: i64,
    /// Bumped by every library change (installs, uninstalls elsewhere).
    lib_gen: i64,
    /// Bumped by every uninstall here: a game list read before it is stale.
    list_gen: u64,
    walking: bool,
    walk_again: bool,
    listener_armed: bool,
    /// Open pages that re-render when a measurement lands.
    renderers: Vec<Renderer>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache {
        overview: None, games: Vec::new(), archives: None, keep_archives: true, error: String::new(),
        loading: false, measured_at: -1, lib_gen: 0, list_gen: 0, walking: false, walk_again: false,
        listener_armed: false, renderers: Vec::new(),
    });
}

/// A new game folder or a factory reset: nothing measured still applies.
pub fn reset_cache() {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.overview = None;
        c.games.clear();
        c.archives = None;
        c.measured_at = -1;
        c.list_gen += 1;
    });
}

fn arm_listener() {
    let armed = CACHE.with(|c| std::mem::replace(&mut c.borrow_mut().listener_armed, true));
    if !armed {
        bus::on_library_changed(|_| CACHE.with(|c| c.borrow_mut().lib_gen += 1));
    }
}

fn rerender() {
    let live: Vec<Rc<dyn Fn()>> = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.renderers.retain(|(alive, _)| alive.get());
        c.renderers.iter().map(|(_, f)| f.clone()).collect()
    });
    for f in live {
        f();
    }
}

/// Measure the folder, then read the game list - in that order, so the
/// list is at most seconds old when it lands, and never older than an
/// uninstall made while the walk ran (`list_gen`).
fn load() {
    let start = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.walking {
            c.walk_again = true;
            return false;
        }
        c.walking = true;
        c.loading = true;
        c.error.clear();
        true
    });
    if !start {
        return;
    }
    rerender();
    app::local(async move {
        loop {
            let stamp = CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.walk_again = false;
                (c.lib_gen, c.list_gen)
            });
            let core = app::core();
            let first = app::call(async move {
                let o = backend::storage_overview(core.state()).await?;
                let a = archives::archive_usage(core.state()).await?;
                let keep = games::get_config(core.state(), "keep_archives".into()).await.ok().flatten();
                Ok::<_, String>((o, a, keep))
            })
            .await;
            let (o, a, keep) = match first {
                Ok(v) => v,
                Err(e) => {
                    CACHE.with(|c| c.borrow_mut().error = e);
                    break;
                }
            };
            let core = app::core();
            let games = app::call(async move { backend::installed_games_storage(core.state()).await }).await;
            let again = CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.overview = Some(o);
                c.archives = Some(a);
                c.keep_archives = keep.as_deref() != Some("0");
                match games {
                    Ok(g) if c.list_gen == stamp.1 => c.games = g,
                    Ok(_) => c.walk_again = true,
                    Err(e) => c.error = e,
                }
                c.measured_at = stamp.0;
                c.walk_again
            });
            if !again {
                break;
            }
        }
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            c.walking = false;
            c.loading = false;
        });
        rerender();
    });
}

/// Page state that belongs to the open Settings page, not the measurement.
struct View {
    sort: Cell<Sort>,
    /// Two-click destructive actions: the first click arms the label.
    armed: RefCell<Option<String>>,
    busy: RefCell<Option<String>>,
    uninstalling: RefCell<Vec<i64>>,
}

/// Build the page; the second value is called when the section comes into view.
pub fn build(ctx: &Ctx) -> (gtk::Widget, Rc<dyn Fn()>) {
    arm_listener();
    let page = widgets::page("Storage");
    let groups: Rc<RefCell<Vec<gtk::Widget>>> = Rc::new(RefCell::new(Vec::new()));
    let view = Rc::new(View { sort: Cell::new(Sort::Size), armed: RefCell::new(None), busy: RefCell::new(None), uninstalling: RefCell::new(Vec::new()) });

    let render: Rc<dyn Fn()> = {
        let (page, groups, view, ctx) = (page.clone(), groups.clone(), view.clone(), ctx.clone());
        Rc::new(move || render_page(&ctx, &page, &groups, &view))
    };
    CACHE.with(|c| c.borrow_mut().renderers.push((ctx.alive.clone(), render.clone())));
    render();

    // The walk takes seconds on a large library: once per visit, and again
    // only after the library changed.
    let shown: Rc<dyn Fn()> = {
        let (view, render) = (view.clone(), render.clone());
        Rc::new(move || {
            *view.armed.borrow_mut() = None;
            let stale = CACHE.with(|c| {
                let c = c.borrow();
                !c.loading && (c.overview.is_none() || c.measured_at != c.lib_gen)
            });
            if stale {
                load();
            } else {
                render();
            }
        })
    };
    (page.upcast(), shown)
}

fn render_page(ctx: &Ctx, page: &widgets::Page, groups: &Rc<RefCell<Vec<gtk::Widget>>>, view: &Rc<View>) {
    for g in groups.borrow_mut().drain(..) {
        page.remove(&g);
    }
    let add = |g: &adw::PreferencesGroup| {
        page.add(g);
        groups.borrow_mut().push(g.clone().upcast());
    };
    let (overview, games, archives, keep, error, loading) = CACHE.with(|c| {
        let c = c.borrow();
        (c.overview.clone(), c.games.clone(), c.archives, c.keep_archives, c.error.clone(), c.loading)
    });

    if !error.is_empty() {
        let g = widgets::group("Storage", None);
        let l = widgets::note(&error);
        l.add_css_class("error");
        g.add(&l);
        add(&g);
    }
    if loading && overview.is_none() {
        let g = widgets::group("Storage", None);
        let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(10).build();
        row.append(&gtk::Spinner::builder().spinning(true).build());
        row.append(&widgets::note("Measuring the game folder - up to a minute on a large library…"));
        g.add(&row);
        add(&g);
    }
    let Some(o) = overview else { return };

    // ── The drive ──
    let drive = widgets::group("", None);
    let refresh = widgets::BusyButton::new("Refresh", "Measuring…");
    refresh.set_busy(loading);
    refresh.widget.connect_clicked(|_| load());
    let head = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).build();
    let path = gtk::Label::builder().label(&o.folder).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::Middle).tooltip_text(&o.folder).css_classes(["setting-code"]).build();
    head.append(&path);
    head.append(&refresh.widget);
    let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["storage-drive"]).build();
    card.append(&head);
    card.append(&usage_bar(&o));
    // Wraps in a narrow dialog instead of setting its minimum width.
    let line = adw::WrapBox::builder().child_spacing(16).line_spacing(4).css_classes(["storage-drive-line"]).build();
    let seg = |markup: String| gtk::Label::builder().use_markup(true).label(&markup).xalign(0.0).build();
    line.append(&seg(format!("<b>{}</b> used by eXorchy", format_bytes(o.used_bytes))));
    line.append(&seg(format!("{} other files", format_bytes(o.other_bytes))));
    let free = seg(format!("<b>{}</b> free of {}", format_bytes(o.free_bytes), format_bytes(o.total_bytes)));
    free.set_hexpand(true);
    free.set_xalign(1.0);
    line.append(&free);
    card.append(&line);
    drive.add(&card);
    add(&drive);

    // ── What takes the space ──
    let what = widgets::group("What takes the space", None);
    for meta in &CATEGORIES {
        let Some(cat) = o.categories.iter().find(|c| c.id == meta.id) else { continue };
        if cat.bytes == 0 && !matches!(meta.id, Category::Games | Category::Archives) {
            continue;
        }
        let dot = gtk::Box::builder().css_classes(["storage-dot", meta.css]).valign(gtk::Align::Center).build();
        let row = Row::with_prefix(Some(&dot), meta.name);
        let mut value = format_bytes(cat.bytes);
        if let Some(unit) = meta.unit {
            if cat.items > 0 {
                value = format!("{value} · {}", plural(cat.items as i64, unit));
            }
        }
        row.set_value(&value);
        match meta.id {
            Category::Archives => {
                let installed = archives.unwrap_or_default();
                let mut hint = if keep {
                    "Kept after install so Reset needs no download and games can be seeded.".to_string()
                } else {
                    "Not kept after install - Reset downloads the game again, and those games cannot be seeded.".to_string()
                };
                if installed.count > 0 {
                    hint.push_str(&format!(" Remove takes the {} of installed games ({}).", plural(installed.count as i64, "archive"), format_bytes(installed.bytes)));
                }
                row.set_hint(&hint);
                let sw = widgets::Switch::new(keep);
                sw.on_change({
                    let sw = sw.clone();
                    move |next| {
                        CACHE.with(|c| c.borrow_mut().keep_archives = next);
                        let core = app::core();
                        let sw = sw.clone();
                        let v = if next { "1" } else { "0" }.to_string();
                        app::spawn(async move { games::set_config(core.clone(), core.state(), "keep_archives".into(), v).await }, move |res| {
                            if let Err(e) = res {
                                log::error!("settings: failed to save keep_archives: {e}");
                                CACHE.with(|c| c.borrow_mut().keep_archives = !next);
                                sw.set_quiet(!next);
                            }
                        });
                    }
                });
                row.add_action(&sw.widget);
                row.activates(&sw.widget);
                if installed.count > 0 {
                    row.add_action(&armed_button(ctx, view, "archives", "Remove", &format!("Free {}?", format_bytes(installed.bytes)), "Removing…", Action::Archives));
                }
            }
            Category::Saves if cat.bytes > 0 => {
                row.set_hint(meta.hint);
                row.add_action(&armed_button(ctx, view, "saves", "Delete", &format!("Delete {} of backups?", format_bytes(cat.bytes)), "Deleting…", Action::Saves));
            }
            Category::Caches if cat.bytes > 0 => {
                row.set_hint(meta.hint);
                row.add_action(&armed_button(ctx, view, "caches", "Clear", "Clear caches?", "Clearing…", Action::Caches));
            }
            Category::Packs => {
                row.set_hint(meta.hint);
                let manage = widgets::button("Manage");
                let ctx2 = ctx.clone();
                manage.connect_clicked(move |_| ctx2.go("packs"));
                row.add_action(&manage);
            }
            _ => row.set_hint(meta.hint),
        }
        what.add(&row.widget);
    }
    add(&what);

    // ── Installed games ──
    let installed = widgets::group(&format!("Installed games · {}", games.len()), None);
    let sort_box = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).build();
    sort_box.append(&widgets::note("Sort by"));
    let drop = gtk::DropDown::from_strings(&["Size", "Name", "Last played"]);
    drop.add_css_class("drop");
    drop.set_selected(match view.sort.get() {
        Sort::Size => 0,
        Sort::Name => 1,
        Sort::Played => 2,
    });
    drop.connect_selected_notify({
        let view = view.clone();
        move |d| {
            view.sort.set(match d.selected() {
                1 => Sort::Name,
                2 => Sort::Played,
                _ => Sort::Size,
            });
            rerender();
        }
    });
    sort_box.append(&drop);
    installed.set_header_suffix(Some(&sort_box));
    if games.is_empty() {
        installed.add(&widgets::note("No games installed."));
    }
    for g in sorted_games(games, view.sort.get()) {
        installed.add(&game_row(ctx, view, g));
    }
    add(&installed);
}

/// The bar: a homogeneous grid, each category spanning its share of the
/// columns, the rest of the disk in the foreign colour.
fn usage_bar(o: &StorageOverview) -> gtk::Grid {
    let grid = gtk::Grid::builder().column_homogeneous(true).hexpand(true).css_classes(["storage-bar"]).build();
    let total = o.total_bytes.max(1) as f64;
    let mut col = 0;
    let mut place = |bytes: u64, classes: &[&str], tip: String| {
        let span = ((bytes as f64 / total) * BAR_COLUMNS as f64).round() as i32;
        let span = span.min(BAR_COLUMNS - col);
        if span <= 0 {
            return;
        }
        let seg = gtk::Box::builder().tooltip_text(&tip).build();
        seg.add_css_class("storage-seg");
        for c in classes {
            seg.add_css_class(c);
        }
        grid.attach(&seg, col, 0, span, 1);
        col += span;
    };
    for meta in &CATEGORIES {
        if let Some(cat) = o.categories.iter().find(|c| c.id == meta.id) {
            if cat.bytes > 0 {
                place(cat.bytes, &[meta.css], format!("{} · {}", meta.name, format_bytes(cat.bytes)));
            }
        }
    }
    place(o.other_bytes, &["is-foreign"], format!("Other files on this disk · {}", format_bytes(o.other_bytes)));
    // The free space keeps the grid at full width.
    if col < BAR_COLUMNS {
        let rest = gtk::Box::builder().css_classes(["storage-seg", "is-free"]).build();
        grid.attach(&rest, col, 0, BAR_COLUMNS - col, 1);
    }
    grid
}

fn total_of(g: &GameStorage) -> u64 {
    g.game_bytes + g.archive_bytes + g.save_bytes
}

fn sorted_games(mut list: Vec<GameStorage>, sort: Sort) -> Vec<GameStorage> {
    match sort {
        Sort::Name => list.sort_by_key(|a| a.title.to_lowercase()),
        Sort::Played => list.sort_by(|a, b| b.last_played.as_deref().unwrap_or("").cmp(a.last_played.as_deref().unwrap_or(""))),
        Sort::Size => list.sort_by_key(|a| std::cmp::Reverse(total_of(a))),
    }
    list
}

fn game_row(ctx: &Ctx, view: &Rc<View>, g: GameStorage) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::builder().activatable(false).selectable(false).css_classes(["settings-row", "storage-game"]).build();
    let line = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).build();
    let main = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
    main.append(&gtk::Label::builder().label(&g.title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["settings-row-label"]).build());
    let meta = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(6).build();
    if let Some(tag) = platform_tag(Some(&g.collection)) {
        meta.append(&gtk::Label::builder().label(tag).css_classes(["badge"]).build());
    }
    if !g.language.is_empty() {
        meta.append(&gtk::Label::builder().label(&g.language).css_classes(["badge"]).build());
    }
    meta.append(&gtk::Label::builder().label(last_played_label(g.last_played.as_deref())).css_classes(["settings-row-hint"]).build());
    main.append(&meta);
    line.append(&main);

    let size = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).halign(gtk::Align::End).valign(gtk::Align::Center).build();
    size.append(&gtk::Label::builder().label(format_bytes(total_of(&g))).xalign(1.0).css_classes(["settings-row-value"]).build());
    if g.archive_bytes > 0 || g.save_bytes > 0 {
        let mut parts = vec![format!("game {}", format_bytes(g.game_bytes))];
        if g.archive_bytes > 0 {
            parts.push(format!("archive {}", format_bytes(g.archive_bytes)));
        }
        if g.save_bytes > 0 {
            parts.push(format!("saves {}", format_bytes(g.save_bytes)));
        }
        size.append(&gtk::Label::builder().label(parts.join(" · ")).xalign(1.0).wrap(true).css_classes(["settings-row-hint"]).build());
    }
    line.append(&size);

    let key = format!("game:{}", g.id);
    let uninstalling = view.uninstalling.borrow().contains(&g.id);
    let btn = widgets::BusyButton::new(if view.armed.borrow().as_deref() == Some(&key) { "Confirm uninstall?" } else { "Uninstall" }, "Uninstalling…");
    btn.set_busy(uninstalling);
    btn.widget.connect_clicked({
        let (view, alive) = (view.clone(), ctx.alive.clone());
        move |_| {
            if view.armed.borrow().as_deref() != Some(&key) {
                *view.armed.borrow_mut() = Some(key.clone());
                rerender();
                return;
            }
            *view.armed.borrow_mut() = None;
            view.uninstalling.borrow_mut().push(g.id);
            rerender();
            let (view, id, alive) = (view.clone(), g.id, alive.clone());
            let status: Rc<dyn Fn(&str)> = Rc::new(|_| {});
            let done: Rc<dyn Fn(bool)> = Rc::new(move |ok| {
                view.uninstalling.borrow_mut().retain(|x| *x != id);
                if ok {
                    CACHE.with(|c| {
                        let mut c = c.borrow_mut();
                        c.list_gen += 1;
                        c.games.retain(|x| x.id != id);
                    });
                }
                if alive.get() {
                    rerender();
                }
                load();
            });
            actions::uninstall(id, g.title.clone(), status, done);
        }
    });
    line.append(&btn.widget);
    // A narrow dialog stacks title, size and button like the other rows.
    widgets::follow_narrow(&line, move |line, narrow| {
        line.set_orientation(if narrow { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal });
        size.set_halign(if narrow { gtk::Align::Start } else { gtk::Align::End });
        btn.widget.set_halign(if narrow { gtk::Align::Start } else { gtk::Align::Fill });
    });
    row.set_child(Some(&line));
    row
}

#[derive(Clone, Copy)]
enum Action {
    Archives,
    Saves,
    Caches,
}

/// A destructive button: the first click arms the label, the second runs
/// the action; the toast says what was freed and the folder is re-measured.
fn armed_button(ctx: &Ctx, view: &Rc<View>, key: &str, idle: &str, armed_label: &str, busy_label: &str, action: Action) -> gtk::Button {
    let armed = view.armed.borrow().as_deref() == Some(key);
    let btn = widgets::BusyButton::new(if armed { armed_label } else { idle }, busy_label);
    btn.set_busy(view.busy.borrow().as_deref() == Some(key));
    let key = key.to_string();
    btn.widget.connect_clicked({
        let (view, ctx) = (view.clone(), ctx.clone());
        move |_| {
            if view.armed.borrow().as_deref() != Some(&key) {
                *view.armed.borrow_mut() = Some(key.clone());
                rerender();
                return;
            }
            *view.armed.borrow_mut() = None;
            *view.busy.borrow_mut() = Some(key.clone());
            rerender();
            let core = app::core();
            let (view, key2) = (view.clone(), key.clone());
            let _ = &ctx;
            app::spawn(
                async move {
                    match action {
                        Action::Archives => archives::remove_installed_archives(core.state(), core.state()).await.map(|r| Freed { items: r.count as u64, bytes: r.bytes }),
                        Action::Saves => backend::delete_save_backups(core.state()).await,
                        Action::Caches => backend::clear_media_caches(core.state()).await,
                    }
                },
                move |res| {
                    match res {
                        Ok(r) => bus::toast(&match action {
                            Action::Archives => format!("Removed {} · {} freed", plural(r.items as i64, "archive"), format_bytes(r.bytes)),
                            Action::Saves => format!("Deleted {} · {} freed", plural(r.items as i64, "backup"), format_bytes(r.bytes)),
                            Action::Caches => format!("Cleared {} of cached media", format_bytes(r.bytes)),
                        }),
                        Err(e) => bus::toast_with("Couldn't free the space", Some(&e), None),
                    }
                    // The button is done when the action is; the re-measure
                    // that follows takes seconds and shows on Refresh instead.
                    if view.busy.borrow().as_deref() == Some(&key2) {
                        *view.busy.borrow_mut() = None;
                    }
                    load();
                },
            );
        }
    });
    btn.widget
}
