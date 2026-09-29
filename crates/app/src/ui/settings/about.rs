//! About: version, author, the log folder, credits (eXoDOS, Exodium by Thomas
//! Vollstädt) and the factory reset.

use adw::prelude::*;
use exorchy_core::commands::{games, setup};

use super::storage;
use super::widgets::{self, Ctx, Row};
use crate::app;
use crate::ui::dialogs;

pub fn build(ctx: &Ctx) -> gtk::Widget {
    let page = widgets::page("About");

    let header = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).halign(gtk::Align::Center).css_classes(["about-logo"]).build();
    // The large wordmark is ~300 px; a narrow page gets the smaller one.
    let wide = crate::ui::logo::ascii(4.0);
    let small = crate::ui::logo::ascii(2.5);
    for logo in [&wide, &small] {
        logo.set_halign(gtk::Align::Center);
        header.append(logo);
    }
    widgets::follow_narrow(&header, move |_, narrow| {
        wide.set_visible(!narrow);
        small.set_visible(narrow);
    });
    header.append(
        &gtk::Label::builder()
            .label("THE EXODOS LAUNCHER FOR OMARCHY")
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["tagline", "small"])
            .build(),
    );
    page.add(&header);

    let app_group = widgets::group("eXorchy", None);
    app_group.add(&Row::new("Version").value(env!("CARGO_PKG_VERSION")).hint("An eXo launcher for Omarchy (Arch + Hyprland).").widget);
    app_group.add(&Row::new("Author").value("Renan Tonheiro").hint("eXorchy, the Omarchy port of Exodium.").widget);
    app_group.add(&updates_row());
    let open_btn = widgets::button("Open");
    let log_row = Row::new("Log folder").hint("Share exorchy.log when a download stalls or the app misbehaves.").selectable().code().action(&open_btn);
    app_group.add(&log_row.widget);
    page.add(&app_group);

    {
        let log_row = log_row.clone();
        app::spawn(async move { setup::get_log_dir().await }, move |res| {
            if let Ok(dir) = res {
                log_row.set_value(&dir);
            }
        });
    }
    open_btn.connect_clicked({
        let log_row = log_row.clone();
        move |_| {
            log_row.set_hint("Share exorchy.log when a download stalls or the app misbehaves.");
            let core = app::core();
            let log_row = log_row.clone();
            app::spawn(async move { setup::open_log_folder(core.clone()).await }, move |res| {
                if let Err(e) = res {
                    log_row.set_error(&format!("Could not open log folder: {e}"));
                }
            });
        }
    });

    let credits = widgets::group("Credits", Some("eXorchy is derived from Exodium by Thomas Vollstädt - nearly everything that makes it work is his."));
    credits.add(
        &Row::new("eXoDOS")
            .hint("The collections, their metadata and the emulator configs come from the eXo project.")
            .action(&widgets::link("https://www.retro-exo.com/", "retro-exo.com"))
            .widget,
    );
    credits.add(
        &Row::new("Exodium")
            .hint(
                "Created by Thomas Vollstädt (MIT licence); eXorchy is its Omarchy port. Visit his GitHub page to support his work, \
                 or to get Exodium for other operating systems.",
            )
            .action(&widgets::link("https://github.com/tvollstaedt/exodium", "GitHub"))
            .widget,
    );
    page.add(&credits);

    let danger = widgets::group("Danger zone", None);
    danger.add_css_class("danger-zone");
    let reset_btn = widgets::danger_button("Reset…");
    let reset_row = Row::new("Factory reset").hint("Clears all data and returns to setup.").action(&reset_btn);
    danger.add(&reset_row.widget);
    page.add(&danger);
    reset_btn.connect_clicked({
        let ctx = ctx.clone();
        move |_| confirm_reset(&ctx)
    });

    page.upcast()
}

/// Updates: what the last check found, "Check now", and Update when a newer
/// release exists and this copy can update itself (`ui::updates`).
fn updates_row() -> gtk::ListBoxRow {
    let check = widgets::button("Check now");
    let update = widgets::button("Update");
    let row = Row::new("Updates").action(&check);
    row.add_action(&update);
    let show = {
        let (row, update) = (row.clone(), update.clone());
        move || {
            let (text, can_update) = crate::ui::updates::status_text();
            row.set_value(&text);
            update.set_visible(can_update);
            row.set_hint(if crate::ui::updates::available().is_some() && !can_update {
                "This copy was not installed as the pacman package: update it the way you installed it."
            } else {
                ""
            });
        }
    };
    show();
    check.connect_clicked({
        let row = row.clone();
        move |b| {
            b.set_sensitive(false);
            row.set_value("Checking…");
            let (b, row, show) = (b.clone(), row.clone(), show.clone());
            crate::ui::updates::check_now(move |res| {
                b.set_sensitive(true);
                match res {
                    Ok(_) => show(),
                    Err(e) => row.set_error(&format!("Could not check: {e}")),
                }
            });
        }
    });
    update.connect_clicked(|_| crate::ui::updates::start_update());
    row.widget
}

/// The reset dialog: the database and settings go; the game folder only
/// when asked for, with a warning that cannot be missed.
fn confirm_reset(ctx: &Ctx) {
    let core = app::core();
    let ctx = ctx.clone();
    app::spawn(async move { games::get_config(core.state(), "data_dir".into()).await.ok().flatten() }, move |data_dir| {
        let dialog = adw::AlertDialog::builder()
            .heading("Factory reset")
            .body("Clears the eXorchy database and all settings. Your downloaded game files stay on disk and can be re-imported later.")
            .build();
        let extra = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
        let label = match &data_dir {
            Some(d) if !d.is_empty() => format!("Also delete game folder ({d})"),
            _ => "Also delete game folder".into(),
        };
        let check = gtk::CheckButton::with_label(&label);
        let warning = widgets::note("This will permanently delete all downloaded game files. This cannot be undone.");
        warning.add_css_class("error");
        warning.set_visible(false);
        check.connect_toggled({
            let warning = warning.clone();
            move |c| warning.set_visible(c.is_active())
        });
        extra.append(&check);
        extra.append(&warning);
        dialog.set_extra_child(Some(&extra));
        dialog.add_responses(&[("cancel", "Cancel"), ("reset", "Reset")]);
        dialog.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let ctx2 = ctx.clone();
        dialog.connect_response(None, move |_, r| {
            if r == "reset" {
                run_reset(&ctx2, check.is_active());
            }
        });
        dialog.present(Some(&ctx.window));
    });
}

fn run_reset(ctx: &Ctx, delete_game_data: bool) {
    // Leave Settings first, then show setup in place of the library.
    ctx.close();
    let core = app::core();
    let window = ctx.window.clone();
    app::spawn(
        async move { setup::factory_reset(core.state(), core.state(), core.state(), delete_game_data).await },
        move |res| match res {
            Ok(()) => {
                storage::reset_cache();
                crate::ui::window::restart_to_setup();
            }
            Err(e) => dialogs::error(&window, "Reset failed", &format!("Reset failed: {e}")),
        },
    );
}
