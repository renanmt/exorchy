//! eXorchy: the GTK4 shell over `exorchy_core`.
//!
//! Module map:
//! - `app`      the bridge: backend handle, tokio → GTK main loop, event pump
//! - `theme`    Omarchy palette → GTK CSS custom properties, live
//! - `snapshot` EXORCHY_SNAPSHOT: render the window to a PNG (developer aid)
//! - `ui/`      windows, pages and widgets (see `ui/mod.rs`)

mod app;
mod snapshot;
mod theme;
mod ui;

use gtk::glib;
use gtk::prelude::*;

/// The GApplication id. It is also the Wayland app id Hyprland sees, and the
/// name of the desktop file (`org.exorchy.eXorchy.desktop`).
pub const APP_ID: &str = "org.exorchy.eXorchy";

fn main() -> glib::ExitCode {
    // The backend first: logger, database, managed state, theme watcher.
    let boot = exorchy_core::bootstrap();
    app::install(boot.app.clone());
    // The interface size (Settings → Appearance), before any widget exists.
    let scale = boot
        .app
        .try_state::<exorchy_core::commands::DbState>()
        .and_then(|db| db.lock().ok().and_then(|c| exorchy_core::db::queries::get_config(&c, theme::UI_SCALE_KEY).ok().flatten()))
        .and_then(|v| v.parse::<f64>().ok());
    theme::set_ui_scale(scale.unwrap_or(theme::DEFAULT_UI_SCALE));

    // GApplication is single-instance by itself: a second `exorchy` activates
    // the running one, which presents its window.
    let application = adw::Application::builder().application_id(APP_ID).build();
    let startup_error = boot.startup_error.clone();
    application.connect_startup(|_| {
        theme::init();
    });
    application.connect_activate(move |application| {
        if let Some(win) = application.active_window() {
            win.present();
            return;
        }
        let window = ui::window::build(application, startup_error.clone());
        snapshot::arm(&window);
        window.present();
    });
    let core = boot.app.clone();
    application.connect_shutdown(move |_| {
        exorchy_core::shutdown(&core);
    });
    application.run()
}
