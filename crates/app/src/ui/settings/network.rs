//! Network: online/offline mode (the torrent session), sharing with other
//! users, speed limits, and the Windows 9x packet-capture grant.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use exorchy_core::commands::win9x::{self, Win9xNetworkStatus};
use exorchy_core::commands::{emulator_cmds, games, setup};

use super::widgets::{self, Ctx, Row};
use super::{packs, parse_rate_limit};
use crate::app;
use crate::ui::{bus, downloads};

pub fn build(ctx: &Ctx) -> gtk::Widget {
    let page = widgets::page("Network");

    let dl = widgets::group("Downloads", Some("Games come from the eXoDOS BitTorrent swarm."));
    let (online_row, online) = widgets::switch_row("Online mode", "", !bus::offline());
    dl.add(&online_row.widget);
    let mode_error = widgets::note("");
    mode_error.add_css_class("error");
    mode_error.set_visible(false);
    dl.add(&mode_error);
    // Inert but visible while offline: the choice still matters for when
    // you go back online.
    let (seed_row, seeding) = widgets::switch_row("Share with other users", "", false);
    dl.add(&seed_row.widget);
    let limits = Row::new("Speed limits").hint("Whole session, both directions. Empty means unlimited.");
    let (limit_box, down, up) = limit_inputs();
    limits.set_below(&limit_box);
    dl.add(&limits.widget);
    page.add(&dl);

    // The emulated PC's network card, not the torrent client; the grant is
    // system-wide, so the row says what it costs.
    let mp = widgets::group("Windows 9x multiplayer", None);
    mp.set_visible(false);
    let net_btn = widgets::BusyButton::new("Enable…", "Waiting…");
    let capture = Row::new("Packet capture").action(&net_btn.widget);
    mp.add(&capture.widget);
    page.add(&mp);

    let seeding_on = Rc::new(Cell::new(false));

    // Rows re-read the mode: labels, hints and what is inert.
    let apply_mode: Rc<dyn Fn()> = {
        let (online_row, online, seed_row, seeding, seeding_on, down, up) = (online_row.clone(), online.clone(), seed_row.clone(), seeding.clone(), seeding_on.clone(), down.clone(), up.clone());
        Rc::new(move || {
            let offline = bus::offline();
            online.set_quiet(!offline);
            online_row.set_label(if offline { "Offline mode" } else { "Online mode" });
            online_row.set_hint(if offline {
                "The torrent client stays off - only games already on disk can be played."
            } else {
                "Games, previews and content packs are downloaded from the torrents."
            });
            seed_row.set_hint(if offline {
                "Nothing is shared while offline. Your choice is kept."
            } else {
                "Uploads parts of your games to other users while eXorchy runs. Distributing game files carries legal risk in some countries."
            });
            seeding.set_quiet(seeding_on.get() && !offline);
            seeding.set_sensitive(!offline);
            down.set_sensitive(!offline);
            up.set_sensitive(!offline && seeding_on.get());
        })
    };

    let core = app::core();
    app::spawn(
        async move {
            let get = |k: &str| {
                let c = core.clone();
                let k = k.to_string();
                async move { games::get_config(c.state(), k).await.ok().flatten() }
            };
            let mode = get("network_mode").await;
            (mode, get("seeding_enabled").await, get("rate_limit_down_kbps").await, get("rate_limit_up_kbps").await)
        },
        {
            let (apply_mode, seeding_on, down, up) = (apply_mode.clone(), seeding_on.clone(), down.clone(), up.clone());
            move |(mode, seed, d, u)| {
                bus::set_offline(mode.as_deref() == Some("offline"));
                // Opt-in: only an explicit "1" counts, mirroring the backend.
                seeding_on.set(seed.as_deref() == Some("1"));
                down.set_text(d.as_deref().unwrap_or(""));
                up.set_text(u.as_deref().unwrap_or(""));
                apply_mode();
            }
        },
    );
    apply_mode();

    // Rebuilds the torrent state: offline drops every manager, online
    // creates a fresh session and re-adopts interrupted downloads.
    online.on_change({
        let (online, mode_error, apply_mode, ctx) = (online.clone(), mode_error.clone(), apply_mode.clone(), ctx.clone());
        move |go_online| {
            mode_error.set_visible(false);
            online.set_sensitive(false);
            let previous_offline = bus::offline();
            // Trackers and HTTP pack jobs stop BEFORE the managers go.
            let (stopped_downloads, stopped_packs) = if go_online { (0, 0) } else { (downloads::stop_all(), packs::cancel_all()) };
            bus::set_offline(!go_online);
            let core = app::core();
            let mode = if go_online { "live" } else { "offline" }.to_string();
            let (online, mode_error, apply_mode, ctx) = (online.clone(), mode_error.clone(), apply_mode.clone(), ctx.clone());
            app::spawn(
                async move {
                    // The config write MUST land before init_download_manager.
                    games::set_config(core.clone(), core.state(), "network_mode".into(), mode).await?;
                    setup::init_download_manager(core.state(), core.state(), core.state()).await?;
                    Ok::<(), String>(())
                },
                move |res| {
                    online.set_sensitive(true);
                    match res {
                        Ok(()) => {
                            let mut notes = Vec::new();
                            if stopped_downloads > 0 {
                                notes.push(format!(
                                    "{stopped_downloads} game download{} paused - resumes when you go back online",
                                    if stopped_downloads == 1 { "" } else { "s" }
                                ));
                            }
                            if stopped_packs > 0 {
                                notes.push(format!("{stopped_packs} content pack download{} cancelled", if stopped_packs == 1 { "" } else { "s" }));
                            }
                            let detail = if notes.is_empty() { None } else { Some(format!("{}.", notes.join("; "))) };
                            bus::toast_with(
                                if go_online { "Online mode - downloads enabled" } else { "Offline mode - torrent client stopped" },
                                detail.as_deref(),
                                None,
                            );
                            if go_online {
                                downloads::resume_all();
                                // Going online is where the emulator is fetched
                                // and where an old install owes its seeding answer.
                                let core = app::core();
                                app::spawn(async move { emulator_cmds::ensure_dosbox_staging(core.clone(), core.state()).await }, |_| {});
                                crate::ui::onboarding::run(&ctx.window);
                            }
                        }
                        Err(e) => {
                            bus::set_offline(previous_offline);
                            mode_error.set_label(&format!("Could not switch mode: {e}"));
                            mode_error.set_visible(true);
                            let core = app::core();
                            let back = if previous_offline { "offline" } else { "live" }.to_string();
                            app::spawn(async move { games::set_config(core.clone(), core.state(), "network_mode".into(), back).await }, |r| {
                                if let Err(e) = r {
                                    log::error!("settings: could not roll back network_mode: {e}");
                                }
                            });
                        }
                    }
                    apply_mode();
                },
            );
        }
    });

    // Persist and apply; rolls back if the backend refuses.
    seeding.on_change({
        let (seeding_on, apply_mode) = (seeding_on.clone(), apply_mode.clone());
        move |next| {
            let previous = seeding_on.get();
            seeding_on.set(next);
            apply_mode();
            let core = app::core();
            let (seeding_on, apply_mode) = (seeding_on.clone(), apply_mode.clone());
            app::spawn(async move { games::set_seeding_enabled(core.state(), core.state(), next).await }, move |res| {
                if let Err(e) = res {
                    log::error!("settings: failed to save seeding preference: {e}");
                    seeding_on.set(previous);
                    apply_mode();
                }
            });
        }
    });

    // Saves on leaving the field: applying mid-typing would throttle to
    // "5" on the way to "500".
    let limit_error = widgets::note("");
    limit_error.add_css_class("error");
    limit_error.set_visible(false);
    limit_box.append(&limit_error);
    let save_limits: Rc<dyn Fn()> = {
        let (down, up, limit_error) = (down.clone(), up.clone(), limit_error.clone());
        Rc::new(move || {
            limit_error.set_visible(false);
            let d = parse_rate_limit(&down.text());
            let u = parse_rate_limit(&up.text());
            down.set_text(&d.map(|v| v.to_string()).unwrap_or_default());
            up.set_text(&u.map(|v| v.to_string()).unwrap_or_default());
            let core = app::core();
            let limit_error = limit_error.clone();
            app::spawn(async move { games::set_rate_limits(core.state(), core.state(), u, d).await }, move |res| {
                if let Err(e) = res {
                    limit_error.set_label(&format!("Could not apply the limits: {e}"));
                    limit_error.set_visible(true);
                }
            });
        })
    };
    for entry in [&down, &up] {
        let s = save_limits.clone();
        entry.connect_activate(move |_| s());
        let focus = gtk::EventControllerFocus::new();
        let s = save_limits.clone();
        focus.connect_leave(move |_| s());
        entry.add_controller(focus);
    }

    // Windows 9x packet capture.
    let show_status: Rc<dyn Fn(&Win9xNetworkStatus)> = {
        let (mp, capture, net_btn) = (mp.clone(), capture.clone(), net_btn.clone());
        Rc::new(move |st: &Win9xNetworkStatus| {
            mp.set_visible(true);
            capture.set_hint(&st.detail);
            net_btn.widget.set_visible(st.can_enable || st.enabled);
            net_btn.set_label(if st.enabled { "Remove…" } else { "Enable…" });
            match &st.manual_hint {
                Some(h) => capture.set_below(&widgets::code(h)),
                None => capture.clear_below(),
            }
        })
    };
    let enabled_now = Rc::new(Cell::new(false));
    {
        let core = app::core();
        let (show_status, enabled_now, ctx) = (show_status.clone(), enabled_now.clone(), ctx.clone());
        let _ = &core;
        app::spawn(async move { win9x::win9x_network_status().await }, move |res| {
            if let (Ok(st), true) = (res, ctx.is_alive()) {
                enabled_now.set(st.enabled);
                show_status(&st);
            }
        });
    }
    net_btn.widget.connect_clicked({
        let (net_btn, show_status, enabled_now) = (net_btn.clone(), show_status.clone(), enabled_now.clone());
        move |_| {
            let enable = !enabled_now.get();
            net_btn.set_busy(true);
            let core = app::core();
            let (net_btn, show_status, enabled_now) = (net_btn.clone(), show_status.clone(), enabled_now.clone());
            app::spawn(
                async move {
                    if enable {
                        win9x::enable_win9x_network(core.clone()).await
                    } else {
                        win9x::disable_win9x_network(core.clone()).await
                    }
                },
                move |res| {
                    net_btn.set_busy(false);
                    match res {
                        Ok(st) => {
                            enabled_now.set(st.enabled);
                            show_status(&st);
                            bus::toast(if enable { "Windows 9x multiplayer enabled" } else { "Windows 9x multiplayer disabled" });
                        }
                        // "cancelled" is the user dismissing the OS dialog - not a failure.
                        Err(e) if e.contains("cancelled") => {}
                        Err(e) => bus::toast_with(if enable { "Could not enable multiplayer" } else { "Could not disable multiplayer" }, Some(&e), None),
                    }
                },
            );
        }
    });

    page.upcast()
}

/// "Down [   ] KB/s   Up [   ] KB/s".
fn limit_inputs() -> (gtk::Box, gtk::Entry, gtk::Entry) {
    let row = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
    let line = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(18).build();
    let field = |label: &str| {
        let b = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(6).css_classes(["limit-field"]).build();
        b.append(&widgets::note(label));
        let e = gtk::Entry::builder().placeholder_text("∞").input_purpose(gtk::InputPurpose::Digits).width_chars(7).max_width_chars(7).css_classes(["field"]).build();
        b.append(&e);
        b.append(&widgets::note("KB/s"));
        line.append(&b);
        e
    };
    let down = field("Down");
    let up = field("Up");
    row.append(&line);
    (row, down, up)
}
