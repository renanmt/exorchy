//! First run: where the data lives (fresh folder or an existing eXo tree)
//! and how the network is used. Both routes converge on the network step;
//! the choices are written before the torrent session starts (§11).

use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

use exorchy_core::commands::{games, setup};
use gtk::prelude::*;

use crate::app;
use crate::ui::dialogs;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Scratch,
    Import,
}

struct State {
    source: Source,
    data_dir: String,
    exodos_dir: String,
    valid: bool,
    offline: bool,
}

pub struct SetupPage {
    pub widget: gtk::Widget,
}

impl SetupPage {
    /// `on_complete` runs after the session was initialised.
    pub fn new(window: &gtk::Window, on_complete: Rc<dyn Fn()>) -> Self {
        let state = Rc::new(RefCell::new(State {
            source: Source::Scratch,
            data_dir: String::new(),
            exodos_dir: String::new(),
            valid: false,
            offline: false,
        }));

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .hhomogeneous(true)
            .vhomogeneous(false)
            .build();

        let error = gtk::Label::builder().wrap(true).max_width_chars(70).xalign(0.0).visible(false).css_classes(["danger"]).build();

        // ── mode ──
        let mode = page_box("How do you want to get started?");
        let grid = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        grid.set_homogeneous(true);
        let scratch_btn = mode_button("folder-download-symbolic", "Start from scratch", "Download games on demand from the eXoDOS torrents");
        let import_btn = mode_button("folder-open-symbolic", "Import eXoDOS installation", "Use your existing eXoDOS collection - nothing will be modified");
        grid.append(&scratch_btn);
        grid.append(&import_btn);
        mode.append(&grid);
        stack.add_named(&mode, Some("mode"));

        // ── scratch ──
        let scratch = page_box("Where should eXorchy keep its data?");
        let dir_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let dir_entry = gtk::Entry::builder().hexpand(true).css_classes(["field"]).placeholder_text("Data folder").build();
        let browse = button("Browse", &[]);
        dir_row.append(&dir_entry);
        dir_row.append(&browse);
        scratch.append(&dir_row);
        let preview = gtk::Label::builder().xalign(0.0).css_classes(["path-preview"]).wrap(true).build();
        scratch.append(&preview);
        scratch.append(&note("Games are stored under <data folder>/eXoDOS, in eXo's own layout, so the folder stays usable with other launchers."));
        let scratch_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let back1 = button("Back", &[]);
        let next1 = button("Continue", &["primary"]);
        next1.set_hexpand(true);
        scratch_actions.append(&back1);
        scratch_actions.append(&next1);
        scratch.append(&scratch_actions);
        stack.add_named(&scratch, Some("scratch"));

        // ── import ──
        let import = page_box("Select your eXoDOS folder. eXorchy only reads from it - your files are never modified.");
        let imp_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let imp_entry = gtk::Entry::builder().hexpand(true).css_classes(["field"]).placeholder_text("eXoDOS folder").editable(false).build();
        let imp_browse = button("Browse", &[]);
        imp_row.append(&imp_entry);
        imp_row.append(&imp_browse);
        import.append(&imp_row);
        let validation = gtk::Label::builder().xalign(0.0).wrap(true).build();
        import.append(&validation);
        let import_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let back2 = button("Back", &[]);
        let next2 = button("Continue", &["primary"]);
        next2.set_hexpand(true);
        next2.set_sensitive(false);
        import_actions.append(&back2);
        import_actions.append(&next2);
        import.append(&import_actions);
        stack.add_named(&import, Some("import"));

        // ── network ──
        let network = page_box("How should eXorchy use the network?");
        let net_grid = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        net_grid.set_homogeneous(true);
        let online_btn = mode_button("network-wireless-symbolic", "Online", "Download games and the emulator from the eXoDOS torrents.");
        let offline_btn = mode_button("network-offline-symbolic", "Offline", "No torrent client, nothing downloaded or shared. There is nothing to play until you go online.");
        online_btn.add_css_class("selected");
        net_grid.append(&online_btn);
        net_grid.append(&offline_btn);
        network.append(&net_grid);
        let seeding_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let seeding = gtk::Switch::builder().active(true).valign(gtk::Align::Start).build();
        let seeding_text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        seeding_text.append(&gtk::Label::builder().xalign(0.0).label("Share my downloads with other users (seeding)").build());
        seeding_text.append(&gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["setup-note"]).max_width_chars(60).label("While eXorchy runs, it uploads parts of the games you have to other users. That keeps the collection alive - but it also means you are distributing the files, which is a legal risk in some countries.").build());
        seeding_row.append(&seeding);
        seeding_row.append(&seeding_text);
        network.append(&seeding_row);
        network.append(&note("Both settings can be changed any time in Settings → Network."));
        let net_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let back3 = button("Back", &[]);
        let go = button("Continue", &["primary"]);
        go.set_hexpand(true);
        net_actions.append(&back3);
        net_actions.append(&go);
        network.append(&net_actions);
        stack.add_named(&network, Some("network"));

        // ── busy ──
        let busy = page_box("Setting up...");
        let bar = gtk::ProgressBar::new();
        busy.append(&bar);
        let busy_label = busy.first_child().and_downcast::<gtk::Label>();
        glib::timeout_add_local(std::time::Duration::from_millis(80), glib::clone!(#[weak] bar, #[weak] stack, #[upgrade_or] glib::ControlFlow::Break, move || {
            if stack.visible_child_name().as_deref() == Some("busy") {
                bar.pulse();
            }
            glib::ControlFlow::Continue
        }));
        stack.add_named(&busy, Some("busy"));

        // ── card ──
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(16)
            .css_classes(["setup-card"])
            .width_request(640)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        let logo = gtk::Image::from_paintable(Some(&crate::ui::window::app_icon()));
        logo.set_pixel_size(64);
        card.append(&logo);
        card.append(&gtk::Label::builder().label("Welcome to eXorchy").css_classes(["title-1"]).build());
        card.append(&stack);
        card.append(&error);

        let page = gtk::Box::builder().css_classes(["setup-page"]).hexpand(true).vexpand(true).build();
        page.append(&card);

        // ── wiring ──
        let show_error = glib::clone!(#[weak] error, move |msg: &str| {
            error.set_label(msg);
            error.set_visible(!msg.is_empty());
        });

        app::spawn(setup::get_default_data_dir(), glib::clone!(#[weak] dir_entry, move |r| {
            if let Ok(dir) = r {
                dir_entry.set_text(&dir);
            }
        }));
        dir_entry.connect_changed(glib::clone!(#[strong] state, #[weak] preview, #[weak] next1, move |e| {
            let text = e.text().to_string();
            preview.set_label(&if text.is_empty() { String::new() } else { format!("Games will live in {}/eXoDOS", text.trim_end_matches('/')) });
            next1.set_sensitive(!text.trim().is_empty());
            state.borrow_mut().data_dir = text;
        }));

        scratch_btn.connect_clicked(glib::clone!(#[weak] stack, #[strong] show_error, move |_| { show_error(""); stack.set_visible_child_name("scratch"); }));
        import_btn.connect_clicked(glib::clone!(#[weak] stack, #[strong] show_error, move |_| { show_error(""); stack.set_visible_child_name("import"); }));
        back1.connect_clicked(glib::clone!(#[weak] stack, move |_| stack.set_visible_child_name("mode")));
        back2.connect_clicked(glib::clone!(#[weak] stack, move |_| stack.set_visible_child_name("mode")));

        browse.connect_clicked(glib::clone!(#[weak] window, #[weak] dir_entry, move |_| {
            dialogs::pick_folder(&window, "Select eXorchy data folder", move |p| {
                if let Some(p) = p { dir_entry.set_text(&p); }
            });
        }));
        imp_browse.connect_clicked(glib::clone!(#[weak] window, #[weak] imp_entry, #[weak] validation, #[weak] next2, #[strong] state, move |_| {
            let (imp_entry, validation, next2, state) = (imp_entry.clone(), validation.clone(), next2.clone(), state.clone());
            dialogs::pick_folder(&window, "Select your eXoDOS folder", move |p| {
                let Some(p) = p else { return };
                imp_entry.set_text(&p);
                validation.set_label("Checking...");
                validation.set_css_classes(&["muted"]);
                next2.set_sensitive(false);
                state.borrow_mut().exodos_dir = p.clone();
                app::spawn(setup::validate_exodos_dir(p), glib::clone!(#[weak] validation, #[weak] next2, #[strong] state, move |r| {
                    let (ok, hint) = match r {
                        Ok(v) => (v.valid, v.hint),
                        Err(e) => (false, e),
                    };
                    validation.set_label(&if ok { "Looks like an eXoDOS folder.".to_string() } else { hint });
                    validation.set_css_classes(&[if ok { "validation-ok" } else { "validation-bad" }]);
                    next2.set_sensitive(ok);
                    state.borrow_mut().valid = ok;
                }));
            });
        }));

        let to_network = glib::clone!(#[strong] state, #[weak] stack, #[weak] online_btn, #[weak] offline_btn, #[strong] show_error, move |src: Source| {
            {
                let mut s = state.borrow_mut();
                s.source = src;
                s.offline = false;
            }
            online_btn.add_css_class("selected");
            offline_btn.remove_css_class("selected");
            show_error("");
            stack.set_visible_child_name("network");
        });
        next1.connect_clicked(glib::clone!(#[strong] to_network, move |_| to_network(Source::Scratch)));
        next2.connect_clicked(glib::clone!(#[strong] to_network, move |_| to_network(Source::Import)));

        let pick_net = glib::clone!(#[strong] state, #[weak] online_btn, #[weak] offline_btn, #[weak] seeding_row, move |offline: bool| {
            state.borrow_mut().offline = offline;
            if offline { offline_btn.add_css_class("selected"); online_btn.remove_css_class("selected"); }
            else { online_btn.add_css_class("selected"); offline_btn.remove_css_class("selected"); }
            seeding_row.set_visible(!offline);
        });
        online_btn.connect_clicked(glib::clone!(#[strong] pick_net, move |_| pick_net(false)));
        offline_btn.connect_clicked(glib::clone!(#[strong] pick_net, move |_| pick_net(true)));
        back3.connect_clicked(glib::clone!(#[weak] stack, #[strong] state, move |_| {
            stack.set_visible_child_name(match state.borrow().source { Source::Scratch => "scratch", Source::Import => "import" });
        }));

        go.connect_clicked(glib::clone!(#[strong] state, #[weak] stack, #[weak] seeding, #[strong] show_error, #[strong] on_complete, move |_| {
            let (source, data_dir, exodos_dir, offline) = {
                let s = state.borrow();
                (s.source, s.data_dir.clone(), s.exodos_dir.clone(), s.offline)
            };
            let seed = !offline && seeding.is_active();
            if let Some(l) = &busy_label {
                l.set_label(if source == Source::Import { "Importing from local directory..." } else { "Setting up..." });
            }
            show_error("");
            stack.set_visible_child_name("busy");
            let core = app::core();
            let show_error = show_error.clone();
            let on_complete = on_complete.clone();
            app::local(glib::clone!(#[weak] stack, async move {
                let result: Result<(), String> = match source {
                    Source::Scratch => {
                        let c = core.clone();
                        let mode = if offline { "offline" } else { "live" }.to_string();
                        app::call(async move { setup::setup_fresh(c.clone(), c.state(), data_dir, mode, seed).await }).await
                    }
                    Source::Import => {
                        let c = core.clone();
                        app::call(async move {
                            games::set_config(c.clone(), c.state(), "network_mode".into(), if offline { "offline" } else { "live" }.into()).await?;
                            games::set_config(c.clone(), c.state(), "seeding_enabled".into(), if seed { "1" } else { "0" }.into()).await?;
                            setup::setup_from_local(c.clone(), c.state(), c.state(), c.state(), exodos_dir).await.map(|_| ())
                        })
                        .await
                    }
                };
                let result = match result {
                    Ok(()) => {
                        let c = core.clone();
                        app::call(async move { setup::init_download_manager(c.state(), c.state(), c.state()).await.map(|_| ()) }).await
                    }
                    Err(e) => Err(e),
                };
                match result {
                    Ok(()) => on_complete(),
                    Err(e) => {
                        show_error(&format!("{}: {e}", if source == Source::Import { "Import failed" } else { "Failed to initialize" }));
                        stack.set_visible_child_name("network");
                    }
                }
            }));
        }));

        SetupPage { widget: page.upcast() }
    }
}

fn page_box(subtitle: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 14);
    b.append(&gtk::Label::builder().label(subtitle).xalign(0.0).wrap(true).max_width_chars(70).css_classes(["subtitle"]).build());
    b
}

fn note(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).max_width_chars(70).css_classes(["setup-note"]).build()
}

pub fn button(label: &str, extra: &[&str]) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("btn");
    for c in extra {
        b.add_css_class(c);
    }
    b
}

fn mode_button(icon: &str, title: &str, desc: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let img = gtk::Image::from_icon_name(icon);
    img.set_pixel_size(32);
    img.add_css_class("mode-icon");
    content.append(&img);
    content.append(&gtk::Label::builder().label(title).css_classes(["mode-title"]).wrap(true).max_width_chars(28).justify(gtk::Justification::Center).build());
    content.append(&gtk::Label::builder().label(desc).css_classes(["mode-desc"]).wrap(true).max_width_chars(34).justify(gtk::Justification::Center).build());
    gtk::Button::builder().child(&content).css_classes(["mode-btn"]).build()
}
