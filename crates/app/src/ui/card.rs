//! One game in the grid (and on the My Library shelves): cover, title, year
//! and genre, platform and language badges, the action label (Play /
//! Download size / Incomplete), a download overlay with progress and cancel,
//! and the favourite star. Recycled by `GridView`: `bind` restarts the cover
//! walk with a generation counter so a late texture never lands on the
//! wrong game.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use exorchy_core::models::Game;
use gtk::glib;
use gtk::prelude::*;

use crate::ui::util::{format_bytes, parse_lang_entries, platform_tag};
use crate::ui::{actions, bus, covers, downloads};

thread_local! {
    /// The game the detail panel shows; its card carries the `selected` class.
    static SELECTED: Cell<Option<i64>> = const { Cell::new(None) };
}

/// Record which game is open; the library repaints the cards it knows.
pub fn set_selected_id(id: Option<i64>) {
    SELECTED.with(|s| s.set(id));
}

pub const CARD_WIDTH: i32 = crate::theme::scaled(172);
pub const ART_HEIGHT: i32 = crate::theme::scaled(226);
/// Half the gap between cards.
const CARD_MARGIN: i32 = crate::theme::scaled(5);

pub struct Card {
    pub widget: gtk::Box,
    game: RefCell<Option<Game>>,
    generation: Cell<u64>,
    cover: gtk::Picture,
    cover_empty: gtk::Label,
    title: gtk::Label,
    meta: gtk::Label,
    badges: gtk::Box,
    action: gtk::Label,
    fav: gtk::Button,
    dl_overlay: gtk::Box,
    dl_bar: gtk::ProgressBar,
    dl_pct: gtk::Label,
    /// Hide the platform badge when a collection filter implies it.
    hide_platform: Cell<bool>,
    /// On the Recently played shelf: the menu can take it off.
    in_recent: Cell<bool>,
    on_detail: Rc<dyn Fn(Game)>,
}

impl Card {
    pub fn new(on_detail: Rc<dyn Fn(Game)>) -> Rc<Self> {
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["game-card"])
            .width_request(CARD_WIDTH)
            .margin_start(CARD_MARGIN)
            .margin_end(CARD_MARGIN)
            .margin_top(CARD_MARGIN)
            .margin_bottom(CARD_MARGIN)
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
            .build();

        let dl_bar = gtk::ProgressBar::builder().hexpand(true).valign(gtk::Align::Center).build();
        let dl_pct = gtk::Label::builder().css_classes(["game-card-pct"]).max_width_chars(6).build();
        let dl_cancel = gtk::Button::builder().icon_name("window-close-symbolic").css_classes(["btn", "icon", "ghost"]).tooltip_text("Cancel download").build();
        let dl_overlay = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .css_classes(["game-card-download"])
            .valign(gtk::Align::Fill)
            .halign(gtk::Align::Fill)
            .visible(false)
            .build();
        let dl_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        dl_row.set_valign(gtk::Align::Center);
        dl_row.set_vexpand(true);
        dl_row.set_margin_start(10);
        dl_row.set_margin_end(10);
        dl_row.append(&dl_bar);
        dl_row.append(&dl_cancel);
        dl_overlay.append(&dl_row);
        dl_overlay.append(&dl_pct);

        let fav = gtk::Button::builder()
            .label("★")
            .css_classes(["fav-btn"])
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .tooltip_text("Add to favorites")
            .build();

        let art = gtk::Overlay::new();
        art.set_child(Some(&cover));
        art.add_overlay(&cover_empty);
        art.add_overlay(&dl_overlay);
        art.add_overlay(&fav);
        widget.append(&art);

        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(3).css_classes(["game-card-body"]).build();
        // Ellipsised labels report the full text as natural width; capping it
        // keeps the card's natural size at its request, which is what the
        // grid sizes its cells by.
        let title = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).lines(2).wrap(true).max_width_chars(12).css_classes(["game-card-title"]).build();
        let meta = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(12).css_classes(["game-card-meta"]).build();
        let badges = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(4).build();
        let action = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(12).css_classes(["game-card-action"]).build();
        body.append(&title);
        body.append(&meta);
        body.append(&badges);
        body.append(&action);
        widget.append(&body);

        let card = Rc::new(Card {
            widget,
            game: RefCell::new(None),
            generation: Cell::new(0),
            cover,
            cover_empty,
            title,
            meta,
            badges,
            action,
            fav,
            dl_overlay,
            dl_bar,
            dl_pct,
            hide_platform: Cell::new(false),
            in_recent: Cell::new(false),
            on_detail,
        });

        // Left click: details. Right click: the actions menu.
        let click = gtk::GestureClick::builder().button(1).build();
        click.connect_released(glib::clone!(#[weak] card, move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(game) = card.game.borrow().clone() {
                (card.on_detail)(game);
            }
        }));
        card.widget.add_controller(click);
        let right = gtk::GestureClick::builder().button(3).build();
        right.connect_pressed(glib::clone!(#[weak] card, move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            card.show_menu(x, y);
        }));
        card.widget.add_controller(right);

        card.fav.connect_clicked(glib::clone!(#[weak] card, move |_| card.toggle_favorite()));
        dl_cancel.connect_clicked(glib::clone!(#[weak] card, move |_| {
            if let Some(id) = card.game.borrow().as_ref().and_then(|g| g.id) {
                downloads::cancel(id);
            }
        }));
        card
    }

    pub fn set_in_recent(&self, v: bool) {
        self.in_recent.set(v);
    }

    pub fn game_id(&self) -> Option<i64> {
        self.game.borrow().as_ref().and_then(|g| g.id)
    }

    pub fn set_hide_platform(&self, hide: bool) {
        self.hide_platform.set(hide);
    }

    /// Show a game. Idempotent for the same row; cheap enough to call on
    /// every refresh.
    pub fn bind(self: &Rc<Self>, game: &Game) {
        let same_cover = self
            .game
            .borrow()
            .as_ref()
            .map(|g| g.thumbnail_key == game.thumbnail_key && g.torrent_source == game.torrent_source)
            .unwrap_or(false);
        self.game.replace(Some(game.clone()));

        self.title.set_label(&game.title);
        self.title.set_tooltip_text(Some(&game.title));
        // "1991 · DOS · Strategy", as in the concept; the platform drops out
        // where the shelf already says it.
        let mut meta = Vec::new();
        if let Some(y) = game.year {
            meta.push(y.to_string());
        }
        if !self.hide_platform.get() {
            if let Some(p) = platform_tag(game.torrent_source.as_deref()) {
                meta.push(p.to_string());
            }
        }
        if let Some(g) = game.genre.as_deref().filter(|g| !g.is_empty()) {
            meta.push(g.split([';', '/']).next().unwrap_or(g).trim().to_string());
        }
        self.meta.set_label(&meta.join(" · "));

        while let Some(c) = self.badges.first_child() {
            self.badges.remove(&c);
        }
        let langs = parse_lang_entries(game.available_languages.as_deref());
        for l in &langs {
            let cls = match l.state {
                2 => "badge-lang-installed",
                1 => "badge-lang-library",
                _ => "badge-lang",
            };
            self.badges.append(&badge(&l.lang, cls));
        }
        self.badges.set_visible(self.badges.first_child().is_some());

        self.set_favorited(game.favorited);
        self.refresh_selected();
        if game.installed || game.in_library {
            self.widget.add_css_class("installed");
        } else {
            self.widget.remove_css_class("installed");
        }
        self.refresh_download();

        if !same_cover {
            self.load_cover();
        }
    }

    /// Fetch the cover again (a poster pack was installed or removed).
    pub fn reload_cover(self: &Rc<Self>) {
        if self.game.borrow().is_some() {
            self.load_cover();
        }
    }

    fn load_cover(self: &Rc<Self>) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let (source, key, title) = {
            let g = self.game.borrow();
            let g = g.as_ref().expect("bound");
            (g.torrent_source.clone(), g.thumbnail_key.clone(), g.title.clone())
        };
        self.cover.set_paintable(gtk::gdk::Paintable::NONE);
        self.cover_empty.set_visible(false);
        let card = Rc::downgrade(self);
        // Exactly the card's size: a GtkPicture's natural size is its
        // texture's pixel size, and the grid sizes its cells by it.
        covers::request(source.as_deref(), key.as_deref(), covers::Size::Fill(CARD_WIDTH as u32, ART_HEIGHT as u32), move |texture| {
            let Some(card) = card.upgrade() else { return };
            if card.generation.get() != generation {
                return;
            }
            match texture {
                Some(t) => card.cover.set_paintable(Some(&t)),
                None => {
                    card.cover_empty.set_label(&title);
                    card.cover_empty.set_visible(true);
                }
            }
        });
    }

    fn set_favorited(&self, on: bool) {
        if on {
            self.fav.add_css_class("is-favorited");
            self.fav.set_tooltip_text(Some("Remove from favorites"));
        } else {
            self.fav.remove_css_class("is-favorited");
            self.fav.set_tooltip_text(Some("Add to favorites"));
        }
    }

    fn toggle_favorite(self: &Rc<Self>) {
        let Some(id) = self.game_id() else { return };
        let prev = self.game.borrow().as_ref().map(|g| g.favorited).unwrap_or(false);
        self.set_favorited(!prev);
        let card = Rc::downgrade(self);
        actions::toggle_favorite(id, move |res| {
            let Some(card) = card.upgrade() else { return };
            if card.game_id() != Some(id) {
                return;
            }
            match res {
                Ok(v) => {
                    if let Some(g) = card.game.borrow_mut().as_mut() {
                        g.favorited = v;
                    }
                    card.set_favorited(v);
                    bus::notify_favorite_changed(id, v);
                }
                Err(_) => card.set_favorited(prev),
            }
        });
    }

    /// Paint (or clear) the selection ring.
    pub fn refresh_selected(&self) {
        let selected = self.game_id().is_some() && self.game_id() == SELECTED.with(|s| s.get());
        if selected {
            self.widget.add_css_class("selected");
        } else {
            self.widget.remove_css_class("selected");
        }
    }

    /// Re-read the download store for this game and paint the state.
    pub fn refresh_download(&self) {
        let g = self.game.borrow();
        let Some(g) = g.as_ref() else { return };
        let dl = g.id.and_then(downloads::state);
        let downloading = dl.as_ref().map(|d| d.downloading).unwrap_or(false);
        self.dl_overlay.set_visible(downloading);
        if let Some(d) = &dl {
            self.dl_bar.set_fraction(d.progress.clamp(0.0, 1.0));
            self.dl_pct.set_label(&if d.progress > 0.0 { format!("{:.0}%", d.progress * 100.0) } else { "…".into() });
        }
        let (text, cls) = if let Some(d) = dl.as_ref().filter(|d| d.downloading) {
            (d.status.clone(), "action-downloading")
        } else if g.installed {
            ("▶ Play".to_string(), "action-installed")
        } else if g.in_library {
            ("⚠ Incomplete".to_string(), "action-incomplete")
        } else if bus::offline() {
            ("Not installed".to_string(), "action-offline")
        } else {
            (
                match g.download_size {
                    Some(b) if b > 0 => format!("↓ {}", format_bytes(b as u64)),
                    _ => "↓ Download".to_string(),
                },
                "action-download",
            )
        };
        self.action.set_label(&text);
        for c in ["action-downloading", "action-installed", "action-incomplete", "action-offline", "action-download"] {
            self.action.remove_css_class(c);
        }
        self.action.add_css_class(cls);
        // The dense card of the concept: the line only says what needs
        // attention (a download, an incomplete install); installed games
        // wear the accent outline, the rest say nothing.
        self.action.set_visible(matches!(cls, "action-downloading" | "action-incomplete"));
    }

    fn show_menu(self: &Rc<Self>, x: f64, y: f64) {
        let Some(game) = self.game.borrow().clone() else { return };
        let Some(id) = game.id else { return };
        let menu = gtk::Popover::builder().has_arrow(false).position(gtk::PositionType::Bottom).css_classes(["context-menu"]).build();
        menu.set_parent(&self.widget);
        menu.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        let items = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let downloading = downloads::is_downloading(id);

        let add = |label: &str, danger: bool, f: Box<dyn Fn()>| {
            let b = gtk::Button::builder().label(label).css_classes(["menu-item"]).build();
            if danger {
                b.add_css_class("danger");
            }
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            let m = menu.clone();
            b.connect_clicked(move |_| {
                m.popdown();
                f();
            });
            items.append(&b);
        };

        let card = self.clone();
        let g = game.clone();
        add("Details…", false, Box::new(move || (card.on_detail)(g.clone())));
        if game.installed && !bus::is_running(id) {
            let title = game.title.clone();
            add("▶ Play", false, Box::new(move || actions::play(id, title.clone(), Rc::new(|_| {}))));
        } else if bus::is_running(id) {
            add("■ Stop", false, Box::new(move || actions::stop(id)));
        } else if !game.installed && !downloading && !bus::offline() && game.game_torrent_index.is_some() {
            let g = game.clone();
            add(if game.in_library { "↓ Re-download" } else { "↓ Download" }, false, Box::new(move || actions::download(&g)));
        }
        if downloading {
            add("✕ Cancel download", false, Box::new(move || downloads::cancel(id)));
        }
        {
            let (g, w) = (game.clone(), self.widget.clone());
            add("Add to playlist…", false, Box::new(move || actions::add_to_playlist(&w, &g)));
        }
        if game.installed {
            let (g, w) = (game.clone(), self.widget.clone());
            add("Game settings…", false, Box::new(move || actions::game_settings(&w, &g)));
            let (g, w) = (game.clone(), self.widget.clone());
            add("↺ Reset game data", true, Box::new(move || actions::reset(&w, &g, Rc::new(|_| {}))));
        }
        if self.in_recent.get() {
            let title = game.title.clone();
            add("Remove from Recently played", false, Box::new(move || crate::ui::hidden::remove_from_recent(id, &title)));
        }
        {
            let title = game.title.clone();
            if crate::ui::hidden::is_hidden(id) {
                add("Unhide title", false, Box::new(move || crate::ui::hidden::unhide(id, &title)));
            } else {
                add("Hide title", false, Box::new(move || crate::ui::hidden::hide(id, &title)));
            }
        }
        if game.installed || game.in_library {
            let (g, w) = (game.clone(), self.widget.clone());
            add("Uninstall", true, Box::new(move || actions::uninstall_group(&w, &g, Rc::new(|_| {}))));
        }
        menu.set_child(Some(&items));
        menu.connect_closed(|m| {
            let m = m.clone();
            glib::idle_add_local_once(move || m.unparent());
        });
        menu.popup();
    }
}

fn badge(text: &str, cls: &str) -> gtk::Label {
    gtk::Label::builder().label(text).css_classes(["badge", cls]).build()
}
