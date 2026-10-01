//! Where the library lives: the start-up gate and the move from Settings.
//!
//! `gate` runs between "setup is done" and the library, before the torrent
//! session or anything else can touch the library folder. It finishes a move
//! Settings requested (`library_move_target`), asks where a library that is
//! no longer found went, and offers once to move a library that still sits
//! directly in the home folder (the old default) to `~/Games/eXorchy`.
//! Refusing keeps it where it is; that layout keeps working.
//!
//! `move_from_settings` never moves anything while the app runs: it checks
//! the target, records it and restarts eXorchy, and the gate does the move.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use exorchy_core::commands::library_location::{self as loc, LibraryStatus, MovePlan};

use crate::app;
use crate::ui::setup::button;
use crate::ui::{bus, dialogs};

const PAGE: &str = "library-location";

/// Decide what the library folder needs before the library starts; `done`
/// shows the library.
pub fn gate(window: &adw::ApplicationWindow, stack: &gtk::Stack, done: Rc<dyn Fn()>) {
    let core = app::core();
    let (window, stack) = (window.clone(), stack.clone());
    app::spawn(
        async move {
            if let Err(e) = loc::finish_library_move_cleanup(core.state()).await {
                log::warn!("library move cleanup: {e}");
            }
            loc::library_status(core.state()).await
        },
        move |status| match status {
            Ok(Some(s)) if s.pending_target.is_some() => run_move(&stack, s.pending_target.clone().unwrap_or_default(), done),
            Ok(Some(s)) if !s.found => missing(&window, &stack, &s, done),
            Ok(Some(s)) if s.offer_move => offer(&window, &stack, &s, done),
            Ok(_) => done(),
            Err(e) => {
                log::error!("library_status: {e}");
                done();
            }
        },
    );
}

/// Settings → General → Game folder → Move: pick, check, confirm, restart.
pub fn move_from_settings(window: &gtk::Window) {
    let window = window.clone();
    dialogs::pick_folder(&window, "Move the library to", {
        let window = window.clone();
        move |picked| {
            let Some(target) = picked else { return };
            let core = app::core();
            app::spawn(async move { loc::plan_library_move(core.state(), target).await }, move |plan| match plan {
                Err(e) => dialogs::error(&window, "The library cannot move there", &e),
                Ok(plan) => confirm_and_restart(&window, plan),
            });
        }
    });
}

/// Settings → General → Game folder → Locate, offered only while the
/// configured folder is missing.
pub fn locate_from_settings(window: &gtk::Window, located: impl Fn() + 'static) {
    let window = window.clone();
    dialogs::pick_folder(&window, "Where is your library?", {
        let window = window.clone();
        move |picked| {
            let Some(path) = picked else { return };
            let core = app::core();
            app::spawn(async move { loc::locate_library(core.state(), path).await }, move |r| match r {
                Ok(()) => located(),
                Err(e) => dialogs::error(&window, "That is not the library", &e),
            });
        }
    });
}

fn confirm_and_restart(window: &gtk::Window, plan: MovePlan) {
    let body = if plan.same_disk {
        format!("The library moves to {} in an instant: it is on the same disk.\n\neXorchy restarts to move it.", plan.target)
    } else {
        format!(
            "{} will be copied to {}, which can take a while. The old copy is deleted once the new one is complete.\n\neXorchy restarts to move it. Downloads pause meanwhile and resume afterwards.",
            size(plan.bytes),
            plan.target
        )
    };
    let window2 = window.clone();
    dialogs::confirm(window, "Move the library?", &body, "Restart and move", false, move || {
        let core = app::core();
        let target = plan.target.clone();
        let window = window2.clone();
        app::spawn(
            async move {
                loc::request_library_move(core.state(), target).await?;
                loc::relaunch_after_exit().await
            },
            move |r| match r {
                Ok(()) => {
                    if let Some(app) = window.application() {
                        app.quit();
                    }
                }
                Err(e) => {
                    // Nothing was moved; do not leave a move for the next start.
                    let core = app::core();
                    app::spawn(async move { loc::cancel_library_move(core.state()).await }, |_| {});
                    dialogs::error(&window, "The library was not moved", &e);
                }
            },
        );
    });
}

// ── Pages ────────────────────────────────────────────────────────────────

fn show(stack: &gtk::Stack, content: &gtk::Box) {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .css_classes(["setup-card", "library-location"])
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    card.append(content);
    let clamp = adw::Clamp::builder().maximum_size(640).tightening_threshold(560).child(&card).margin_start(16).margin_end(16).margin_top(16).margin_bottom(16).build();
    let page = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).css_classes(["setup-page"]).hexpand(true).vexpand(true).child(&clamp).build();
    if let Some(old) = stack.child_by_name(PAGE) {
        stack.remove(&old);
    }
    stack.add_named(&page, Some(PAGE));
    stack.set_visible_child_name(PAGE);
}

/// Show the library, then drop this page once the crossfade away from it is
/// over: removing the child a transition is drawing leaves the window blank.
fn leave(stack: &gtk::Stack, done: &Rc<dyn Fn()>) {
    done();
    let Some(page) = stack.child_by_name(PAGE) else { return };
    let wait = std::time::Duration::from_millis(stack.transition_duration() as u64 + 50);
    let stack = stack.clone();
    glib::timeout_add_local_once(wait, move || {
        if page.parent().as_ref() == Some(stack.upcast_ref()) && stack.visible_child().as_ref() != Some(&page) {
            stack.remove(&page);
        }
    });
}

fn text(label: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder().label(label).xalign(0.0).wrap(true).max_width_chars(70).css_classes(classes.to_vec()).build()
}

fn actions(buttons: &[&gtk::Button]) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for b in buttons {
        row.append(*b);
    }
    if let Some(last) = buttons.last() {
        last.set_hexpand(true);
    }
    row
}

/// The old default: offer `~/Games/eXorchy` once.
fn offer(window: &adw::ApplicationWindow, stack: &gtk::Stack, s: &LibraryStatus, done: Rc<dyn Fn()>) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.append(&text("A tidier home for your library", &["title-2"]));
    content.append(&text(
        &format!(
            "Your games and eXorchy's files are in your home folder ({0}/{1} and {0}/content). New installs keep them together in {2}.",
            tilde(s.data_dir.trim_end_matches('/')),
            s.root_folder,
            tilde(&s.suggested)
        ),
        &["subtitle"],
    ));
    content.append(&text(
        "Only those two folders move; nothing else in your home folder is touched. If you keep them where they are, eXorchy works exactly as before, and you can move them later in Settings → General.",
        &["setup-note"],
    ));
    let error = text("", &["danger"]);
    error.set_visible(false);
    content.append(&error);
    let keep = button("Keep them here", &[]);
    let other = button("Choose a folder…", &[]);
    let go = button(&format!("Move to {}", tilde(&s.suggested)), &["primary"]);
    content.append(&actions(&[&keep, &other, &go]));
    show(stack, &content);

    let busy = Rc::new(Cell::new(false));
    // A request is checked like one from Settings, then run right here:
    // nothing else has started yet.
    let request = {
        let (stack, done, error, busy) = (stack.clone(), done.clone(), error.clone(), busy.clone());
        Rc::new(move |target: String| {
            if busy.replace(true) {
                return;
            }
            let core = app::core();
            let (stack, done, error, busy) = (stack.clone(), done.clone(), error.clone(), busy.clone());
            app::spawn(async move { loc::request_library_move(core.state(), target).await }, move |r| {
                busy.set(false);
                match r {
                    Ok(plan) => run_move(&stack, plan.target, done.clone()),
                    Err(e) => {
                        error.set_label(&e);
                        error.set_visible(true);
                    }
                }
            });
        })
    };
    let suggested = s.suggested.clone();
    go.connect_clicked({
        let request = request.clone();
        move |_| request(suggested.clone())
    });
    other.connect_clicked({
        let window = window.clone();
        move |_| {
            let request = request.clone();
            dialogs::pick_folder(&window, "Move the library to", move |p| {
                if let Some(p) = p {
                    request(p);
                }
            });
        }
    });
    keep.connect_clicked({
        let (stack, done) = (stack.clone(), done.clone());
        move |_| {
            let core = app::core();
            let (stack, done) = (stack.clone(), done.clone());
            app::spawn(async move { loc::decline_library_move(core.state()).await }, move |r| {
                if let Err(e) = r {
                    log::warn!("decline_library_move: {e}");
                }
                leave(&stack, &done);
            });
        }
    });
}

/// The configured folder is gone: locate it, or carry on (the library then
/// starts empty in that folder).
fn missing(window: &adw::ApplicationWindow, stack: &gtk::Stack, s: &LibraryStatus, done: Rc<dyn Fn()>) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.append(&text("Your library was not found", &["title-2"]));
    let expected = if s.root_folder == "." { s.data_dir.clone() } else { format!("{}/{}", s.data_dir.trim_end_matches('/'), s.root_folder) };
    content.append(&text(&format!("eXorchy expected it in {expected}."), &["subtitle"]));
    content.append(&text(
        "If you moved the folder, or it is on a drive that is not connected or is mounted somewhere else now, show eXorchy where it is. Continuing without it starts an empty library in the old place.",
        &["setup-note"],
    ));
    let error = text("", &["danger"]);
    error.set_visible(false);
    content.append(&error);
    let carry_on = button("Continue without it", &[]);
    let locate = button("Locate…", &["primary"]);
    content.append(&actions(&[&carry_on, &locate]));
    show(stack, &content);

    carry_on.connect_clicked({
        let (stack, done) = (stack.clone(), done.clone());
        move |_| leave(&stack, &done)
    });
    locate.connect_clicked({
        let (window, stack) = (window.clone(), stack.clone());
        move |_| {
            let (stack, done, error) = (stack.clone(), done.clone(), error.clone());
            dialogs::pick_folder(&window, "Where is your library?", move |p| {
                let Some(path) = p else { return };
                let core = app::core();
                app::spawn(async move { loc::locate_library(core.state(), path).await }, move |r| match r {
                    Ok(()) => leave(&stack, &done),
                    Err(e) => {
                        error.set_label(&e);
                        error.set_visible(true);
                    }
                });
            });
        }
    });
}

/// Run the recorded move with a progress bar. A failure leaves the library
/// where it was (the backend switches only after every folder is in place).
fn run_move(stack: &gtk::Stack, target: String, done: Rc<dyn Fn()>) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.append(&text("Moving your library", &["title-2"]));
    content.append(&text(&format!("To {target}"), &["subtitle", "move-path"]));
    let bar = gtk::ProgressBar::builder().css_classes(["move-progress"]).build();
    content.append(&bar);
    let status = text("Moving…", &["setup-note"]);
    content.append(&status);
    let error = text("", &["danger"]);
    error.set_visible(false);
    content.append(&error);
    let keep = button("Keep it where it is", &[]);
    let retry = button("Try again", &["primary"]);
    let failed_actions = actions(&[&keep, &retry]);
    failed_actions.set_visible(false);
    content.append(&failed_actions);
    show(stack, &content);

    // Same disk: a rename, no progress events; the bar just pulses.
    let running = Rc::new(Cell::new(true));
    let copying = Rc::new(Cell::new(false));
    glib::timeout_add_local(std::time::Duration::from_millis(120), {
        let (bar, running, copying) = (bar.downgrade(), running.clone(), copying.clone());
        move || match bar.upgrade() {
            Some(bar) if running.get() => {
                if !copying.get() {
                    bar.pulse();
                }
                glib::ControlFlow::Continue
            }
            _ => glib::ControlFlow::Break,
        }
    });
    app::on_event("library-move-progress", {
        let (bar, status, running, copying) = (bar.downgrade(), status.downgrade(), running.clone(), copying.clone());
        move |p| {
            let (Some(bar), Some(status)) = (bar.upgrade(), status.upgrade()) else { return };
            if !running.get() {
                return;
            }
            let done = p.get("done").and_then(|v| v.as_u64()).unwrap_or(0);
            let total = p.get("total").and_then(|v| v.as_u64()).unwrap_or(0).max(1);
            let item = p.get("item").and_then(|v| v.as_str()).unwrap_or("");
            copying.set(true);
            bar.set_fraction(done as f64 / total as f64);
            status.set_label(&format!("Copying {item} · {} of {}", size(done), size(total)));
        }
    });

    let start: Rc<dyn Fn()> = {
        let (stack, done, error, status, failed_actions, running) =
            (stack.clone(), done.clone(), error.clone(), status.clone(), failed_actions.clone(), running.clone());
        Rc::new(move || {
            error.set_visible(false);
            failed_actions.set_visible(false);
            status.set_label("Moving…");
            running.set(true);
            let core = app::core();
            let (stack, done, error, status, failed_actions, running) =
                (stack.clone(), done.clone(), error.clone(), status.clone(), failed_actions.clone(), running.clone());
            app::spawn(async move { loc::run_library_move(core.clone(), core.state()).await }, move |r| {
                running.set(false);
                match r {
                    Ok(dir) => {
                        crate::ui::settings::storage::reset_cache();
                        leave(&stack, &done);
                        bus::toast(&format!("Library moved to {}", tilde(&dir)));
                    }
                    Err(e) => {
                        log::error!("library move: {e}");
                        status.set_label("Nothing was lost: the library is still where it was.");
                        error.set_label(&e);
                        error.set_visible(true);
                        failed_actions.set_visible(true);
                    }
                }
            });
        })
    };
    retry.connect_clicked({
        let start = start.clone();
        move |_| start()
    });
    keep.connect_clicked({
        let (stack, done) = (stack.clone(), done.clone());
        move |_| {
            let core = app::core();
            let (stack, done) = (stack.clone(), done.clone());
            app::spawn(async move { loc::cancel_library_move(core.state()).await }, move |_| leave(&stack, &done));
        }
    });
    start();
}

fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && path.starts_with(&h) => format!("~{}", &path[h.len()..]),
        _ => path.to_string(),
    }
}

fn size(bytes: u64) -> String {
    let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    if gb >= 1.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
