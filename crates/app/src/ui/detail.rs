//! The game detail panel: cover, title, language variants (one selected row
//! at a time, so a merged card can play the German version), the action bar
//! (Play / Stop / Download / Cancel, favourite, more), the status line,
//! description, information table, gallery and manual.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use exorchy_core::commands::{assets, games};
use exorchy_core::models::Game;
use gtk::glib;
use gtk::prelude::*;

use crate::app;
use crate::ui::util::{esc, format_bytes, platform_tag};
use crate::ui::{actions, bus, covers, downloads};

pub const PANEL_WIDTH: i32 = 560;

pub struct DetailPanel {
    pub widget: gtk::Revealer,
    window: gtk::Window,
    game: RefCell<Option<Game>>,
    variants: RefCell<Vec<Game>>,
    selected: Cell<Option<i64>>,
    generation: Cell<u64>,
    busy: RefCell<Option<String>>,
    launching: Cell<bool>,
    cover: gtk::Picture,
    title: gtk::Label,
    subtitle: gtk::Label,
    chips: gtk::Box,
    actions: gtk::Box,
    status: gtk::Label,
    description: gtk::Label,
    notes: gtk::Label,
    info: gtk::Grid,
    gallery: gtk::FlowBox,
    gallery_head: gtk::Label,
    manual_path: RefCell<Option<String>>,
    scroller: gtk::ScrolledWindow,
    /// Below the cover; `ui::media` fills it.
    pub media_slot: gtk::Box,
    shown_listeners: RefCell<Vec<Rc<dyn Fn(Option<&Game>)>>>,
}

impl DetailPanel {
    pub fn new(window: &gtk::Window) -> Rc<Self> {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).width_request(PANEL_WIDTH).css_classes(["detail-panel"]).build();

        let head = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).css_classes(["detail-head"]).build();
        let close = gtk::Button::builder().icon_name("window-close-symbolic").css_classes(["btn", "icon", "ghost"]).tooltip_text("Close (Esc)").build();
        let title = gtk::Label::builder().xalign(0.0).wrap(true).hexpand(true).css_classes(["title-2"]).selectable(true).build();
        head.append(&title);
        head.append(&close);
        root.append(&head);

        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).margin_start(16).margin_end(16).margin_bottom(20).build();
        let cover = gtk::Picture::builder().content_fit(gtk::ContentFit::Contain).height_request(280).can_shrink(true).css_classes(["detail-cover"]).build();
        body.append(&cover);
        // The media module (preview video, theme music) mounts here.
        let media_slot = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        body.append(&media_slot);
        let subtitle = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["secondary"]).build();
        body.append(&subtitle);
        let chips = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(6).visible(false).build();
        body.append(&chips);
        let actions = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).css_classes(["detail-actions"]).build();
        body.append(&actions);
        let status = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["muted", "small"]).visible(false).build();
        body.append(&status);
        let description = gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).css_classes(["detail-description"]).build();
        body.append(&description);
        let notes = gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).visible(false).css_classes(["muted", "small"]).build();
        body.append(&notes);
        let info = gtk::Grid::builder().row_spacing(4).column_spacing(16).css_classes(["detail-info"]).build();
        body.append(&info);
        let gallery_head = gtk::Label::builder().label("Screenshots").xalign(0.0).css_classes(["title-3"]).visible(false).build();
        body.append(&gallery_head);
        let gallery = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).column_spacing(6).row_spacing(6).min_children_per_line(3).max_children_per_line(6).homogeneous(true).build();
        body.append(&gallery);

        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&body).build();
        root.append(&scroller);

        let widget = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideLeft)
            .transition_duration(200)
            .reveal_child(false)
            .child(&root)
            .build();

        let panel = Rc::new(DetailPanel {
            widget,
            window: window.clone(),
            game: RefCell::new(None),
            variants: RefCell::new(Vec::new()),
            selected: Cell::new(None),
            generation: Cell::new(0),
            busy: RefCell::new(None),
            launching: Cell::new(false),
            cover,
            title,
            subtitle,
            chips,
            actions,
            status,
            description,
            notes,
            info,
            gallery,
            gallery_head,
            manual_path: RefCell::new(None),
            scroller,
            media_slot,
            shown_listeners: RefCell::new(Vec::new()),
        });
        close.connect_clicked(glib::clone!(#[weak] panel, move |_| panel.close()));
        panel
    }

    pub fn is_open(&self) -> bool {
        self.widget.reveals_child()
    }

    pub fn close(&self) {
        self.widget.set_reveal_child(false);
        self.notify_shown(None);
    }

    /// Called with the game when the panel opens on a (different) game and
    /// with `None` when it closes. The media module hangs its preview here.
    pub fn on_shown(&self, f: impl Fn(Option<&Game>) + 'static) {
        self.shown_listeners.borrow_mut().push(Rc::new(f));
    }

    fn notify_shown(&self, g: Option<&Game>) {
        let cbs = self.shown_listeners.borrow().clone();
        for cb in cbs {
            cb(g);
        }
    }

    pub fn current(&self) -> Option<Game> {
        self.game.borrow().clone()
    }

    pub fn show(self: &Rc<Self>, game: Game) {
        let same = self.game.borrow().as_ref().and_then(|g| g.id) == game.id && game.id.is_some();
        self.game.replace(Some(game.clone()));
        if !same {
            self.selected.set(game.id);
            self.variants.replace(vec![game.clone()]);
            self.scroller.vadjustment().set_value(0.0);
            self.load_cover(&game);
            self.load_variants(&game);
            self.load_metadata(&game);
        }
        self.render();
        self.widget.set_reveal_child(true);
        if !same {
            self.notify_shown(Some(&game));
        }
    }

    /// The library changed for `id`: re-read the row if it is ours.
    pub fn refresh_by_id(self: &Rc<Self>, id: i64) {
        let mine = self.game.borrow().as_ref().and_then(|g| g.id) == Some(id) || self.variants.borrow().iter().any(|v| v.id == Some(id));
        if !mine {
            return;
        }
        let Some(game) = self.game.borrow().clone() else { return };
        let core = app::core();
        let gid = game.id.unwrap_or(id);
        app::spawn(async move { games::get_game(core.state(), gid).await }, glib::clone!(#[weak(rename_to = panel)] self, move |res| {
            if let Ok(Some(fresh)) = res {
                if panel.game.borrow().as_ref().and_then(|g| g.id) == fresh.id {
                    panel.game.replace(Some(fresh.clone()));
                    panel.load_variants(&fresh);
                    panel.load_metadata(&fresh);
                    panel.render();
                }
            }
        }));
    }

    pub fn refresh_download(self: &Rc<Self>, id: i64) {
        let mine = self.variants.borrow().iter().any(|v| v.id == Some(id));
        if mine {
            self.render_actions();
        }
    }

    pub fn refresh_running(self: &Rc<Self>) {
        self.render_actions();
    }

    fn load_cover(self: &Rc<Self>, game: &Game) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.cover.set_paintable(gtk::gdk::Paintable::NONE);
        let panel = Rc::downgrade(self);
        covers::request(game.torrent_source.as_deref(), game.thumbnail_key.as_deref(), covers::Size::Fit(PANEL_WIDTH as u32 - 32, 320), move |t| {
            let Some(panel) = panel.upgrade() else { return };
            if panel.generation.get() != generation {
                return;
            }
            panel.cover.set_paintable(t.as_ref());
            panel.cover.set_visible(t.is_some());
        });
    }

    fn load_variants(self: &Rc<Self>, game: &Game) {
        let multi = game.available_languages.as_deref().map(|l| l.contains(',')).unwrap_or(false);
        let (Some(sc), Some(src)) = (game.shortcode.clone(), game.torrent_source.clone()) else { return };
        if !multi {
            return;
        }
        let core = app::core();
        let gid = game.id;
        app::spawn(async move { games::get_game_variants(core.state(), core.state(), sc, src).await }, glib::clone!(#[weak(rename_to = panel)] self, move |res| {
            if panel.game.borrow().as_ref().and_then(|g| g.id) != gid {
                return;
            }
            if let Ok(rows) = res {
                if !rows.is_empty() {
                    if !rows.iter().any(|r| r.id == panel.selected.get()) {
                        panel.selected.set(rows.iter().find(|r| r.installed).or(rows.first()).and_then(|r| r.id));
                    }
                    panel.variants.replace(rows);
                    panel.render();
                }
            }
        }));
    }

    fn load_metadata(self: &Rc<Self>, game: &Game) {
        self.gallery.remove_all();
        self.gallery_head.set_visible(false);
        self.manual_path.replace(None);
        let core = app::core();
        let (src, title, sc, manual, gid) = (
            game.torrent_source.clone().unwrap_or_else(|| "eXoDOS".into()),
            game.title.clone(),
            game.shortcode.clone(),
            game.manual_path.clone(),
            game.id,
        );
        app::spawn(
            async move { assets::get_game_metadata(core.state(), src, title, sc, manual).await },
            glib::clone!(#[weak(rename_to = panel)] self, move |res| {
                if panel.game.borrow().as_ref().and_then(|g| g.id) != gid {
                    return;
                }
                let Ok(meta) = res else { return };
                panel.manual_path.replace(meta.manual_path.clone());
                panel.gallery.remove_all();
                panel.gallery_head.set_visible(!meta.images.is_empty());
                for (thumb, full) in meta.thumbnails.iter().zip(meta.images.iter()) {
                    let pic = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).height_request(96).css_classes(["gallery-thumb"]).build();
                    let path = std::path::PathBuf::from(thumb);
                    let p2 = pic.clone();
                    let handle = exorchy_core::host::async_runtime::spawn_blocking(move || gtk::gdk::Texture::from_filename(&path).ok());
                    app::local(async move {
                        if let Ok(Some(t)) = handle.await {
                            p2.set_paintable(Some(&t));
                        }
                    });
                    let btn = gtk::Button::builder().child(&pic).css_classes(["gallery-btn"]).build();
                    let full = full.clone();
                    btn.connect_clicked(move |_| actions::open_document(full.clone()));
                    panel.gallery.insert(&btn, -1);
                }
                panel.render_actions();
            }),
        );
    }

    fn selected_row(&self) -> Option<Game> {
        let sel = self.selected.get();
        self.variants.borrow().iter().find(|v| v.id == sel).cloned().or_else(|| self.game.borrow().clone())
    }

    fn render(self: &Rc<Self>) {
        let Some(game) = self.game.borrow().clone() else { return };
        self.title.set_label(&game.title);
        let mut parts = Vec::new();
        if let Some(y) = game.year {
            parts.push(y.to_string());
        }
        if let Some(d) = game.developer.as_deref().filter(|s| !s.is_empty()) {
            parts.push(d.to_string());
        }
        if let Some(p) = platform_tag(game.torrent_source.as_deref()) {
            parts.push(p.to_string());
        }
        self.subtitle.set_label(&parts.join(" · "));

        // Language chips.
        while let Some(c) = self.chips.first_child() {
            self.chips.remove(&c);
        }
        let rows = self.variants.borrow().clone();
        self.chips.set_visible(rows.len() > 1);
        if rows.len() > 1 {
            let mut group: Option<gtk::ToggleButton> = None;
            for r in &rows {
                let label = format!("{}{}", r.language, if r.installed { " ✓" } else { "" });
                let b = gtk::ToggleButton::builder().label(&label).css_classes(["chip"]).active(r.id == self.selected.get()).build();
                if let Some(g) = &group {
                    b.set_group(Some(g));
                } else {
                    group = Some(b.clone());
                }
                let id = r.id;
                b.connect_toggled(glib::clone!(#[weak(rename_to = panel)] self, move |b| {
                    if b.is_active() && panel.selected.get() != id {
                        panel.selected.set(id);
                        panel.render();
                    }
                }));
                self.chips.append(&b);
            }
        }

        let row = self.selected_row().unwrap_or(game.clone());
        let desc = row.description.clone().or(game.description.clone()).unwrap_or_default();
        self.description.set_label(&desc);
        self.description.set_visible(!desc.is_empty());
        let notes = row.notes.clone().unwrap_or_default();
        self.notes.set_label(&notes);
        self.notes.set_visible(!notes.is_empty());

        // Information table.
        while let Some(c) = self.info.first_child() {
            self.info.remove(&c);
        }
        let mut r = 0;
        let mut add = |k: &str, v: Option<String>| {
            let Some(v) = v.filter(|v| !v.trim().is_empty()) else { return };
            let kl = gtk::Label::builder().label(k).xalign(0.0).css_classes(["muted", "small"]).valign(gtk::Align::Start).build();
            let vl = gtk::Label::builder().label(&v).xalign(0.0).wrap(true).selectable(true).hexpand(true).build();
            self.info.attach(&kl, 0, r, 1, 1);
            self.info.attach(&vl, 1, r, 1, 1);
            r += 1;
        };
        add("Developer", row.developer.clone());
        add("Publisher", row.publisher.clone());
        add("Released", row.release_date.clone().or(row.year.map(|y| y.to_string())));
        add("Genre", row.genre.clone());
        add("Series", row.series.clone());
        add("Play mode", row.play_mode.clone());
        add("Players", row.max_players.map(|n| n.to_string()));
        add("Rating", row.rating.map(|r| format!("{r:.1} / 5{}", row.rating_votes.map(|v| format!(" ({v} votes)")).unwrap_or_default())));
        add("Region", row.region.clone());
        add("Platform", Some(row.platform.clone()));
        add("Collection", row.torrent_source.clone());
        add("Size", row.download_size.filter(|s| *s > 0).map(|s| format_bytes(s as u64)));
        add("Emulator", Some(emulator_name(&row)));

        self.render_actions();
    }

    fn render_actions(self: &Rc<Self>) {
        while let Some(c) = self.actions.first_child() {
            self.actions.remove(&c);
        }
        let Some(game) = self.game.borrow().clone() else { return };
        let Some(row) = self.selected_row() else { return };
        let Some(id) = row.id else { return };

        if let Some(b) = self.busy.borrow().clone() {
            let l = gtk::Label::builder().label(&format!("{b}…")).css_classes(["muted"]).build();
            let sp = gtk::Spinner::builder().spinning(true).build();
            self.actions.append(&sp);
            self.actions.append(&l);
            return;
        }

        let dl = downloads::state(id);
        let downloading = dl.as_ref().map(|d| d.downloading).unwrap_or(false);
        let running = bus::is_running(id);
        let installed = row.installed || dl.as_ref().map(|d| d.installed).unwrap_or(false);

        if installed {
            if running {
                let b = btn("■ Stop", &["primary"]);
                b.connect_clicked(move |_| actions::stop(id));
                self.actions.append(&b);
            } else {
                let b = btn(if self.launching.get() { "Starting…" } else { "▶ Play" }, &["primary"]);
                b.set_sensitive(!self.launching.get());
                let (title, panel) = (row.title.clone(), Rc::downgrade(self));
                b.connect_clicked(move |_| {
                    let panel = panel.clone();
                    let busy: Rc<dyn Fn(bool)> = Rc::new(move |on| {
                        if let Some(p) = panel.upgrade() {
                            p.launching.set(on);
                            p.render_actions();
                        }
                    });
                    actions::play(id, title.clone(), busy);
                });
                self.actions.append(&b);
            }
            if let Some(m) = self.manual_path.borrow().clone() {
                let b = btn("Manual", &[]);
                b.connect_clicked(move |_| actions::open_document(m.clone()));
                self.actions.append(&b);
            }
        } else if downloading {
            let d = dl.clone().unwrap_or_default();
            let bx = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).hexpand(true).valign(gtk::Align::Center).build();
            let bar = gtk::ProgressBar::new();
            bar.set_fraction(d.progress.clamp(0.0, 1.0));
            if d.status.starts_with("Waiting") || d.status.starts_with("Extracting") {
                bar.pulse();
            }
            bx.append(&bar);
            bx.append(&gtk::Label::builder().label(&d.status).xalign(0.0).wrap(true).css_classes(["small", "muted"]).build());
            self.actions.append(&bx);
            let b = btn("✕ Cancel", &[]);
            b.connect_clicked(move |_| downloads::cancel(id));
            self.actions.append(&b);
        } else if bus::offline() {
            self.actions.append(&gtk::Label::builder().label("Not installed - offline mode").css_classes(["muted"]).tooltip_text("Enable downloads in Settings → Network").build());
        } else if row.game_torrent_index.is_some() {
            let label = if row.in_library {
                "↓ Re-download".to_string()
            } else {
                match row.download_size {
                    Some(s) if s > 0 => format!("↓ Download {}", format_bytes(s as u64)),
                    _ => "↓ Download".into(),
                }
            };
            let b = btn(&label, &["primary"]);
            let r = row.clone();
            b.connect_clicked(move |_| actions::download(&r));
            self.actions.append(&b);
        }

        // Favourite: frequent and reversible, stays in the bar.
        let fav = btn(if game.favorited { "★" } else { "☆" }, &["icon"]);
        fav.set_tooltip_text(Some(if game.favorited { "Remove from favorites" } else { "Add to favorites" }));
        if let Some(gid) = game.id {
            fav.connect_clicked(glib::clone!(#[weak(rename_to = panel)] self, move |_| {
                actions::toggle_favorite(gid, glib::clone!(#[weak] panel, move |res| {
                    if let Ok(v) = res {
                        if let Some(g) = panel.game.borrow_mut().as_mut() {
                            g.favorited = v;
                        }
                        bus::notify_library_changed(gid);
                        panel.render_actions();
                    }
                }));
            }));
        }
        self.actions.append(&fav);

        // Everything else behind one control.
        {
            let more = gtk::MenuButton::builder().label("⋯").css_classes(["btn", "icon"]).tooltip_text("More actions").build();
            let pop = gtk::Popover::builder().has_arrow(false).css_classes(["context-menu"]).build();
            let items = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let add = |label: &str, danger: bool, f: Box<dyn Fn()>| {
                let b = gtk::Button::builder().label(label).css_classes(["menu-item"]).build();
                if danger {
                    b.add_css_class("danger");
                }
                let p = pop.clone();
                b.connect_clicked(move |_| {
                    p.popdown();
                    f();
                });
                items.append(&b);
            };
            let status: Rc<dyn Fn(&str)> = {
                let panel = Rc::downgrade(self);
                Rc::new(move |s: &str| {
                    if let Some(p) = panel.upgrade() {
                        p.busy.replace(if s.is_empty() { None } else { Some(s.trim_end_matches("...").trim_end_matches('…').to_string()) });
                        p.render_actions();
                    }
                })
            };
            {
                let (w, r) = (self.window.clone(), game.clone());
                add("Add to playlist…", false, Box::new(move || actions::add_to_playlist(&w, &r)));
            }
            if installed {
                let (w, r) = (self.window.clone(), row.clone());
                add("Game settings…", false, Box::new(move || actions::game_settings(&w, &r)));
                let (w, r, s) = (self.window.clone(), row.clone(), status.clone());
                add("↺ Reset game data", true, Box::new(move || actions::reset(&w, &r, s.clone())));
            }
            let (w, r, s) = (self.window.clone(), row.clone(), status.clone());
            add("Uninstall", true, Box::new(move || actions::uninstall_group(&w, &r, s.clone())));
            pop.set_child(Some(&items));
            more.set_popover(Some(&pop));
            self.actions.append(&more);
        }

        // Status line under the bar.
        let text = dl.as_ref().filter(|d| !d.downloading && !d.status.is_empty() && !d.installed).map(|d| d.status.clone());
        self.status.set_label(text.as_deref().unwrap_or(""));
        self.status.set_visible(text.is_some());
    }
}

fn btn(label: &str, extra: &[&str]) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("btn");
    for c in extra {
        b.add_css_class(c);
    }
    b
}

/// What will run the game, from the variant slug (the web UI's `emulatorName`).
pub fn emulator_name(g: &Game) -> String {
    if g.torrent_source.as_deref() == Some("eXoScummVM") {
        return "ScummVM".into();
    }
    match g.dosbox_variant.as_deref() {
        Some("x98") => "DOSBox-X".into(),
        Some("pcbox") => "PCBox (not shipped)".into(),
        Some(v) if v.starts_with("86box") => "86Box".into(),
        _ => "DOSBox Staging".into(),
    }
}

#[allow(dead_code)]
pub fn title_markup(g: &Game) -> String {
    esc(&g.title)
}
