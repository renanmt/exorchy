//! Hidden titles: the adult switch, a search to hide a title, and the hidden
//! list with Unhide. A hidden title leaves Browse and the shelves; an
//! installed one is still found by name and playable. The state lives in
//! `ui::hidden`; this page re-reads on `bus::on_visibility_changed`.

use std::rc::Rc;

use adw::prelude::*;
use exorchy_core::commands::games;
use exorchy_core::models::Game;

use super::widgets::{self, Ctx, Row};
use crate::app;
use crate::ui::{bus, hidden};

pub fn build(ctx: &Ctx) -> gtk::Widget {
    let page = widgets::page("Hidden titles");

    // ── Adult titles ──
    let adult = widgets::group("Adult titles", None);
    let (adult_row, adult_switch) = widgets::switch_row(
        "Show adult titles",
        "eXo tags some titles Adult. While this is off they are hidden everywhere, installed ones too, and the genre is not offered.",
        false,
    );
    adult.add(&adult_row.widget);
    page.add(&adult);
    {
        let sw = adult_switch.clone();
        app::spawn(async move { games::get_show_adult().await }, move |res| sw.set_quiet(res.unwrap_or(false)));
    }
    {
        let sw = adult_switch.clone();
        adult_switch.on_change(move |next| {
            let sw = sw.clone();
            hidden::set_show_adult(next, move |res| {
                if let Err(e) = res {
                    log::error!("settings: failed to save show_adult: {e}");
                    sw.set_quiet(!next);
                }
            });
        });
    }

    // ── Hide a title ──
    let find = widgets::group("Hide a title", Some("Search by name. Hiding keeps the game installed; it just stops being listed."));
    let search = gtk::SearchEntry::builder().placeholder_text("Game title…").css_classes(["search"]).build();
    let search_row = gtk::ListBoxRow::builder().activatable(false).selectable(false).css_classes(["settings-row"]).child(&search).build();
    find.add(&search_row);
    // Results sit right under the search; hidden while there are none.
    let results = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).visible(false).build();
    page.add(&find);
    page.add(&results);

    // ── The hidden list ──
    let list_group = widgets::group("Hidden titles", None);
    page.add(&list_group);
    let rows: Rc<std::cell::RefCell<Vec<gtk::Widget>>> = Rc::default();

    let reload_list: Rc<dyn Fn()> = {
        let (group, rows) = (list_group.clone(), rows.clone());
        Rc::new(move || {
            let core = app::core();
            let (group, rows) = (group.clone(), rows.clone());
            app::spawn(async move { games::get_hidden_games(core.state()).await }, move |res| {
                for w in rows.borrow_mut().drain(..) {
                    group.remove(&w);
                }
                let list = res.unwrap_or_default();
                group.set_title(&format!("Hidden titles · {}", list.len()));
                if list.is_empty() {
                    let note = widgets::note("No hidden titles. Hide one here, or from a game's menu.");
                    group.add(&note);
                    rows.borrow_mut().push(note.upcast());
                }
                for g in list {
                    let row = game_row(&g, true);
                    group.add(&row.widget);
                    rows.borrow_mut().push(row.widget.upcast());
                }
            });
        })
    };

    let run_search: Rc<dyn Fn()> = {
        let (search, results) = (search.clone(), results.clone());
        Rc::new(move || {
            let q = search.text().trim().to_string();
            while let Some(c) = results.first_child() {
                results.remove(&c);
            }
            if q.len() < 2 {
                results.set_visible(false);
                return;
            }
            let core = app::core();
            let results = results.clone();
            app::spawn(
                async move { games::get_games(core.state(), Some(1), Some(20), Some(q), None, Some("title".into()), None, None, None, None).await },
                move |res| {
                    while let Some(c) = results.first_child() {
                        results.remove(&c);
                    }
                    let found = res.map(|l| l.games).unwrap_or_default();
                    if found.is_empty() {
                        results.append(&gtk::ListBoxRow::builder().activatable(false).child(&widgets::note("No match.")).build());
                    }
                    for g in &found {
                        let hidden_now = g.id.is_some_and(hidden::is_hidden);
                        results.append(&game_row(g, hidden_now).widget);
                    }
                    results.set_visible(true);
                },
            );
        })
    };
    search.connect_search_changed({
        let run = run_search.clone();
        move |_| run()
    });

    reload_list();
    {
        let (alive, reload, run) = (ctx.alive.clone(), reload_list.clone(), run_search.clone());
        // Hidden elsewhere (a card menu, the Undo toast) or here: both lists follow.
        bus::on_visibility_changed(move |_| {
            // `ui::hidden` re-read its set before notifying: buttons read right.
            if alive.get() {
                reload();
                run();
            }
        });
    }

    page.upcast()
}

/// A title with Hide or Unhide.
fn game_row(g: &Game, is_hidden: bool) -> Row {
    let mut meta = Vec::new();
    if let Some(y) = g.year {
        meta.push(y.to_string());
    }
    if let Some(src) = g.torrent_source.as_deref() {
        meta.push(src.to_string());
    }
    if g.installed {
        meta.push("installed".into());
    }
    let button = widgets::button(if is_hidden { "Unhide" } else { "Hide" });
    let row = Row::new(&g.title).hint(&meta.join(" · ")).action(&button);
    if let Some(id) = g.id {
        let title = g.title.clone();
        button.connect_clicked(move |b| {
            b.set_sensitive(false);
            if is_hidden {
                hidden::unhide(id, &title);
            } else {
                hidden::hide(id, &title);
            }
        });
    }
    row
}
