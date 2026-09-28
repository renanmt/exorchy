//! General: the game folder, the installed-games rescan, game defaults
//! (CRT shader, fullscreen) and the music preferences.

use adw::prelude::*;
use exorchy_core::commands::{games, library, setup};

use super::widgets::{self, Ctx, Row};
use super::{plural, storage};
use crate::app;
use crate::ui::{bus, dialogs};

pub fn build(ctx: &Ctx) -> gtk::Widget {
    let page = widgets::page("General");

    // ── Library ──
    let lib = widgets::group("Library", None);
    let change = widgets::button("Change…");
    let folder = Row::new("Game folder")
        .value("Not set")
        .hint("Points eXorchy at an existing folder - nothing is moved.")
        .selectable()
        .code()
        .action(&change);
    lib.add(&folder.widget);
    let scan = widgets::BusyButton::new("Scan", "Scanning…");
    let installed = Row::new("Installed games").hint("Re-scan the disk for games that are already there.").action(&scan.widget);
    lib.add(&installed.widget);
    let (start_row, start_library) = widgets::switch_row("Open in My Library", "Start on your installed games instead of Browse.", false);
    lib.add(&start_row.widget);
    page.add(&lib);

    // ── Game defaults ──
    let defaults = widgets::group("Game defaults", Some("Applied on every launch, on top of eXoDOS's own configs."));
    let (crt_row, crt) = widgets::switch_row("Auto CRT shaders", "A CRT shader matched to the game's video mode. DOSBox ECE (Windows) has none.", true);
    let (fs_row, fullscreen) = widgets::switch_row("Launch in fullscreen", "Alt+Enter still toggles at runtime.", false);
    defaults.add(&crt_row.widget);
    defaults.add(&fs_row.widget);
    page.add(&defaults);

    // ── Music ──
    let music = widgets::group("Music", None);
    let (ap_row, autoplay) = widgets::switch_row("Play theme music", "Starts a game's theme when you open its details.", true);
    let (ct_row, continuous) = widgets::switch_row("Continue with the next theme", "When a theme ends, the next one plays.", true);
    music.add(&ap_row.widget);
    music.add(&ct_row.widget);
    page.add(&music);

    // Game defaults mirror launch_game's own defaults until the load lands.
    let core = app::core();
    app::spawn(
        async move {
            let get = |k: &str| {
                let c = core.clone();
                let k = k.to_string();
                async move { games::get_config(c.state(), k).await.ok().flatten() }
            };
            (
                (get("data_dir").await, get("global_glshader").await, get("default_fullscreen").await, get("music_autoplay").await, get("music_continuous").await),
                get("start_tab").await,
            )
        },
        {
            let (folder, crt, fullscreen, autoplay, continuous, start_library) =
                (folder.clone(), crt.clone(), fullscreen.clone(), autoplay.clone(), continuous.clone(), start_library.clone());
            move |((dir, shader, fs, ap, ct), start)| {
                start_library.set_quiet(start.as_deref() == Some("library"));
                folder.set_value(dir.as_deref().filter(|d| !d.is_empty()).unwrap_or("Not set"));
                crt.set_quiet(shader.is_none() || shader.as_deref() == Some("crt-auto"));
                fullscreen.set_quiet(fs.as_deref() == Some("fullscreen"));
                autoplay.set_quiet(ap.is_none() || ap.as_deref() == Some("1"));
                continuous.set_quiet(ct.is_none() || ct.as_deref() == Some("1"));
            }
        },
    );

    bind_toggle(&crt, "global_glshader", "crt-auto", "default");
    bind_toggle(&fullscreen, "default_fullscreen", "fullscreen", "window");
    bind_toggle(&autoplay, "music_autoplay", "1", "0");
    bind_toggle(&continuous, "music_continuous", "1", "0");
    bind_toggle(&start_library, "start_tab", "library", "browse");

    scan.widget.connect_clicked({
        let (scan, installed) = (scan.clone(), installed.clone());
        move |_| {
            scan.set_busy(true);
            installed.set_hint("Re-scan the disk for games that are already there.");
            let core = app::core();
            let (scan, installed) = (scan.clone(), installed.clone());
            app::spawn(async move { library::scan_installed_games(core.state(), core.state(), Some(true)).await }, move |res| {
                scan.set_busy(false);
                match res {
                    Ok(n) => {
                        let msg = format!("{} marked as installed", plural(n as i64, "game"));
                        installed.set_hint(&msg);
                        bus::toast(&msg);
                        if let Some(lib) = crate::ui::window::library() {
                            lib.fetch();
                            lib.refresh_shelves();
                        }
                    }
                    Err(e) => installed.set_error(&format!("Error: {e}")),
                }
            });
        }
    });

    change.connect_clicked({
        let ctx = ctx.clone();
        let folder = folder.clone();
        move |_| change_data_dir(&ctx, &folder)
    });

    page.upcast()
}

/// Save a switch as `on`/`off`; a refused write puts the switch back.
fn bind_toggle(sw: &widgets::Switch, key: &'static str, on: &'static str, off: &'static str) {
    let s = sw.clone();
    sw.on_change(move |next| {
        let core = app::core();
        let s = s.clone();
        let value = if next { on } else { off }.to_string();
        app::spawn(async move { games::set_config(core.clone(), core.state(), key.into(), value).await }, move |res| {
            if let Err(e) = res {
                log::error!("settings: failed to save {key}: {e}");
                s.set_quiet(!next);
            }
        });
    });
}

/// Change points at a folder, it never moves one - so the case worth
/// catching is an empty target chosen by someone who meant to relocate.
fn change_data_dir(ctx: &Ctx, folder: &Row) {
    let ctx = ctx.clone();
    let folder = folder.clone();
    let window = ctx.window.clone();
    dialogs::pick_folder(&window, "Select new data directory", move |selected| {
        let Some(selected) = selected else { return };
        let core = app::core();
        let sel = selected.clone();
        app::spawn(
            async move {
                let current = games::get_config(core.state(), "data_dir".into()).await.ok().flatten().unwrap_or_default();
                let target_empty = setup::data_dir_is_empty(sel.clone()).await.unwrap_or(false);
                let current_empty = if current.is_empty() { true } else { setup::data_dir_is_empty(current.clone()).await.unwrap_or(true) };
                (current, target_empty, current_empty)
            },
            move |(current, target_empty, current_empty)| {
                if target_empty && !current_empty {
                    let (ctx2, folder2, sel2) = (ctx.clone(), folder.clone(), selected.clone());
                    dialogs::confirm(
                        &ctx.window,
                        "That folder is empty",
                        &format!("eXorchy will look for games in {selected}, but it does not move anything there. Your downloaded games stay in {current} and keep using that space. Use the empty folder anyway?"),
                        "Use it anyway",
                        false,
                        move || apply_data_dir(&ctx2, &folder2, sel2),
                    );
                } else {
                    apply_data_dir(&ctx, &folder, selected);
                }
            },
        );
    });
}

/// Persist the dir and rebuild what derives from it; the rescan's count
/// answers "did it find my games?".
fn apply_data_dir(ctx: &Ctx, folder: &Row, selected: String) {
    folder.set_value(&selected);
    storage::reset_cache();
    let core = app::core();
    let ctx = ctx.clone();
    app::spawn(
        async move {
            games::set_config(core.clone(), core.state(), "data_dir".into(), selected).await?;
            // The managers must point at the new folder before the scan reads them.
            setup::init_download_manager(core.state(), core.state(), core.state()).await?;
            library::scan_installed_games(core.state(), core.state(), Some(true)).await
        },
        move |res: Result<usize, String>| {
            match res {
                Ok(n) => bus::toast(&format!("{} found in the new folder", plural(n as i64, "game"))),
                Err(e) => bus::toast_with("No games found in that folder", Some(&e), None),
            }
            ctx.close();
            crate::ui::window::reinit_library();
        },
    );
}
