//! First-session dialogs (the web UI's `SeedingConsentDialog` +
//! `stores/seeding.ts`, and `WelcomeModal`): the seeding question for
//! installs that predate it, then the welcome offer of a content pack.
//! Both only while online - offline there is no swarm to seed and no pack
//! to fetch - and each is asked once (`seeding_enabled`, `welcome_seen`).

use std::rc::Rc;

use exorchy_core::commands::{content_packs, games};
use gtk::glib;
use adw::prelude::*;

use crate::app;
use crate::ui::bus;
use crate::ui::util::format_bytes;

/// After the library is up: seeding consent (only when the key is unset),
/// then the welcome modal (only when `welcome_seen` is unset).
pub fn run(window: &impl IsA<gtk::Window>) {
    if bus::offline() {
        return;
    }
    let win: gtk::Window = window.clone().upcast();
    let core = app::core();
    app::spawn(async move { games::get_config(core.state(), "seeding_enabled".into()).await }, move |res| {
        let then: Rc<dyn Fn()> = {
            let win = win.clone();
            Rc::new(move || welcome_if_needed(&win))
        };
        match res {
            // Unset means the install owes the answer; the backend reads unset
            // as off meanwhile.
            Ok(None) => seeding_consent(&win, None, then),
            // Asking on a failed read would mean asking on every start; the
            // safe state (not seeding) already holds, so stay quiet.
            _ => then(),
        }
    });
}

/// Two buttons, not dismissable. A refused save re-asks with the error.
fn seeding_consent(win: &gtk::Window, error: Option<String>, then: Rc<dyn Fn()>) {
    let mut body = String::from(
        "Until now, eXorchy shared parts of the games you have with other players while it was running, without asking. From this version on it is up to you.\n\n\
         Sharing keeps the collection alive for everyone - but it also means you are distributing the game files, which is a legal risk in some countries. \
         eXorchy is not uploading anything until you answer, and you can change your mind any time in Settings → Network.",
    );
    if let Some(e) = error {
        body.push_str(&format!("\n\nCould not save that: {e}"));
    }
    let dialog = adw::AlertDialog::builder().heading("Sharing is now your choice").body(&body).can_close(false).build();
    dialog.add_responses(&[("off", "Don't share"), ("on", "Keep sharing")]);
    dialog.set_response_appearance("on", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("on"));
    let parent = win.clone();
    dialog.connect_response(None, move |_, r| {
        let enabled = r == "on";
        let core = app::core();
        let (win, then) = (parent.clone(), then.clone());
        app::spawn(async move { games::set_seeding_enabled(core.state(), core.state(), enabled).await }, move |res| match res {
            Ok(()) => {
                bus::toast(if enabled {
                    "Sharing is on - thank you for keeping the collection alive."
                } else {
                    "Sharing is off. You can turn it on in Settings → Network."
                });
                then();
            }
            Err(e) => seeding_consent(&win, Some(e), then),
        });
    });
    dialog.present(Some(win));
}

fn welcome_if_needed(win: &gtk::Window) {
    let core = app::core();
    let win = win.clone();
    app::spawn(async move { games::get_config(core.state(), "welcome_seen".into()).await }, move |res| {
        if let Ok(None) = res {
            let core = app::core();
            let win = win.clone();
            app::spawn(async move { content_packs::list_content_packs(core.state(), "eXoDOS".into()).await }, move |packs| welcome(&win, packs));
        }
    });
}

/// The offer: box art packs for eXoDOS, or "skip". Only an explicit choice
/// marks the modal seen - if the packs could not load (network), the user
/// is asked again next time.
fn welcome(win: &gtk::Window, packs: Result<Vec<content_packs::ContentPackStatus>, String>) {
    let dialog = adw::AlertDialog::builder().heading("Enhance your library").build();
    let extra = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
    let credit = gtk::Label::builder()
        .use_markup(true)
        .label("eXorchy is an unofficial launcher. Every game, config, and piece of artwork is the work of the <a href=\"https://www.retro-exo.com/exodos.html\">eXoDOS preservation project</a>.")
        .xalign(0.0)
        .wrap(true)
        .margin_top(8)
        .css_classes(["muted", "small", "welcome-credit"])
        .build();

    match packs {
        Err(_) => {
            dialog.set_body("Content packs unavailable right now. Check Settings later when you're online.");
            extra.append(&credit);
            dialog.set_extra_child(Some(&extra));
            dialog.add_responses(&[("close", "Close")]);
            dialog.set_default_response(Some("close"));
            dialog.present(Some(win));
        }
        Ok(packs) => {
            dialog.set_body("Download optional content to see box art for your games. You can manage these anytime in Settings.");
            let skip = gtk::CheckButton::builder().label("Skip for now").active(true).css_classes(["welcome-option"]).build();
            extra.append(&skip);
            let mut options: Vec<(gtk::CheckButton, String, String, u64)> = Vec::new();
            for p in &packs {
                let b = gtk::CheckButton::builder().css_classes(["welcome-option"]).sensitive(p.available).build();
                b.set_group(Some(&skip));
                let label = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                label.append(&gtk::Label::builder().label(&p.display_name).xalign(0.0).build());
                label.append(&gtk::Label::builder().label(format!("- ~{}", format_bytes(p.size_bytes))).css_classes(["muted"]).build());
                if !p.available {
                    label.append(&gtk::Label::builder().label("Coming soon").css_classes(["badge"]).build());
                }
                b.set_child(Some(&label));
                extra.append(&b);
                options.push((b, p.id.clone(), p.display_name.clone(), p.size_bytes));
            }
            extra.append(&credit);
            dialog.set_extra_child(Some(&extra));
            dialog.add_responses(&[("continue", "Continue")]);
            dialog.set_response_appearance("continue", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("continue"));
            dialog.set_close_response("continue");
            let options = Rc::new(options);
            // The button says what it will do.
            for (b, ..) in options.iter() {
                b.connect_toggled(glib::clone!(#[weak] dialog, #[strong] options, move |_| {
                    let any = options.iter().any(|(b, ..)| b.is_active());
                    dialog.set_response_label("continue", if any { "Download & Continue" } else { "Continue" });
                }));
            }
            dialog.connect_response(None, move |_, _| {
                let chosen = options.iter().find(|(b, ..)| b.is_active()).map(|(_, id, name, size)| (id.clone(), name.clone(), *size));
                let core = app::core();
                app::local(async move {
                    // Record the choice first - even if the download fails to
                    // start, the user decided and must not be asked again.
                    let c = core.clone();
                    if let Err(e) = app::call(async move { games::set_config(c.clone(), c.state(), "welcome_seen".into(), "1".into()).await }).await {
                        log::warn!("welcome_seen: {e}");
                    }
                    let Some((id, name, size)) = chosen else { return };
                    let c = core.clone();
                    match app::call(async move { content_packs::install_content_pack(c.clone(), "eXoDOS".into(), id).await }).await {
                        Ok(()) => bus::toast(&format!("Downloading {name} ({}) - progress in Settings → Content packs.", format_bytes(size))),
                        Err(e) => bus::toast_with(&format!("Couldn't start the {name} download"), Some(&e), None),
                    }
                });
            });
            dialog.present(Some(win));
        }
    }
}
