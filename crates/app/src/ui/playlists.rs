//! Playlists (the web UI's `PlaylistMenu`, `PlaylistNameDialog` and the
//! shelf menu in `Library.tsx`): the "Add to playlist…" picker with
//! membership checks, and the management dialog. Both list the user's
//! playlists with rename / delete; curated playlists ship with the
//! catalogue and are read-only, so they only appear (greyed) in management.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use exorchy_core::commands::playlists;
use exorchy_core::models::{Game, Playlist};
use gtk::glib;
use adw::prelude::*;

use crate::app;
use crate::ui::{bus, dialogs};

/// One dialog, two modes: with a game it is the picker (checkboxes toggle
/// membership), without one it manages the lists.
struct PlaylistDialog {
    dialog: adw::Dialog,
    list: gtk::ListBox,
    empty: gtk::Label,
    game_id: Option<i64>,
    /// Playlists the game is in. Once the user toggles anything the
    /// (possibly still in-flight) initial fetch must not overwrite it.
    member: RefCell<HashSet<i64>>,
    touched: Cell<bool>,
    /// A failed toggle flips the check back; the handler must not treat
    /// that as a second request.
    reverting: Cell<bool>,
}

/// "Add to playlist…" for a game.
pub fn pick_for_game(parent: &impl IsA<gtk::Widget>, game: &Game) {
    let Some(game_id) = game.id else { return };
    let d = build(parent, "Add to playlist", Some(&game.title), Some(game_id));
    let (core, d2) = (app::core(), d.clone());
    app::spawn(async move { playlists::get_game_playlists(core.state(), game_id).await }, move |res| {
        if let Ok(ids) = res {
            if !d2.touched.get() {
                d2.member.replace(ids.into_iter().collect());
                reload(&d2);
            }
        }
    });
    reload(&d);
}

/// The management dialog: rename / delete the user's playlists, create new
/// ones, see the curated ones. For the library page's playlist filter.
#[allow(dead_code)]
pub fn manage(parent: &impl IsA<gtk::Widget>) {
    let d = build(parent, "Playlists", None, None);
    reload(&d);
}

fn build(parent: &impl IsA<gtk::Widget>, title: &str, subtitle: Option<&str>, game_id: Option<i64>) -> Rc<PlaylistDialog> {
    let dialog = adw::Dialog::builder().title(title).content_width(440).build();
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).margin_start(16).margin_end(16).margin_top(4).margin_bottom(16).css_classes(["dialog-body"]).build();
    if let Some(s) = subtitle {
        body.append(&gtk::Label::builder().label(s).xalign(0.0).wrap(true).css_classes(["secondary"]).build());
    }
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["playlist-list"]).build();
    body.append(&list);
    let empty = gtk::Label::builder().label("No playlists yet - create one below.").xalign(0.0).css_classes(["muted"]).visible(false).build();
    body.append(&empty);
    let new_btn = gtk::Button::builder().label("＋ New playlist…").halign(gtk::Align::Start).css_classes(["btn"]).build();
    body.append(&new_btn);
    tv.set_content(Some(&body));
    dialog.set_child(Some(&tv));
    dialog.present(Some(parent));

    let d = Rc::new(PlaylistDialog {
        dialog,
        list,
        empty,
        game_id,
        member: RefCell::new(HashSet::new()),
        touched: Cell::new(false),
        reverting: Cell::new(false),
    });
    new_btn.connect_clicked(glib::clone!(#[weak] d, move |_| {
        d.touched.set(true);
        name_dialog(&d.dialog, NameMode::Create, d.game_id, Rc::new(glib::clone!(#[weak] d, move |id| {
            if id.is_some() && d.game_id.is_some() {
                if let Some(id) = id {
                    d.member.borrow_mut().insert(id);
                }
            }
            reload(&d);
        })));
    }));
    d
}

fn reload(d: &Rc<PlaylistDialog>) {
    let core = app::core();
    app::spawn(async move { playlists::get_playlists(core.state()).await }, glib::clone!(#[weak] d, move |res| {
        let lists = res.unwrap_or_default();
        render(&d, &lists);
    }));
}

fn render(d: &Rc<PlaylistDialog>, lists: &[Playlist]) {
    d.list.remove_all();
    let user: Vec<&Playlist> = lists.iter().filter(|p| p.kind == "user").collect();
    let curated: Vec<&Playlist> = lists.iter().filter(|p| p.kind != "user").collect();
    d.empty.set_visible(user.is_empty());
    for p in &user {
        d.list.append(&user_row(d, p));
    }
    // The picker offers only what can change; management shows the curated
    // lists so their existence and size are visible somewhere.
    if d.game_id.is_none() && !curated.is_empty() {
        let head = gtk::Label::builder().label("Curated by eXo").xalign(0.0).css_classes(["muted", "small", "playlist-section"]).build();
        head.set_margin_top(8);
        d.list.append(&head);
        for p in &curated {
            let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).css_classes(["playlist-row"]).build();
            row.append(&gtk::Label::builder().label(&p.name).xalign(0.0).hexpand(true).wrap(true).css_classes(["secondary"]).build());
            row.append(&gtk::Label::builder().label(count_label(p.game_count)).css_classes(["muted", "small"]).build());
            row.append(&gtk::Label::builder().label("read-only").css_classes(["badge"]).build());
            d.list.append(&row);
        }
    }
}

fn count_label(n: i64) -> String {
    if n == 1 {
        "1 game".into()
    } else {
        format!("{n} games")
    }
}

fn user_row(d: &Rc<PlaylistDialog>, p: &Playlist) -> gtk::Box {
    let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).css_classes(["playlist-row"]).build();
    let (id, name) = (p.id, p.name.clone());
    if let Some(game_id) = d.game_id {
        let check = gtk::CheckButton::builder().label(&name).hexpand(true).active(d.member.borrow().contains(&id)).build();
        let (dd, n) = (d.clone(), name.clone());
        check.connect_toggled(move |c| {
            if dd.reverting.get() {
                return;
            }
            toggle(&dd, c, id, game_id, &n);
        });
        row.append(&check);
    } else {
        row.append(&gtk::Label::builder().label(&name).xalign(0.0).hexpand(true).wrap(true).build());
    }
    row.append(&gtk::Label::builder().label(count_label(p.game_count)).css_classes(["muted", "small"]).build());
    let rename = gtk::Button::builder().icon_name("document-edit-symbolic").css_classes(["btn", "icon", "ghost"]).tooltip_text("Rename").build();
    let pl = p.clone();
    rename.connect_clicked(glib::clone!(#[weak] d, move |_| {
        d.touched.set(true);
        name_dialog(&d.dialog, NameMode::Rename(pl.clone()), None, Rc::new(glib::clone!(#[weak] d, move |_| reload(&d))));
    }));
    row.append(&rename);
    let delete = gtk::Button::builder().icon_name("user-trash-symbolic").css_classes(["btn", "icon", "ghost", "danger"]).tooltip_text("Delete playlist").build();
    delete.connect_clicked(glib::clone!(#[weak] d, move |_| {
        d.touched.set(true);
        confirm_delete(&d, id, name.clone());
    }));
    row.append(&delete);
    row
}

/// Optimistic: the check already flipped; revert it if the backend refuses.
fn toggle(d: &Rc<PlaylistDialog>, check: &gtk::CheckButton, playlist_id: i64, game_id: i64, name: &str) {
    d.touched.set(true);
    let member = check.is_active();
    if member {
        d.member.borrow_mut().insert(playlist_id);
    } else {
        d.member.borrow_mut().remove(&playlist_id);
    }
    let core = app::core();
    let (name, check) = (name.to_string(), check.clone());
    app::spawn(
        async move { playlists::set_playlist_membership(core.state(), playlist_id, game_id, member).await },
        glib::clone!(#[weak] d, move |res| match res {
            Ok(()) => {
                bus::toast(&if member { format!("Added to \"{name}\"") } else { format!("Removed from \"{name}\"") });
                bus::notify_playlists_changed();
                bus::notify_library_changed(game_id);
            }
            Err(e) => {
                bus::toast_with(&format!("Couldn't update \"{name}\""), Some(&e), None);
                if member {
                    d.member.borrow_mut().remove(&playlist_id);
                } else {
                    d.member.borrow_mut().insert(playlist_id);
                }
                d.reverting.set(true);
                check.set_active(!member);
                d.reverting.set(false);
            }
        }),
    );
}

fn confirm_delete(d: &Rc<PlaylistDialog>, id: i64, name: String) {
    dialogs::confirm(
        &d.dialog,
        "Delete playlist",
        &format!("Delete \"{name}\"? The games stay in your library."),
        "Delete",
        true,
        glib::clone!(#[weak] d, move || {
            let core = app::core();
            app::spawn(async move { playlists::delete_playlist(core.state(), id).await }, glib::clone!(#[weak] d, move |res| {
                match res {
                    Ok(()) => {
                        bus::toast(&format!("Deleted \"{name}\""));
                        d.member.borrow_mut().remove(&id);
                        bus::notify_playlists_changed();
                        if let Some(g) = d.game_id {
                            bus::notify_library_changed(g);
                        }
                    }
                    Err(e) => bus::toast_with(&format!("Couldn't delete \"{name}\""), Some(&e), None),
                }
                reload(&d);
            }));
        }),
    );
}

enum NameMode {
    Create,
    Rename(Playlist),
}

/// The create / rename dialog. Create with a `game_id` also adds that game
/// to the fresh playlist. `done` gets the created playlist's id (None on
/// rename); the dialog stays open, error inline, when the backend refuses
/// (duplicate name).
fn name_dialog(parent: &impl IsA<gtk::Widget>, mode: NameMode, game_id: Option<i64>, done: Rc<dyn Fn(Option<i64>)>) {
    let (title, initial, verb, rename_id) = match &mode {
        NameMode::Create => ("New playlist", String::new(), "Create", None),
        NameMode::Rename(p) => ("Rename playlist", p.name.clone(), "Rename", Some(p.id)),
    };
    let dialog = adw::Dialog::builder().title(title).content_width(400).build();
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).margin_start(16).margin_end(16).margin_top(4).margin_bottom(16).build();
    let entry = gtk::Entry::builder().placeholder_text("Playlist name").max_length(80).text(&initial).css_classes(["field"]).build();
    body.append(&entry);
    let error = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["danger", "small"]).visible(false).build();
    body.append(&error);
    let buttons = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).halign(gtk::Align::End).build();
    let cancel = gtk::Button::builder().label("Cancel").css_classes(["btn"]).build();
    let save = gtk::Button::builder().label(verb).css_classes(["btn", "primary"]).sensitive(!initial.trim().is_empty()).build();
    buttons.append(&cancel);
    buttons.append(&save);
    body.append(&buttons);
    tv.set_content(Some(&body));
    dialog.set_child(Some(&tv));
    dialog.present(Some(parent));
    dialog.set_focus(Some(&entry));
    entry.select_region(0, -1);

    cancel.connect_clicked(glib::clone!(#[weak] dialog, move |_| { dialog.close(); }));
    entry.connect_changed(glib::clone!(#[weak] save, move |e| save.set_sensitive(!e.text().trim().is_empty())));

    let saving = Rc::new(Cell::new(false));
    let submit: Rc<dyn Fn()> = {
        let (dialog, entry, error, save, saving) = (dialog.clone(), entry.clone(), error.clone(), save.clone(), saving.clone());
        Rc::new(move || {
            let name = entry.text().trim().to_string();
            if name.is_empty() || saving.get() {
                return;
            }
            saving.set(true);
            error.set_visible(false);
            save.set_sensitive(false);
            save.set_label("Saving…");
            let core = app::core();
            let (n, verb) = (name.clone(), verb);
            let done = done.clone();
            let (dialog, error, save, saving) = (dialog.clone(), error.clone(), save.clone(), saving.clone());
            app::spawn(
                async move {
                    match rename_id {
                        Some(id) => playlists::rename_playlist(core.state(), id, n).await.map(|_| None),
                        None => {
                            let id = playlists::create_playlist(core.state(), n).await?;
                            if let Some(g) = game_id {
                                playlists::set_playlist_membership(core.state(), id, g, true).await?;
                            }
                            Ok(Some(id))
                        }
                    }
                },
                move |res: Result<Option<i64>, String>| {
                    saving.set(false);
                    save.set_sensitive(true);
                    save.set_label(verb);
                    match res {
                        Ok(created) => {
                            bus::toast(&match (rename_id, game_id) {
                                (Some(_), _) => format!("Renamed to \"{name}\""),
                                (None, Some(_)) => format!("Created \"{name}\" and added the game"),
                                (None, None) => format!("Playlist \"{name}\" created"),
                            });
                            bus::notify_playlists_changed();
                            if let Some(g) = game_id {
                                bus::notify_library_changed(g);
                            }
                            dialog.close();
                            done(created);
                        }
                        Err(e) => {
                            error.set_label(&e);
                            error.set_visible(true);
                        }
                    }
                },
            );
        })
    };
    let s = submit.clone();
    save.connect_clicked(move |_| s());
    entry.connect_activate(move |_| submit());
}
