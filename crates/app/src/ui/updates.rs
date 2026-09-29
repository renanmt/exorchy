//! Updates of eXorchy itself: a check against the GitHub releases shortly
//! after start and every six hours (unless switched off in Settings →
//! General, or offline), a banner at the top of the library when a newer
//! release exists (Update · What's new · Skip this version), and the update
//! itself: `app_update::launch_app_update` opens a terminal that installs
//! that release and starts eXorchy again, and this process quits so the new
//! one can start. Only a pacman-installed copy offers Update; any other gets
//! the notice with a hint. Config keys: `update_check` ("0" = off),
//! `update_skipped` (a tag the user dismissed).

use std::cell::RefCell;
use std::time::Duration;

use exorchy_core::commands::app_update::{self, AppUpdate};
use exorchy_core::commands::games;
use gtk::glib;
use gtk::prelude::*;

use crate::app;
use crate::ui::{bus, dialogs, downloads};

const FIRST_CHECK: Duration = Duration::from_secs(8);
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

struct State {
    window: gtk::Window,
    banner: gtk::Box,
    title: gtk::Label,
    update: gtk::Button,
    hint: gtk::Label,
    available: Option<AppUpdate>,
    /// A check has answered this session.
    checked: bool,
    supported: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Mount the banner in `slot` (top of the library) and start checking.
pub fn install(window: &gtk::Window, slot: &gtk::Box) {
    let title = gtk::Label::builder().xalign(0.0).css_classes(["update-title"]).build();
    let hint = gtk::Label::builder()
        .label("Installed from source: update with git.")
        .css_classes(["muted", "small"])
        .visible(false)
        .build();
    let notes = gtk::Button::builder().label("What's new").css_classes(["btn", "ghost"]).build();
    let skip = gtk::Button::builder().label("Skip this version").css_classes(["btn", "ghost"]).build();
    let update = gtk::Button::builder().label("Update").css_classes(["btn", "primary"]).visible(false).build();
    // Groups that wrap as a whole in a narrow tile: the message, then the
    // hint and the buttons.
    let message = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).valign(gtk::Align::Center).build();
    message.append(&gtk::Image::from_icon_name("software-update-available-symbolic"));
    message.append(&title);
    let buttons = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(6).build();
    buttons.append(&notes);
    buttons.append(&skip);
    buttons.append(&update);
    let content = adw::WrapBox::builder().child_spacing(12).line_spacing(4).hexpand(true).build();
    content.append(&message);
    content.append(&hint);
    content.append(&buttons);
    let banner = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).css_classes(["update-banner"]).visible(false).build();
    banner.append(&content);
    slot.append(&banner);

    notes.connect_clicked(|_| {
        if let Some(u) = available() {
            let window = STATE.with(|s| s.borrow().as_ref().map(|s| s.window.clone()));
            gtk::UriLauncher::new(&u.url).launch(window.as_ref(), gtk::gio::Cancellable::NONE, |_| {});
        }
    });
    skip.connect_clicked(|_| {
        let Some(u) = available() else { return };
        let core = app::core();
        app::spawn(async move { games::set_config(core.clone(), core.state(), "update_skipped".into(), u.tag).await }, |res| {
            if let Err(e) = res {
                log::error!("update: could not remember the skipped version: {e}");
            }
        });
        with_state(|s| s.banner.set_visible(false));
    });
    update.connect_clicked(|_| start_update());

    STATE.with(|s| {
        *s.borrow_mut() = Some(State { window: window.clone(), banner, title, update, hint, available: None, checked: false, supported: false })
    });
    app::spawn(async { app_update::update_supported().await }, |ok| with_state(|s| s.supported = ok));

    glib::timeout_add_local_once(FIRST_CHECK, || {
        background_check();
        glib::timeout_add_local(EVERY, || {
            background_check();
            glib::ControlFlow::Continue
        });
    });
}

fn with_state(f: impl FnOnce(&mut State)) {
    STATE.with(|s| {
        if let Some(state) = s.borrow_mut().as_mut() {
            f(state);
        }
    });
}

/// The newer release found by the last check, if any.
pub fn available() -> Option<AppUpdate> {
    STATE.with(|s| s.borrow().as_ref().and_then(|s| s.available.clone()))
}

/// Whether Update is offered (the pacman-installed copy).
pub fn supported() -> bool {
    STATE.with(|s| s.borrow().as_ref().map(|s| s.supported).unwrap_or(false))
}

/// The scheduled check: respects the switch, offline mode and a skipped tag.
fn background_check() {
    if bus::offline() {
        return;
    }
    let core = app::core();
    app::spawn(
        async move {
            let enabled = games::get_config(core.state(), "update_check".into()).await.ok().flatten().as_deref() != Some("0");
            if !enabled {
                return Ok(None);
            }
            let skipped = games::get_config(core.state(), "update_skipped".into()).await.ok().flatten();
            let found = app_update::check_app_update().await?;
            Ok(found.filter(|u| skipped.as_deref() != Some(u.tag.as_str())))
        },
        |res: Result<Option<AppUpdate>, String>| match res {
            Ok(found) => show(found),
            Err(e) => log::info!("update check: {e}"),
        },
    );
}

/// "Check now" (Settings → About): ignores the switch and a skipped tag.
pub fn check_now(done: impl FnOnce(Result<Option<AppUpdate>, String>) + 'static) {
    app::spawn(async { app_update::check_app_update().await }, move |res| {
        if let Ok(found) = &res {
            show(found.clone());
        }
        done(res);
    });
}

fn show(found: Option<AppUpdate>) {
    with_state(|s| {
        s.checked = true;
        s.available = found.clone();
        match &found {
            Some(u) => {
                s.title.set_label(&format!("eXorchy {} is available", u.version));
                s.update.set_visible(s.supported);
                s.hint.set_visible(!s.supported);
                s.banner.set_visible(true);
            }
            None => s.banner.set_visible(false),
        }
    });
}

/// Update: warn about what quitting interrupts, then open the update
/// terminal and quit.
pub fn start_update() {
    let Some(u) = available() else { return };
    let Some(window) = STATE.with(|s| s.borrow().as_ref().map(|s| s.window.clone())) else { return };
    let downloading = downloads::active_ids().len();
    let running = bus::running().len();
    let mut body = format!(
        "eXorchy closes, a terminal installs version {} (it asks for your password) and eXorchy starts again.",
        u.version
    );
    if running > 0 {
        body.push_str("\n\nA game is running: it keeps running, but save your progress first to be safe.");
    }
    if downloading > 0 {
        body.push_str(&format!(
            "\n\n{} paused and resume when eXorchy is back.",
            if downloading == 1 { "1 download is".to_string() } else { format!("{downloading} downloads are") }
        ));
    }
    let tag = u.tag.clone();
    let win = window.clone();
    dialogs::confirm(&window, &format!("Update to eXorchy {}?", u.version), &body, "Update", false, move || {
        app::spawn(async move { app_update::launch_app_update(tag).await }, move |res| match res {
            Ok(()) => {
                if let Some(app) = win.application() {
                    app.quit();
                }
            }
            Err(e) => bus::toast_with("Couldn't start the update", Some(&e), None),
        });
    });
}

/// For the About page: `(label, update available and supported)`.
pub fn status_text() -> (String, bool) {
    let checked = STATE.with(|s| s.borrow().as_ref().map(|s| s.checked).unwrap_or(false));
    let current = app_update::current_version();
    match available() {
        Some(u) => (format!("{} available (you have {current})", u.version), supported()),
        None if checked => (format!("{current} · up to date"), false),
        None => (format!("{current} · not checked yet"), false),
    }
}
