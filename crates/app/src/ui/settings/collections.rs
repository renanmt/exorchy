//! Collections: one switch per eXo collection. The set lives in the
//! `collections` config key; a toggle re-runs every per-collection load in
//! dependency order (download managers, cover dirs, installed scan, grid).

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use exorchy_core::commands::{games, library, setup};

use super::widgets::{self, Ctx};
use super::{collection_hint, normalise_collections, parse_collections, plural, sort_by_display_order, BASE_COLLECTION};
use crate::app;
use crate::ui::{bus, covers};

struct State {
    enabled: Vec<String>,
    known: Vec<String>,
    switches: Vec<(String, widgets::Switch)>,
    busy: bool,
}

pub fn build(ctx: &Ctx) -> gtk::Widget {
    let page = widgets::page("Collections");
    let group = widgets::group(
        "Collections",
        Some("Only enabled collections appear in the library; their games and box art are downloaded only when enabled."),
    );
    let loading = widgets::note("Loading collections…");
    group.add(&loading);
    let footer = widgets::note("");
    footer.set_visible(false);
    page.add(&group);

    let state = Rc::new(RefCell::new(State { enabled: vec![], known: vec![], switches: vec![], busy: false }));

    let core = app::core();
    app::spawn(
        async move {
            let raw = games::get_config(core.state(), "collections".into()).await.ok().flatten();
            let mut available: Vec<(String, String, i64)> = setup::get_available_collections(core.state())
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|c| (c.id, c.display_name, c.game_count))
                .collect();
            let mut ids: Vec<String> = available.iter().map(|c| c.0.clone()).collect();
            sort_by_display_order(&mut ids);
            available.sort_by_key(|c| ids.iter().position(|i| *i == c.0).unwrap_or(usize::MAX));
            (raw, available)
        },
        {
            let (group, footer, state, ctx) = (group.clone(), footer.clone(), state.clone(), ctx.clone());
            move |(raw, available)| {
                group.remove(&loading);
                let known: Vec<String> = available.iter().map(|c| c.0.clone()).collect();
                let enabled = normalise_collections(&parse_collections(raw.as_deref()), &known);
                let mut switches = Vec::new();
                for (id, name, count) in &available {
                    let is_base = id == BASE_COLLECTION;
                    let (row, sw) = widgets::switch_row(name, &collection_hint(id, *count), is_base || enabled.contains(id));
                    if is_base {
                        sw.set_sensitive(false);
                    } else {
                        let (state, footer, ctx, id2) = (state.clone(), footer.clone(), ctx.clone(), id.clone());
                        sw.on_change(move |on| set_enabled(&ctx, &state, &footer, &id2, on));
                    }
                    group.add(&row.widget);
                    switches.push((id.clone(), sw));
                }
                group.add(&footer);
                let mut s = state.borrow_mut();
                s.enabled = enabled;
                s.known = known;
                s.switches = switches;
                drop(s);
                update_footer(&state, &footer);
            }
        },
    );

    page.upcast()
}

fn update_footer(state: &Rc<RefCell<State>>, footer: &gtk::Label) {
    let s = state.borrow();
    let text = if s.busy {
        "Applying… the library reloads when it is done.".to_string()
    } else if s.enabled.len() > 1 {
        format!("{} enabled.", plural(s.enabled.len() as i64, "collection"))
    } else {
        String::new()
    };
    footer.set_label(&text);
    footer.set_visible(!text.is_empty());
}

fn set_switches_sensitive(state: &Rc<RefCell<State>>, on: bool) {
    for (id, sw) in &state.borrow().switches {
        sw.set_sensitive(on && id != BASE_COLLECTION);
    }
}

/// Persist a new set and re-run every per-collection load: the download
/// managers, cover dirs, the installed scan and the grid. Rolls the
/// switches and the stored key back when a step fails.
fn set_enabled(ctx: &Ctx, state: &Rc<RefCell<State>>, footer: &gtk::Label, id: &str, on: bool) {
    let (previous, next) = {
        let s = state.borrow();
        let mut wanted = s.enabled.clone();
        if on {
            wanted.push(id.to_string());
        } else {
            wanted.retain(|c| c != id);
        }
        (s.enabled.clone(), normalise_collections(&wanted, &s.known))
    };
    if next == previous {
        return;
    }
    {
        let mut s = state.borrow_mut();
        s.enabled = next.clone();
        s.busy = true;
    }
    set_switches_sensitive(state, false);
    update_footer(state, footer);

    let core = app::core();
    let list = next.join(",");
    let (ctx, state, footer) = (ctx.clone(), state.clone(), footer.clone());
    app::spawn(
        async move {
            games::set_config(core.clone(), core.state(), "collections".into(), list).await?;
            setup::init_download_manager(core.state(), core.state(), core.state()).await?;
            let _ = library::scan_installed_games(core.state(), core.state(), None).await;
            Ok::<(), String>(())
        },
        move |res| {
            match res {
                Ok(()) => {
                    covers::load_dirs();
                    bus::notify_collections_changed();
                    if let Some(lib) = crate::ui::window::library() {
                        lib.fetch();
                    }
                    finish(&state, &footer);
                }
                Err(e) => {
                    let prev_list = previous.join(",");
                    {
                        let mut s = state.borrow_mut();
                        s.enabled = previous.clone();
                        for (cid, sw) in &s.switches {
                            sw.set_quiet(cid == BASE_COLLECTION || previous.contains(cid));
                        }
                    }
                    bus::toast_with("Could not change collections", Some(&e), None);
                    let core = app::core();
                    let (state, footer) = (state.clone(), footer.clone());
                    app::spawn(
                        async move { games::set_config(core.clone(), core.state(), "collections".into(), prev_list).await },
                        move |_| finish(&state, &footer),
                    );
                }
            }
            let _ = &ctx;
        },
    );
}

fn finish(state: &Rc<RefCell<State>>, footer: &gtk::Label) {
    state.borrow_mut().busy = false;
    set_switches_sensitive(state, true);
    update_footer(state, footer);
}
