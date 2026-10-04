//! General: the game folder, the installed-games rescan, game defaults
//! (fullscreen) and each emulator's graphics filter.

use adw::prelude::*;
use exorchy_core::commands::{games, library, library_location, scummvm, setup};
use exorchy_core::launchers::dosbox;

use super::widgets::{self, Ctx, Row};
use super::{plural, storage};
use crate::app;
use crate::ui::bus;

pub fn build(ctx: &Ctx) -> gtk::Widget {
    let page = widgets::page("General");

    // ── Library ──
    let lib = widgets::group("Library", None);
    let move_btn = widgets::button("Move…");
    let locate = widgets::button("Locate…");
    locate.set_visible(false);
    let folder = Row::new("Library folder")
        .value("Not set")
        .hint("Your games and eXorchy's files. Moving takes them along; eXorchy restarts to do it.")
        .selectable()
        .code()
        .action(&move_btn);
    folder.add_action(&locate);
    lib.add(&folder.widget);
    let scan = widgets::BusyButton::new("Scan", "Scanning…");
    let installed = Row::new("Installed games").hint("Re-scan the disk for games that are already there.").action(&scan.widget);
    lib.add(&installed.widget);
    let (start_row, start_library) = widgets::switch_row("Open in My Library", "Start on your installed games instead of Browse.", false);
    lib.add(&start_row.widget);
    let (updates_row, check_updates) = widgets::switch_row("Check for updates", "Look for a new eXorchy release on GitHub at start and every few hours.", true);
    lib.add(&updates_row.widget);
    page.add(&lib);

    // ── Game defaults ──
    let defaults = widgets::group("Game defaults", Some("Applied on every launch, on top of eXoDOS's own configs."));
    let (fs_row, fullscreen) = widgets::switch_row("Launch in fullscreen", "Alt+Enter still toggles at runtime.", false);
    defaults.add(&fs_row.widget);
    page.add(&defaults);

    // ── Graphics filters ── one per emulator; a game's settings override it.
    let looks = widgets::group("Graphics filters", Some("How every game looks, per emulator: a CRT monitor of the time, or smoothed pixel art. A game's own settings can override it."));
    filter_row(&looks, "DOSBox Staging", "Most DOS and Windows 3.x games. Automatic CRT follows the game's video mode.", "global_glshader", dosbox::STAGING_SHADERS.to_vec(), |v| Some(dosbox::staging_shader(v)));
    let mut dosx = vec![("", "Off (eXo's own)")];
    dosx.extend_from_slice(dosbox::DOSBOX_X_FILTERS);
    filter_row(&looks, "DOSBox-X", "DOS games under DOSBox-X. Windows 9x games set theirs per game.", "dosx_filter", dosx, |v| v.map(str::to_string));
    let mut svm = vec![("", "Off (sharp pixels)")];
    svm.extend(scummvm::FILTERS.iter().map(|f| (f.0, f.1)));
    filter_row(&looks, "ScummVM", "Smooths the pixel art of ScummVM games, HQ2x for example.", "svm_filter", svm, |v| v.map(str::to_string));
    page.add(&looks);


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
                (get("data_dir").await, get("default_fullscreen").await),
                (get("start_tab").await, get("update_check").await),
            )
        },
        {
            let (folder, fullscreen, start_library, check_updates) =
                (folder.clone(), fullscreen.clone(), start_library.clone(), check_updates.clone());
            move |((dir, fs), (start, upd))| {
                start_library.set_quiet(start.as_deref() == Some("library"));
                check_updates.set_quiet(upd.as_deref() != Some("0"));
                folder.set_value(dir.as_deref().filter(|d| !d.is_empty()).unwrap_or("Not set"));
                fullscreen.set_quiet(fs.as_deref() == Some("fullscreen"));
            }
        },
    );

    bind_toggle(&fullscreen, "default_fullscreen", "fullscreen", "window");
    bind_toggle(&start_library, "start_tab", "library", "browse");
    bind_toggle(&check_updates, "update_check", "1", "0");

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

    // Locate is for a library that is gone from its folder, and only then.
    let core = app::core();
    app::spawn(async move { library_location::library_status(core.state()).await }, {
        let (folder, locate) = (folder.clone(), locate.clone());
        move |status| {
            if let Ok(Some(s)) = status {
                if !s.found {
                    locate.set_visible(true);
                    folder.set_error("Not found there. Locate it if you moved it or its drive is mounted elsewhere.");
                }
            }
        }
    });
    move_btn.connect_clicked({
        let window = ctx.window.clone();
        move |_| crate::ui::library_location::move_from_settings(&window)
    });
    locate.connect_clicked({
        let ctx = ctx.clone();
        move |_| {
            let ctx2 = ctx.clone();
            crate::ui::library_location::locate_from_settings(&ctx.window, move || library_located(&ctx2))
        }
    });

    page.upcast()
}

/// Save a switch as `on`/`off`; a refused write puts the switch back.
/// A dropdown of one emulator's filters, stored under `key` in the global
/// config. `current` turns the stored value into an option's id (Staging's
/// unset key means its default shader, for one). Saving starts once the
/// stored value is shown, so loading never writes it back.
fn filter_row(
    group: &adw::PreferencesGroup,
    label: &str,
    hint: &str,
    key: &'static str,
    options: Vec<(&'static str, &'static str)>,
    current: fn(Option<&str>) -> Option<String>,
) {
    let drop = gtk::DropDown::from_strings(&options.iter().map(|(_, l)| *l).collect::<Vec<_>>());
    drop.add_css_class("drop");
    group.add(&Row::new(label).hint(hint).action(&drop).widget);
    let core = app::core();
    app::spawn(async move { games::get_config(core.state(), key.into()).await.ok().flatten() }, move |stored| {
        let id = current(stored.as_deref());
        let idx = options.iter().position(|(v, _)| Some(*v) == id.as_deref()).unwrap_or(0);
        drop.set_selected(idx as u32);
        drop.connect_selected_notify(move |d| {
            let Some((value, _)) = options.get(d.selected() as usize).copied() else { return };
            let core = app::core();
            app::spawn(async move { games::set_config(core.clone(), core.state(), key.into(), value.into()).await }, move |res| {
                if let Err(e) = res {
                    log::error!("settings: failed to save {key}: {e}");
                }
            });
        });
    });
}

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

/// The library was found somewhere else: restart what derives from its
/// folder; the rescan's count answers "is everything there?".
fn library_located(ctx: &Ctx) {
    storage::reset_cache();
    let core = app::core();
    let ctx = ctx.clone();
    app::spawn(
        async move {
            // The managers must point at the new folder before the scan reads them.
            setup::init_download_manager(core.state(), core.state(), core.state()).await?;
            library::scan_installed_games(core.state(), core.state(), Some(true)).await
        },
        move |res: Result<usize, String>| {
            match res {
                Ok(n) => bus::toast(&format!("Library found: {} installed", plural(n as i64, "game"))),
                Err(e) => bus::toast_with("The library was found, but the rescan failed", Some(&e), None),
            }
            ctx.close();
            crate::ui::window::reinit_library();
        },
    );
}
