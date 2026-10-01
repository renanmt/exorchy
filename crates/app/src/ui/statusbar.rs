//! The status bar under the library (the concept's footer): the app and its
//! version, how many games the current view holds (the library's own count
//! label, reparented here so its wording stays in one place), favourites,
//! playlists and enabled collections, then the keyboard hints. Counts follow
//! the bus; the hints and counts give way in a small window.

use std::rc::Rc;

use adw::prelude::*;
use exorchy_core::commands::{app_update, games, playlists};

use crate::app;
use crate::ui::bus;

pub struct StatusBar {
    pub widget: gtk::Box,
    /// The counts; hidden by the library's small-window breakpoint.
    pub details: gtk::Box,
    /// The key hints; hidden below the wide layout (they need the room).
    pub hints: gtk::Box,
    favorites: gtk::Label,
    playlists: gtk::Label,
    collections: gtk::Label,
}

/// Build the bar around the library's `count` label.
pub fn build(count: &gtk::Label) -> Rc<StatusBar> {
    let widget = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(14).css_classes(["statusbar"]).build();
    let brand = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(6).build();
    brand.append(&gtk::Label::builder().label("eXorchy").css_classes(["statusbar-brand"]).build());
    brand.append(&gtk::Label::builder().label(format!("v{}", app_update::current_version())).css_classes(["muted"]).build());
    widget.append(&brand);
    widget.append(&separator());
    if let Some(parent) = count.parent().and_downcast::<gtk::Box>() {
        parent.remove(count);
    }
    count.set_margin_start(0);
    count.set_margin_bottom(0);
    widget.append(count);

    let details = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(14).build();
    let favorites = gtk::Label::new(None);
    let playlists = gtk::Label::new(None);
    let collections = gtk::Label::new(None);
    for l in [&favorites, &playlists, &collections] {
        details.append(&separator());
        details.append(l);
    }
    widget.append(&details);
    let hints = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(14).hexpand(true).halign(gtk::Align::End).build();
    for (key, what) in [("Enter", "Play"), ("I", "Game info"), ("F", "Favorite"), ("/", "Search"), ("Esc", "Back")] {
        let hint = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(6).build();
        hint.append(&gtk::Label::builder().label(key).css_classes(["kbd"]).build());
        hint.append(&gtk::Label::builder().label(what).css_classes(["muted"]).build());
        hints.append(&hint);
    }
    widget.append(&hints);

    let bar = Rc::new(StatusBar { widget, details, hints, favorites, playlists, collections });
    refresh(&bar);
    let weak = Rc::downgrade(&bar);
    let again = move || {
        if let Some(b) = weak.upgrade() {
            refresh(&b);
        }
    };
    let a = again.clone();
    bus::on_favorite_changed(move |_| a());
    let a = again.clone();
    bus::on_playlists_changed(move |_| a());
    let a = again.clone();
    bus::on_collections_changed(move |_| a());
    bus::on_library_changed(move |_| again());
    bar
}

fn separator() -> gtk::Label {
    gtk::Label::builder().label("|").css_classes(["statusbar-sep"]).build()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", grouped(n), if n == 1 { one } else { many })
}

/// 12486 -> "12,486", as the concept writes counts.
pub fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn refresh(bar: &Rc<StatusBar>) {
    let core = app::core();
    let weak = Rc::downgrade(bar);
    app::spawn(
        async move {
            let favorites = games::get_games(core.state(), Some(1), Some(1), None, None, None, None, Some(true), None, None)
                .await
                .map(|l| l.total)
                .unwrap_or(0);
            let user_playlists = playlists::get_playlists(core.state())
                .await
                .map(|p| p.iter().filter(|p| p.kind != "curated").count())
                .unwrap_or(0);
            let collections = games::get_config(core.state(), "collections".into())
                .await
                .ok()
                .flatten()
                .map(|c| c.split(',').filter(|s| !s.trim().is_empty()).count())
                .unwrap_or(0);
            (favorites, user_playlists, collections)
        },
        move |(favorites, user_playlists, collections)| {
            let Some(bar) = weak.upgrade() else { return };
            bar.favorites.set_label(&plural(favorites, "favorite", "favorites"));
            bar.playlists.set_label(&plural(user_playlists, "playlist", "playlists"));
            bar.collections.set_label(&plural(collections, "collection", "collections"));
        },
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn counts_are_grouped_like_the_concept() {
        assert_eq!(super::grouped(7), "7");
        assert_eq!(super::grouped(1203), "1,203");
        assert_eq!(super::grouped(12486), "12,486");
        assert_eq!(super::grouped(1_000_000), "1,000,000");
        assert_eq!(super::plural(1, "favorite", "favorites"), "1 favorite");
    }
}
