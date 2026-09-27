//! About: version, the log folder, credits (eXoDOS, Exodium by Thomas
//! Vollstädt) and the factory reset.

use adw::prelude::*;
use exorchy_core::commands::{games, setup};

use super::storage;
use super::widgets::{self, Ctx, Row};
use crate::app;
use crate::ui::dialogs;

pub fn build(ctx: &Ctx) -> gtk::Widget {
    let page = widgets::page("About");

    let app_group = widgets::group("eXorchy", None);
    app_group.add(&Row::new("Version").value(env!("CARGO_PKG_VERSION")).hint("An eXo launcher for Omarchy (Arch + Hyprland).").widget);
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
            .hint("By Thomas Vollstädt, MIT licence. eXorchy is its Omarchy port.")
            .action(&widgets::link("https://github.com/tvollstaedt/exodium", "github.com/tvollstaedt/exodium"))
            .widget,
    );
    let support = Row::new("Support Thomas").hint("If eXorchy is useful to you, the person to thank is upstream.");
    support.add_action(&widgets::link("https://github.com/sponsors/tvollstaedt", "GitHub Sponsors"));
    support.add_action(&widgets::link("https://ko-fi.com/tvollstaedt", "Ko-fi"));
    credits.add(&support.widget);
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
    // Close the dialog first, then overlay whatever was behind it.
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
