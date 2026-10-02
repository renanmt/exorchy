//! The game detail panel, laid out as a dossier: a header (favourite, close),
//! the hero row (cover beside platform, title, meta line, language variants -
//! one selected row at a time, so a merged card can play the German version -
//! and the actions: Play / Stop / Download / Cancel with a menu, playlist,
//! more), then the launch note, the preview video, genre tags, description
//! and four tabs: Overview (facts and features), Media (screenshots, press
//! articles), Manuals and Setup (emulator, game settings, reset, uninstall).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use exorchy_core::commands::{assets, games, win9x};
use exorchy_core::models::Game;
use gtk::glib;
use adw::prelude::*;

use crate::app;
use crate::ui::util::{esc, format_bytes, platform_tag};
use crate::ui::{actions, bus, covers, downloads, launch_notes};

pub fn panel_width() -> i32 {
    crate::theme::scaled(560)
}

pub struct DetailPanel {
    /// The panel itself; the library hosts it in its split view.
    pub widget: gtk::Box,
    open: Cell<bool>,
    open_listeners: RefCell<Vec<OpenListener>>,
    window: gtk::Window,
    game: RefCell<Option<Game>>,
    variants: RefCell<Vec<Game>>,
    selected: Cell<Option<i64>>,
    generation: Cell<u64>,
    busy: RefCell<Option<String>>,
    launching: Cell<bool>,
    cover: gtk::Picture,
    kind: gtk::Label,
    title: gtk::Label,
    subtitle: gtk::Label,
    chips: gtk::Box,
    actions: gtk::Box,
    secondary: gtk::Box,
    favorite: gtk::Button,
    tags: adw::WrapBox,
    status: gtk::Label,
    description: gtk::Label,
    notes: gtk::Label,
    info: gtk::Grid,
    features: gtk::Box,
    gallery: gtk::FlowBox,
    gallery_head: gtk::Label,
    articles_head: gtk::Label,
    articles_slot: gtk::Box,
    manuals: gtk::Box,
    setup: gtk::Box,
    tabs: adw::ViewStack,
    /// Screenshots and articles: together the Media tab's count.
    shots: Cell<usize>,
    articles: Cell<usize>,
    manual_path: RefCell<Option<String>>,
    scroller: gtk::ScrolledWindow,
    /// Below the cover; `ui::media` fills it.
    pub media_slot: gtk::Box,
    /// Right above the action bar; `ui::launch_notes` fills it.
    pub note_slot: gtk::Box,
    /// A blocking launch note: Play is insensitive with this as tooltip.
    play_blocked: RefCell<Option<String>>,
    /// The block is temporary (a support or pack download): Play spins.
    play_pending: Cell<bool>,
    shown_listeners: RefCell<Vec<ShownListener>>,
}

/// A panel-open/close observer (media, launch notes).
type ShownListener = Rc<dyn Fn(Option<&Game>)>;
/// An open/close observer (the split view that hosts the panel).
type OpenListener = Rc<dyn Fn(bool)>;

impl DetailPanel {
    pub fn new(window: &gtk::Window) -> Rc<Self> {
        // The split view's sidebar pane has libadwaita's own background; the
        // host paints the page background around the framed dossier.
        let host = gtk::Box::builder().orientation(gtk::Orientation::Vertical).width_request(crate::theme::scaled(300)).hexpand(true).css_classes(["dossier-host"]).build();
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).vexpand(true).css_classes(["detail-panel", "dossier"]).build();
        host.append(&root);

        let head = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).css_classes(["detail-head"]).build();
        let favorite = gtk::Button::builder().icon_name("non-starred-symbolic").css_classes(["btn", "icon", "ghost", "dossier-star"]).tooltip_text("Add to favorites (F)").build();
        let heading = gtk::Label::builder().label("GAME DOSSIER").xalign(0.0).hexpand(true).css_classes(["dossier-heading"]).build();
        let close = gtk::Button::builder().icon_name("window-close-symbolic").css_classes(["btn", "icon", "ghost"]).tooltip_text("Close (Esc)").build();
        head.append(&favorite);
        head.append(&heading);
        head.append(&close);
        root.append(&head);

        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).margin_start(16).margin_end(16).margin_bottom(20).build();

        // Hero: the cover beside who and what the game is, and what to do.
        let hero = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(16).css_classes(["dossier-hero"]).build();
        // Sized by the art itself (`covers::Size::Boxed`): the frame hugs a
        // box scan or a title screen alike, never letterboxed.
        let cover = gtk::Picture::builder().content_fit(gtk::ContentFit::Contain).can_shrink(false).halign(gtk::Align::Start).valign(gtk::Align::Start).css_classes(["detail-cover"]).build();
        hero.append(&cover);
        let side = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).hexpand(true).build();
        let kind = gtk::Label::builder().xalign(0.0).css_classes(["dossier-kind"]).build();
        let title = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["title-2"]).selectable(true).build();
        let subtitle = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["secondary"]).build();
        let chips = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(6).visible(false).build();
        let actions = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).margin_top(6).css_classes(["detail-actions"]).build();
        let secondary = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).build();
        side.append(&kind);
        side.append(&title);
        side.append(&subtitle);
        side.append(&chips);
        side.append(&actions);
        side.append(&secondary);
        hero.append(&side);
        body.append(&hero);

        let status = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["muted", "small"]).visible(false).build();
        body.append(&status);
        // The launch note (engine missing, support download, ...) right
        // under the actions whose Play it explains.
        let note_slot = gtk::Box::builder().orientation(gtk::Orientation::Vertical).visible(false).build();
        body.append(&note_slot);
        // The media module (preview video, theme music) mounts here. Not in a
        // tab: the video pauses the theme music while it plays, and a hidden
        // tab would play it unseen.
        let media_slot = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        body.append(&media_slot);
        let tags = adw::WrapBox::builder().child_spacing(6).line_spacing(6).visible(false).css_classes(["dossier-tags"]).build();
        body.append(&tags);
        let description = gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).css_classes(["detail-description"]).build();
        body.append(&description);

        // Tabs.
        let tabs = adw::ViewStack::builder().vhomogeneous(false).hhomogeneous(false).build();
        let switcher = adw::InlineViewSwitcher::builder().stack(&tabs).display_mode(adw::InlineViewSwitcherDisplayMode::Labels).css_classes(["dossier-tabs"]).build();
        body.append(&switcher);
        body.append(&tabs);

        // Overview: the facts beside what the game can do here, then eXo's notes.
        let overview = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).build();
        let columns = adw::WrapBox::builder().child_spacing(24).line_spacing(14).build();
        let info = gtk::Grid::builder().row_spacing(4).column_spacing(16).css_classes(["detail-info"]).build();
        columns.append(&info);
        let features = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(["dossier-features"]).build();
        columns.append(&features);
        overview.append(&columns);
        let notes = gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).visible(false).css_classes(["dossier-notes"]).build();
        overview.append(&notes);
        tabs.add_titled(&overview, Some("overview"), "Overview");

        // Media: screenshots, then "Covered in" (magazine articles, ui::reading).
        let media = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
        let gallery_head = gtk::Label::builder().label("Screenshots").xalign(0.0).css_classes(["dossier-section"]).visible(false).build();
        media.append(&gallery_head);
        let gallery = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).column_spacing(6).row_spacing(6).min_children_per_line(2).max_children_per_line(6).homogeneous(true).build();
        media.append(&gallery);
        let articles_head = gtk::Label::builder().label("Magazine articles").xalign(0.0).css_classes(["dossier-section"]).visible(false).build();
        media.append(&articles_head);
        let articles_slot = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        media.append(&articles_slot);
        tabs.add_titled(&media, Some("media"), "Media");

        let manuals = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
        tabs.add_titled(&manuals, Some("manuals"), "Manuals");
        let setup = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
        tabs.add_titled(&setup, Some("setup"), "Setup");

        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&body).build();
        root.append(&scroller);

        let panel = Rc::new(DetailPanel {
            widget: host,
            open: Cell::new(false),
            open_listeners: RefCell::new(Vec::new()),
            window: window.clone(),
            game: RefCell::new(None),
            variants: RefCell::new(Vec::new()),
            selected: Cell::new(None),
            generation: Cell::new(0),
            busy: RefCell::new(None),
            launching: Cell::new(false),
            cover,
            kind,
            title,
            subtitle,
            chips,
            actions,
            secondary,
            favorite,
            tags,
            status,
            description,
            notes,
            info,
            features,
            gallery,
            gallery_head,
            articles_head,
            articles_slot,
            manuals,
            setup,
            tabs,
            shots: Cell::new(0),
            articles: Cell::new(0),
            manual_path: RefCell::new(None),
            scroller,
            media_slot,
            note_slot,
            play_blocked: RefCell::new(None),
            play_pending: Cell::new(false),
            shown_listeners: RefCell::new(Vec::new()),
        });
        close.connect_clicked(glib::clone!(#[weak] panel, move |_| panel.close()));
        panel.favorite.connect_clicked(glib::clone!(#[weak] panel, move |_| panel.toggle_favorite()));
        launch_notes::attach(&panel);
        panel
    }

    /// The row the action bar acts on: the selected language variant, else
    /// the card's game.
    pub fn selected_game(&self) -> Option<Game> {
        self.selected_row()
    }

    /// A launch that cannot work: Play goes insensitive, `blocked` is its
    /// tooltip. `None` lifts the block.
    pub fn set_play_blocked(self: &Rc<Self>, blocked: Option<String>) {
        if *self.play_blocked.borrow() == blocked {
            return;
        }
        self.play_blocked.replace(blocked);
        self.render_actions();
    }

    /// The block is being worked on (a download in flight): Play shows a
    /// spinner instead of failing.
    pub fn set_play_pending(self: &Rc<Self>, pending: bool) {
        if self.play_pending.get() == pending {
            return;
        }
        self.play_pending.set(pending);
        self.render_actions();
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// The dossier is open on this game.
    pub fn shows(&self, id: Option<i64>) -> bool {
        self.open.get() && id.is_some() && self.game.borrow().as_ref().and_then(|g| g.id) == id
    }

    /// Open again on the last game shown (the I shortcut).
    pub fn reopen(self: &Rc<Self>) {
        let last = self.game.borrow().clone();
        if let Some(g) = last {
            self.show(g);
        }
    }

    /// The host (the library's split view) shows or hides the panel on this.
    pub fn on_open_changed(&self, f: impl Fn(bool) + 'static) {
        self.open_listeners.borrow_mut().push(Rc::new(f));
    }

    fn set_open(&self, open: bool) {
        if self.open.replace(open) == open {
            return;
        }
        let cbs = self.open_listeners.borrow().clone();
        for cb in cbs {
            cb(open);
        }
    }

    /// Hide the panel. Unconditional: the host is told to hide even when
    /// the flag already says closed, so the ✕ can never be a no-op while
    /// the sidebar is visible.
    pub fn close(&self) {
        let was_open = self.open.replace(false);
        let cbs = self.open_listeners.borrow().clone();
        for cb in cbs {
            cb(false);
        }
        if was_open {
            self.notify_shown(None);
        }
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

    pub fn show(self: &Rc<Self>, game: Game) {
        let same = self.game.borrow().as_ref().and_then(|g| g.id) == game.id && game.id.is_some();
        // Reopening the same game after a close is an open too: the listeners
        // (media, launch notes) tore down on the close and must come back.
        let reopened = !self.open.get();
        if same {
            // The caller's row is newer than ours (a card after a refresh).
            self.merge_row(&game);
        } else {
            self.game.replace(Some(game.clone()));
        }
        if !same {
            // Every game opens on its overview.
            self.tabs.set_visible_child_name("overview");
            self.shots.set(0);
            self.articles.set(0);
            self.update_media_title();
            self.selected.set(game.id);
            self.variants.replace(vec![game.clone()]);
            self.scroller.vadjustment().set_value(0.0);
            self.load_cover(&game);
            self.load_variants(&game);
            self.load_metadata(&game);
            while let Some(c) = self.articles_slot.first_child() {
                self.articles_slot.remove(&c);
            }
            if let Some(id) = game.id {
                let panel = Rc::downgrade(self);
                self.articles_slot.append(&crate::ui::reading::game_articles_widget(id, &self.window, move |n| {
                    if let Some(p) = panel.upgrade().filter(|p| p.game.borrow().as_ref().and_then(|g| g.id) == Some(id)) {
                        p.articles.set(n);
                        p.update_media_title();
                    }
                }));
            }
            // An installed game whose extras are still downloading after a
            // restart gets its tracker back, so the phase stays visible.
            if game.installed {
                if let Some(id) = game.id {
                    downloads::watch_extras_if_pending(id, Some(game.title.clone()));
                }
            }
        }
        self.render();
        self.set_open(true);
        if !same || reopened {
            self.notify_shown(Some(&game));
        }
    }

    /// The library changed for `id`: re-read the row if it is ours.
    /// Take a fresh copy of the game the panel shows: the row itself and the
    /// variant list the action bar renders from (a single-language game's
    /// only variant IS the row, so it must follow every refresh).
    fn merge_row(&self, fresh: &Game) {
        self.game.replace(Some(fresh.clone()));
        let merged = merged_variants(&self.variants.borrow(), fresh);
        self.variants.replace(merged);
    }

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
                    panel.merge_row(&fresh);
                    panel.load_variants(&fresh);
                    panel.load_metadata(&fresh);
                    panel.render();
                }
            }
        }));
    }

    /// A game was starred or unstarred (here or on a card): if it is the one
    /// shown, repaint the action bar's star; nothing else of the panel changes.
    pub fn set_favorite(self: &Rc<Self>, id: i64, favorited: bool) {
        let changed = match self.game.borrow_mut().as_mut() {
            Some(g) if g.id == Some(id) && g.favorited != favorited => {
                g.favorited = favorited;
                true
            }
            _ => false,
        };
        if changed {
            self.render_actions();
        }
    }

    /// Rebuild the action bar (its menu reads state such as hidden titles).
    pub fn refresh_actions(self: &Rc<Self>) {
        self.render_actions();
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
        covers::request(game.torrent_source.as_deref(), game.thumbnail_key.as_deref(), covers::Size::Boxed(crate::theme::scaled(150) as u32), move |t| {
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
                for (thumb, full) in meta.thumbnails.iter().zip(meta.images.iter()) {
                    let pic = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).height_request(crate::theme::scaled(96)).css_classes(["gallery-thumb"]).build();
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
                panel.shots.set(meta.images.len());
                panel.update_media_title();
                // The manual and counts feed the tabs, the features and Play's menu.
                panel.render();
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
        let platform = platform_tag(game.torrent_source.as_deref());
        self.kind.set_label(&platform.map(str::to_uppercase).unwrap_or_default());
        self.kind.set_visible(platform.is_some());
        let mut parts = Vec::new();
        if let Some(y) = game.year {
            parts.push(y.to_string());
        }
        if let Some(g) = genres(game.genre.as_deref()).into_iter().next() {
            parts.push(g);
        }
        if let Some(d) = game.developer.as_deref().filter(|s| !s.is_empty()) {
            parts.push(d.to_string());
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

        // Tags: the genres, then how it is played.
        while let Some(c) = self.tags.first_child() {
            self.tags.remove(&c);
        }
        let mut tags = genres(row.genre.as_deref());
        if let Some(m) = row.play_mode.as_deref().filter(|m| !m.is_empty()) {
            tags.extend(m.split(['/', ';', ',']).map(str::trim).filter(|t| !t.is_empty()).map(String::from));
        }
        tags.dedup();
        self.tags.set_visible(!tags.is_empty());
        for t in tags.iter().take(6) {
            self.tags.append(&gtk::Label::builder().label(t).css_classes(["dossier-tag"]).build());
        }

        let desc = row.description.clone().or(game.description.clone()).unwrap_or_default();
        self.description.set_label(&desc);
        self.description.set_visible(!desc.is_empty());
        let notes = row.notes.clone().unwrap_or_default();
        self.notes.set_label(&notes);
        self.notes.set_visible(!notes.is_empty());

        // Overview facts.
        while let Some(c) = self.info.first_child() {
            self.info.remove(&c);
        }
        let mut r = 0;
        let mut add = |k: &str, v: Option<String>| {
            let Some(v) = v.filter(|v| !v.trim().is_empty()) else { return };
            let kl = gtk::Label::builder().label(k).xalign(0.0).css_classes(["muted", "small"]).valign(gtk::Align::Start).build();
            let vl = gtk::Label::builder().label(&v).xalign(0.0).wrap(true).max_width_chars(28).selectable(true).build();
            self.info.attach(&kl, 0, r, 1, 1);
            self.info.attach(&vl, 1, r, 1, 1);
            r += 1;
        };
        add("Developer", row.developer.clone());
        add("Publisher", row.publisher.clone());
        // LaunchBox dates are ISO timestamps; the day is what the reader wants.
        add("Released", row.release_date.as_deref().map(|d| d.chars().take(10).collect()).or(row.year.map(|y| y.to_string())));
        add("Platform", Some(row.platform.clone()));
        add("Genre", row.genre.clone());
        add("Series", row.series.clone());
        add("Mode", row.play_mode.clone());
        add("Players", row.max_players.map(|n| n.to_string()));
        add("Region", row.region.clone());
        add("Rating", row.rating.map(|r| format!("{r:.1} / 5{}", row.rating_votes.map(|v| format!(" ({v} votes)")).unwrap_or_default())));
        add("Size", row.download_size.filter(|s| *s > 0).map(|s| format_bytes(s as u64)));

        self.render_features(&row, None);
        self.render_setup(&row, None);
        // DOS / Windows 3.x: the backend knows the engine for sure (eXo's
        // DOSBox-X pins, printing confs, the per-game override).
        if let Some(id) = row.id {
            let core = app::core();
            app::spawn(async move { games::game_engine_info(core.state(), id).await }, glib::clone!(#[weak(rename_to = panel)] self, move |res| {
                let Ok(info) = res else { return };
                let Some(row) = panel.selected_row().filter(|r| r.id == Some(id)) else { return };
                panel.render_features(&row, Some(&info));
                panel.render_setup(&row, Some(&info));
            }));
        }
        self.render_manuals();

        self.render_actions();
    }

    /// What the game can do here, from what is known for sure: the emulator,
    /// printing, players, the manual, language versions, CRT shaders.
    fn render_features(&self, row: &Game, engine: Option<&games::GameEngineInfo>) {
        while let Some(c) = self.features.first_child() {
            self.features.remove(&c);
        }
        self.features.append(&gtk::Label::builder().label("Features").xalign(0.0).css_classes(["muted", "small"]).build());
        let dos_engine = engine.and_then(|e| e.engine.as_deref());
        let emulator = match dos_engine {
            Some("dosbox-x") => "DOSBox-X".to_string(),
            Some(_) => "DOSBox Staging".to_string(),
            None => emulator_name(row),
        };
        let mut items: Vec<(&str, String)> = vec![("applications-games-symbolic", format!("Runs under {emulator}"))];
        if engine.is_some_and(|e| e.prints) {
            items.push(("printer-symbolic", if dos_engine == Some("dosbox-x") { "Prints to PNG".into() } else { "Prints (needs DOSBox-X)".into() }));
        }
        if let Some(n) = row.max_players.filter(|n| *n > 1) {
            items.push(("system-users-symbolic", format!("Up to {n} players")));
        }
        if self.manual_path.borrow().is_some() {
            items.push(("x-office-document-symbolic", "Manual included".into()));
        }
        let languages = self.variants.borrow().len();
        if languages > 1 {
            items.push(("preferences-desktop-locale-symbolic", format!("{languages} language versions")));
        }
        if dos_engine == Some("staging") {
            items.push(("video-display-symbolic", "CRT shaders".into()));
        }
        for (icon, text) in items {
            let line = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).css_classes(["dossier-feature"]).build();
            line.append(&gtk::Image::from_icon_name(icon));
            line.append(&gtk::Label::builder().label(&text).xalign(0.0).build());
            self.features.append(&line);
        }
    }

    /// "Media (n)": screenshots plus articles; hidden when there are none.
    fn update_media_title(&self) {
        let Some(media) = self.gallery.parent() else { return };
        let n = self.shots.get() + self.articles.get();
        let page = self.tabs.page(&media);
        page.set_title(Some(&format!("Media ({n})")));
        page.set_visible(n > 0);
        // Headings only when both kinds are there.
        let both = self.shots.get() > 0 && self.articles.get() > 0;
        self.gallery_head.set_visible(both);
        self.articles_head.set_visible(both);
    }

    /// The Manuals tab, and its count in the tab title.
    fn render_manuals(self: &Rc<Self>) {
        while let Some(c) = self.manuals.first_child() {
            self.manuals.remove(&c);
        }
        let manual = self.manual_path.borrow().clone();
        let page = self.tabs.page(&self.manuals);
        page.set_title(Some(&format!("Manuals ({})", manual.is_some() as u8)));
        page.set_visible(manual.is_some());
        let Some(path) = manual else { return };
        let title = self.game.borrow().as_ref().map(|g| g.title.clone()).unwrap_or_default();
        let line = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(10).css_classes(["dossier-doc"]).build();
        line.append(&gtk::Image::from_icon_name("x-office-document-symbolic"));
        line.append(&gtk::Label::builder().label("Game manual").xalign(0.0).hexpand(true).build());
        let open = btn("Read", &[]);
        let w = self.window.clone();
        open.connect_clicked(move |_| crate::ui::pdf::open_document_viewer(&w, &path, &title, None, 1));
        line.append(&open);
        self.manuals.append(&line);
    }

    /// The Setup tab: what runs the game and the per-game controls.
    fn render_setup(self: &Rc<Self>, row: &Game, engine: Option<&games::GameEngineInfo>) {
        while let Some(c) = self.setup.first_child() {
            self.setup.remove(&c);
        }
        let grid = gtk::Grid::builder().row_spacing(4).column_spacing(16).css_classes(["detail-info"]).build();
        let mut r = 0;
        let mut add = |k: &str, v: String| {
            grid.attach(&gtk::Label::builder().label(k).xalign(0.0).css_classes(["muted", "small"]).build(), 0, r, 1, 1);
            grid.attach(&gtk::Label::builder().label(&v).xalign(0.0).wrap(true).selectable(true).build(), 1, r, 1, 1);
            r += 1;
        };
        let emulator = match engine.and_then(|e| e.engine.as_deref()) {
            Some(e) => {
                let name = if e == "dosbox-x" { "DOSBox-X" } else { "DOSBox Staging" };
                let picked = engine.and_then(|i| i.exo_engine.as_deref()) == Some(e);
                format!("{name}{}", if picked { " (eXo's choice)" } else { " (your choice)" })
            }
            None => emulator_name(row),
        };
        add("Emulator", emulator);
        if let Some(c) = row.torrent_source.clone() {
            add("Collection", c);
        }
        add("Status", if row.installed { "Installed" } else if row.in_library { "In your library, not installed" } else { "Not installed" }.into());
        self.setup.append(&grid);

        if !row.installed {
            self.setup.append(&gtk::Label::builder().label("Install the game to change its emulator and settings.").xalign(0.0).wrap(true).css_classes(["muted", "small"]).build());
            return;
        }
        let buttons = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).build();
        let settings = btn("Game settings…", &[]);
        let (w, g) = (self.window.clone(), row.clone());
        settings.connect_clicked(move |_| actions::game_settings(&w, &g));
        buttons.append(&settings);
        let reset = btn("Reset game data", &["danger"]);
        let (w, g, s) = (self.window.clone(), row.clone(), self.status_setter());
        reset.connect_clicked(move |_| actions::reset(&w, &g, s.clone()));
        buttons.append(&reset);
        self.setup.append(&buttons);
    }

    /// A busy line for long actions started from the panel ("Resetting…").
    fn status_setter(self: &Rc<Self>) -> Rc<dyn Fn(&str)> {
        let panel = Rc::downgrade(self);
        Rc::new(move |s: &str| {
            if let Some(p) = panel.upgrade() {
                p.busy.replace(if s.is_empty() { None } else { Some(s.trim_end_matches("...").trim_end_matches('…').to_string()) });
                // Deferred: a menu's popover may still be closing, and
                // rebuilding the bar under it would leave its grab.
                let p2 = p.clone();
                glib::idle_add_local_once(move || p2.render_actions());
            }
        })
    }

    fn toggle_favorite(self: &Rc<Self>) {
        let Some(gid) = self.game.borrow().as_ref().and_then(|g| g.id) else { return };
        // The bus brings the new state back to this panel (`set_favorite`).
        actions::toggle_favorite(gid, move |res| {
            if let Ok(v) = res {
                bus::notify_favorite_changed(gid, v);
            }
        });
    }

    fn render_actions(self: &Rc<Self>) {
        while let Some(c) = self.actions.first_child() {
            self.actions.remove(&c);
        }
        while let Some(c) = self.secondary.first_child() {
            self.secondary.remove(&c);
        }
        let Some(game) = self.game.borrow().clone() else { return };
        let Some(row) = self.selected_row() else { return };
        let Some(id) = row.id else { return };

        self.favorite.set_icon_name(if game.favorited { "starred-symbolic" } else { "non-starred-symbolic" });
        self.favorite.set_tooltip_text(Some(if game.favorited { "Remove from favorites (F)" } else { "Add to favorites (F)" }));
        if game.favorited {
            self.favorite.add_css_class("is-favorite");
        } else {
            self.favorite.remove_css_class("is-favorite");
        }

        if let Some(b) = self.busy.borrow().clone() {
            let l = gtk::Label::builder().label(format!("{b}…")).css_classes(["muted"]).build();
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
                let b = btn("■ Stop", &["primary", "dossier-primary"]);
                b.connect_clicked(move |_| actions::stop(id));
                self.actions.append(&b);
            } else {
                let blocked = self.play_blocked.borrow().clone();
                let pending = self.play_pending.get();
                let launching = self.launching.get();
                let label = if launching {
                    "Starting…"
                } else if pending {
                    "Preparing…"
                } else {
                    "▶ Play"
                };
                // Play, and what belongs to playing it behind the arrow.
                let play = adw::SplitButton::builder().label(label).css_classes(["primary", "dossier-primary"]).hexpand(true).build();
                if launching || pending {
                    // The spinner is the button's own content while it waits.
                    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                    inner.append(&gtk::Spinner::builder().spinning(true).build());
                    inner.append(&gtk::Label::new(Some(label)));
                    play.set_child(Some(&inner));
                }
                play.set_sensitive(!launching && !pending && blocked.is_none());
                play.set_tooltip_text(blocked.as_deref().or(Some("Play (Enter)")));
                let (title, panel) = (row.title.clone(), Rc::downgrade(self));
                play.connect_clicked(move |_| play_with_net_prompt(&panel, id, title.clone()));
                let (pop, items) = menu();
                {
                    let (w, r, p) = (self.window.clone(), row.clone(), pop.clone());
                    items.append(&menu_item("Game settings…", false, move || {
                        p.popdown();
                        actions::game_settings(&w, &r);
                    }));
                }
                if let Some(m) = self.manual_path.borrow().clone() {
                    let (w, title, p) = (self.window.clone(), row.title.clone(), pop.clone());
                    items.append(&menu_item("Read the manual", false, move || {
                        p.popdown();
                        crate::ui::pdf::open_document_viewer(&w, &m, &title, None, 1);
                    }));
                }
                play.set_popover(Some(&pop));
                self.actions.append(&play);
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
            self.actions.append(&gtk::Label::builder().label("Not installed - offline mode").xalign(0.0).wrap(true).css_classes(["muted"]).tooltip_text("Enable downloads in Settings → Network").build());
        } else if row.game_torrent_index.is_some() {
            let label = if row.in_library {
                "↓ Re-download".to_string()
            } else {
                match row.download_size {
                    Some(s) if s > 0 => format!("↓ Download {}", format_bytes(s as u64)),
                    _ => "↓ Download".into(),
                }
            };
            let b = btn(&label, &["primary", "dossier-primary"]);
            b.set_hexpand(true);
            let r = row.clone();
            b.connect_clicked(move |_| actions::download(&r));
            self.actions.append(&b);
        }

        // Second row: playlists, then everything else behind one control.
        let playlist = btn("Add to playlist", &[]);
        playlist.set_hexpand(true);
        {
            let (w, r) = (self.window.clone(), game.clone());
            playlist.connect_clicked(move |_| actions::add_to_playlist(&w, &r));
        }
        self.secondary.append(&playlist);
        {
            let more = gtk::MenuButton::builder().icon_name("view-more-symbolic").css_classes(["btn", "icon"]).tooltip_text("More actions").build();
            let (pop, items) = menu();
            let status = self.status_setter();
            {
                let title = game.title.clone();
                let p = pop.clone();
                if crate::ui::hidden::is_hidden(id) {
                    items.append(&menu_item("Unhide title", false, move || {
                        p.popdown();
                        crate::ui::hidden::unhide(id, &title);
                    }));
                } else {
                    items.append(&menu_item("Hide title", false, move || {
                        p.popdown();
                        crate::ui::hidden::hide(id, &title);
                    }));
                }
            }
            if installed {
                let (w, r, s, p) = (self.window.clone(), row.clone(), status.clone(), pop.clone());
                items.append(&menu_item("↺ Reset game data", true, move || {
                    p.popdown();
                    actions::reset(&w, &r, s.clone());
                }));
            }
            let (w, r, s, p) = (self.window.clone(), row.clone(), status.clone(), pop.clone());
            items.append(&menu_item("Uninstall", true, move || {
                p.popdown();
                actions::uninstall_group(&w, &r, s.clone());
            }));
            more.set_popover(Some(&pop));
            self.secondary.append(&more);
        }

        // Status line under the actions.
        let text = dl.as_ref().filter(|d| !d.downloading && !d.status.is_empty() && !d.installed).map(|d| d.status.clone());
        self.status.set_label(text.as_deref().unwrap_or(""));
        self.status.set_visible(text.is_some());
    }

    /// Play the shown game (the Enter shortcut); false when Play is not on offer.
    pub fn play_shown(self: &Rc<Self>) -> bool {
        let Some(row) = self.selected_row() else { return false };
        let Some(id) = row.id else { return false };
        let installed = row.installed || downloads::state(id).map(|d| d.installed).unwrap_or(false);
        if !installed || bus::is_running(id) || self.launching.get() || self.play_pending.get() || self.play_blocked.borrow().is_some() {
            return false;
        }
        play_with_net_prompt(&Rc::downgrade(self), id, row.title.clone());
        true
    }

    /// Star or unstar the shown game (the F shortcut).
    pub fn favorite_shown(self: &Rc<Self>) {
        self.toggle_favorite();
    }
}

/// A context-menu popover and the box its items go in.
fn menu() -> (gtk::Popover, gtk::Box) {
    let pop = gtk::Popover::builder().has_arrow(false).css_classes(["context-menu"]).build();
    let items = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pop.set_child(Some(&items));
    (pop, items)
}

fn menu_item(label: &str, danger: bool, f: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::builder().label(label).css_classes(["menu-item"]).build();
    if danger {
        b.add_css_class("danger");
    }
    b.connect_clicked(move |_| f());
    b
}

/// A genre string ("Action / Adventure", "RPG; Strategy") as its parts.
fn genres(genre: Option<&str>) -> Vec<String> {
    genre
        .unwrap_or_default()
        .split(['/', ';', ','])
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .map(String::from)
        .collect()
}

/// Online-capable Win9x games ask once, on the first Play, whether to turn
/// multiplayer on - the backend answers `prompt: false` for every game and
/// every state where the question would be noise. Everything else launches
/// straight away.
fn play_with_net_prompt(panel: &std::rc::Weak<DetailPanel>, id: i64, title: String) {
    let Some(p) = panel.upgrade() else { return };
    if p.launching.get() {
        return;
    }
    let busy: Rc<dyn Fn(bool)> = {
        let panel = panel.clone();
        Rc::new(move |on| {
            if let Some(p) = panel.upgrade() {
                p.launching.set(on);
                p.render_actions();
            }
        })
    };
    let window = p.window.clone();
    let core = app::core();
    app::spawn(async move { win9x::win9x_multiplayer_info(core.state(), id).await }, move |res| match res {
        Ok(info) if info.prompt => net_prompt(&window, id, title, busy),
        _ => actions::play(id, title, busy),
    });
}

/// Asked on Play, not in Settings: this is the moment the online mode would
/// otherwise silently be missing. Either answer can be remembered, and
/// either answer launches - a dismissed system dialog means "not now", not
/// "don't play".
fn net_prompt(parent: &gtk::Window, id: i64, title: String, busy: Rc<dyn Fn(bool)>) {
    let dialog = adw::AlertDialog::builder()
        .heading("Play online?")
        .body(
            "This game can play online against others who own the collection, over a community-run IPX gateway. \
             That needs one-time permission from your system to bridge the emulated network card; you can also play on your own without it.",
        )
        .build();
    dialog.add_responses(&[("offline", "Play offline"), ("setup", "Set up now…")]);
    dialog.set_response_appearance("setup", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("setup"));
    dialog.set_close_response("offline");
    let remember = gtk::CheckButton::with_label("Don't ask again");
    dialog.set_extra_child(Some(&remember));
    dialog.connect_response(None, move |_, response| {
        if remember.is_active() {
            let core = app::core();
            app::spawn(async move { win9x::dismiss_win9x_network_prompt(core.state()).await }, |res| {
                if let Err(e) = res {
                    log::warn!("dismiss_win9x_network_prompt: {e}");
                }
            });
        }
        let (title, busy) = (title.clone(), busy.clone());
        if response == "setup" {
            busy(true);
            let core = app::core();
            app::spawn(async move { win9x::enable_win9x_network(core.clone()).await }, move |res| {
                busy(false);
                if let Err(e) = res {
                    if !e.to_lowercase().contains("cancelled") {
                        bus::toast_with("Could not enable multiplayer", Some(&e), None);
                    }
                }
                actions::play(id, title, busy);
            });
        } else {
            actions::play(id, title, busy);
        }
    });
    dialog.present(Some(parent));
}

fn btn(label: &str, extra: &[&str]) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("btn");
    for c in extra {
        b.add_css_class(c);
    }
    b
}

/// The variant list with `fresh` in place of its own row. A list of one
/// (no language variants) is replaced whole.
fn merged_variants(variants: &[Game], fresh: &Game) -> Vec<Game> {
    if variants.len() <= 1 {
        return vec![fresh.clone()];
    }
    variants
        .iter()
        .map(|v| if v.id == fresh.id && fresh.id.is_some() { fresh.clone() } else { v.clone() })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, lang: &str, installed: bool) -> Game {
        Game { id: Some(id), language: lang.into(), installed, ..Game::default() }
    }

    #[test]
    fn a_single_variant_follows_the_fresh_row() {
        let stale = vec![row(1, "EN", false)];
        let merged = merged_variants(&stale, &row(1, "EN", true));
        assert!(merged[0].installed);
        // Even a different id replaces a one-row list: the panel moved on.
        assert_eq!(merged_variants(&stale, &row(2, "EN", true))[0].id, Some(2));
    }

    #[test]
    fn only_the_matching_variant_is_replaced_in_a_group() {
        let stale = vec![row(1, "EN", false), row(2, "DE", true)];
        let merged = merged_variants(&stale, &row(1, "EN", true));
        assert!(merged[0].installed && merged[1].installed);
        assert_eq!(merged.len(), 2);
    }
}
