//! Emulators: DOSBox Staging (source, the bundled pack, the system-package
//! preference), the DOS MIDI support payload, and - when their collections
//! are enabled - the Windows 9x and ScummVM support payloads with their
//! emulator packs (DOSBox-X, 86Box, the pinned ScummVM builds).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use exorchy_core::commands::content_packs::ContentPackStatus;
use exorchy_core::commands::{emulator_cmds, games, scummvm, win9x};
use exorchy_core::support_files::SupportStatus;

use super::widgets::{self, Ctx, Row};
use super::{dosbox_source_label, is_emulator_pack, job_status_text, packs, parse_collections, support_label, DOSBOX_JOB_KEY};
use crate::app;
use crate::ui::bus;

/// A support row: the value is the payload's phase, a bar while it downloads.
#[derive(Clone)]
struct SupportRow {
    row: Row,
    downloading: Rc<Cell<bool>>,
}

impl SupportRow {
    fn new(label: &str, hint: &str) -> Self {
        Self { row: Row::new(label).value("Checking…").hint(hint), downloading: Rc::new(Cell::new(false)) }
    }

    fn set(&self, ctx: &Ctx, status: Option<&SupportStatus>) {
        self.row.set_value(&support_label(status.map(|s| (s.phase.as_str(), s.progress, s.total_bytes))));
        let downloading = status.map(|s| s.phase == "downloading").unwrap_or(false);
        self.downloading.set(downloading);
        match status {
            Some(s) if downloading => self.row.set_below(&widgets::progress_bar(ctx, s.progress as f64, false)),
            _ => self.row.clear_below(),
        }
    }
}

pub fn build(ctx: &Ctx) -> gtk::Widget {
    let page = widgets::page("Emulators");

    // ── DOSBox Staging ──
    let staging = widgets::group(
        "DOSBox Staging",
        Some("Every DOS game runs in DOSBox Staging. eXorchy downloads its own build, or uses the one installed on the system."),
    );
    let install = widgets::BusyButton::new("Install DOSBox Staging (29 MB)", "Starting…");
    install.widget.set_visible(false);
    let emulator = Row::new("Emulator").value("Checking…").action(&install.widget);
    staging.add(&emulator.widget);
    let (sys_row, use_system) = widgets::switch_row("Prefer system dosbox-staging", "No dosbox-staging on PATH right now. AUR: dosbox-staging-bin", false);
    staging.add(&sys_row.widget);
    let tip = widgets::note("Tip: the system package is kept current by pacman. AUR: dosbox-staging-bin");
    tip.set_visible(false);
    staging.add(&tip);
    page.add(&staging);

    // ── MIDI music ──
    let midi = widgets::group("MIDI music", None);
    let mt32 = SupportRow::new(
        "MT-32 and SoundCanvas",
        "Roland MT-32 ROMs and the SoundCanvas soundfont from eXo's support files. Games that use MIDI music need them.",
    );
    midi.add(&mt32.row.widget);
    page.add(&midi);

    // ── Windows 9x / ScummVM (when enabled) ──
    let win9x_group = widgets::group("Windows 9x", Some("Windows 9x games boot a bundled system image in DOSBox-X or 86Box, both fetched as emulator packs."));
    let win9x_support = SupportRow::new("System images", "Windows 9x system images and eXo's emulator configs from the support files.");
    win9x_group.add(&win9x_support.row.widget);
    win9x_group.set_visible(false);
    page.add(&win9x_group);
    let svm_group = widgets::group("ScummVM", Some("Adventure games run through the ScummVM build eXo pins them to, fetched as an emulator pack."));
    let svm_support = SupportRow::new("Support files", "MT-32 ROMs and eXo's ScummVM extras from the support files.");
    svm_group.add(&svm_support.row.widget);
    svm_group.set_visible(false);
    page.add(&svm_group);

    // Emulator-pack rows are rebuilt when the installed set changes.
    let pack_rows: Rc<RefCell<Vec<(adw::PreferencesGroup, Row)>>> = Rc::new(RefCell::new(Vec::new()));
    let refreshers: widgets::Refreshers = Rc::new(RefCell::new(Vec::new()));
    let seen_gen = Rc::new(Cell::new(packs::installed_gen()));
    let dosbox_error: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    // The emulator row follows the pack job (progress) and the status.
    let refresh_dosbox: Rc<dyn Fn()> = {
        let (emulator, install, sys_row, use_system, tip, dosbox_error, ctx) =
            (emulator.clone(), install.clone(), sys_row.clone(), use_system.clone(), tip.clone(), dosbox_error.clone(), ctx.clone());
        Rc::new(move || {
            let core = app::core();
            let (emulator, install, sys_row, use_system, tip, dosbox_error, ctx) =
                (emulator.clone(), install.clone(), sys_row.clone(), use_system.clone(), tip.clone(), dosbox_error.clone(), ctx.clone());
            app::spawn(
                async move {
                    let status = emulator_cmds::get_dosbox_status(core.state()).await;
                    let sys = games::get_config(core.state(), "use_system_dosbox".into()).await.ok().flatten();
                    (status, sys)
                },
                move |(status, sys)| {
                    if !ctx.is_alive() {
                        return;
                    }
                    use_system.set_quiet(sys.as_deref() == Some("1"));
                    let job = packs::job(DOSBOX_JOB_KEY).filter(|j| !j.finished);
                    match status {
                        Err(e) => {
                            emulator.set_value("Checking…");
                            emulator.set_error(&format!("Could not read the emulator status: {e}"));
                            install.widget.set_visible(false);
                        }
                        Ok(s) => {
                            emulator.set_value(dosbox_source_label(&s.source));
                            let err = dosbox_error.borrow().clone();
                            let hint = if !err.is_empty() {
                                err
                            } else if let Some(j) = &job {
                                job_status_text(&j.phase, j.progress, j.error.as_deref())
                            } else if let Some(p) = &s.path {
                                p.clone()
                            } else {
                                "No dosbox-staging found. Download the bundled build or install the system package.".into()
                            };
                            if dosbox_error.borrow().is_empty() {
                                emulator.set_hint(&hint);
                            } else {
                                emulator.set_error(&hint);
                            }
                            match &job {
                                Some(j) => emulator.set_below(&widgets::progress_bar(&ctx, j.progress, j.phase != "downloading")),
                                None => emulator.clear_below(),
                            }
                            install.widget.set_visible(job.is_none() && !s.pack_installed);
                            let offline = bus::offline();
                            install.widget.set_sensitive(!offline);
                            install.widget.set_tooltip_text(if offline {
                                Some("Offline mode - nothing is downloaded. Enable downloads in Settings → Network.")
                            } else {
                                None
                            });
                            sys_row.set_hint(if s.system_available {
                                "A dosbox-staging on PATH is used instead of the downloaded build."
                            } else {
                                "No dosbox-staging on PATH right now. AUR: dosbox-staging-bin"
                            });
                            tip.set_visible(s.system_available);
                        }
                    }
                },
            );
        })
    };

    // Support payloads: re-read while one downloads.
    let refresh_support: Rc<dyn Fn()> = {
        let (mt32, win9x_support, svm_support, ctx) = (mt32.clone(), win9x_support.clone(), svm_support.clone(), ctx.clone());
        Rc::new(move || {
            let core = app::core();
            let (mt32, win9x_support, svm_support, ctx) = (mt32.clone(), win9x_support.clone(), svm_support.clone(), ctx.clone());
            app::spawn(
                async move {
                    let dos = emulator_cmds::get_dos_support_status(core.state(), core.state()).await.ok();
                    let w9x = win9x::get_win9x_support_status(core.state(), None).await.ok();
                    let svm = scummvm::get_scummvm_support_status(core.state()).await.ok();
                    (dos, w9x, svm)
                },
                move |(dos, w9x, svm)| {
                    if !ctx.is_alive() {
                        return;
                    }
                    mt32.set(&ctx, dos.as_ref());
                    win9x_support.set(&ctx, w9x.as_ref());
                    svm_support.set(&ctx, svm.as_ref());
                },
            );
        })
    };

    // Emulator packs per enabled collection.
    let load_packs: Rc<dyn Fn()> = {
        let (win9x_group, svm_group, pack_rows, refreshers, ctx) = (win9x_group.clone(), svm_group.clone(), pack_rows.clone(), refreshers.clone(), ctx.clone());
        Rc::new(move || {
            let core = app::core();
            let (win9x_group, svm_group, pack_rows, refreshers, ctx) = (win9x_group.clone(), svm_group.clone(), pack_rows.clone(), refreshers.clone(), ctx.clone());
            app::spawn(
                async move {
                    let raw = games::get_config(core.state(), "collections".into()).await.ok().flatten();
                    let enabled = parse_collections(raw.as_deref());
                    let mut out: Vec<(String, Vec<ContentPackStatus>)> = Vec::new();
                    for id in ["eXoWin9x", "eXoScummVM"] {
                        if enabled.iter().any(|c| c == id) {
                            let list = packs::list_packs(core.clone(), id.into()).await.into_iter().filter(|p| is_emulator_pack(&p.id)).collect();
                            out.push((id.to_string(), list));
                        }
                    }
                    out
                },
                move |cols| {
                    if !ctx.is_alive() {
                        return;
                    }
                    for (g, row) in pack_rows.borrow_mut().drain(..) {
                        g.remove(&row.widget);
                    }
                    refreshers.borrow_mut().clear();
                    win9x_group.set_visible(false);
                    svm_group.set_visible(false);
                    for (id, list) in cols {
                        let group = if id == "eXoWin9x" { &win9x_group } else { &svm_group };
                        group.set_visible(true);
                        let list: Vec<Rc<ContentPackStatus>> = list.into_iter().map(Rc::new).collect();
                        for pack in &list {
                            let superseded = list.iter().any(|o| o.installed && o.supersedes.contains(&pack.id));
                            let (row, refresh) = packs::pack_row(&ctx, &id, pack.clone(), superseded);
                            group.add(&row.widget);
                            pack_rows.borrow_mut().push((group.clone(), row));
                            refreshers.borrow_mut().push(refresh);
                        }
                    }
                },
            );
        })
    };

    refresh_dosbox();
    refresh_support();
    load_packs();
    packs::adopt_running("eXoDOS", "dosbox-staging");

    // Support downloads move on their own; one re-read per second while any runs.
    {
        let (refresh_support, mt32, w9, svm) = (refresh_support.clone(), mt32.clone(), win9x_support.clone(), svm_support.clone());
        ctx.poll(Duration::from_secs(1), move || {
            if mt32.downloading.get() || w9.downloading.get() || svm.downloading.get() {
                refresh_support();
            }
            true
        });
    }

    // The pack lands through the job store: the emulator row follows without a reopen.
    packs::subscribe(ctx.alive.clone(), {
        let (refresh_dosbox, load_packs, refreshers, seen_gen) = (refresh_dosbox.clone(), load_packs.clone(), refreshers.clone(), seen_gen.clone());
        move || {
            refresh_dosbox();
            if packs::installed_gen() != seen_gen.get() {
                seen_gen.set(packs::installed_gen());
                load_packs();
            } else {
                for r in refreshers.borrow().iter() {
                    r();
                }
            }
        }
    });

    use_system.on_change({
        let (use_system, refresh_dosbox) = (use_system.clone(), refresh_dosbox.clone());
        move |next| {
            let core = app::core();
            let (use_system, refresh_dosbox) = (use_system.clone(), refresh_dosbox.clone());
            let value = if next { "1" } else { "0" }.to_string();
            app::spawn(
                async move { games::set_config(core.clone(), core.state(), "use_system_dosbox".into(), value).await },
                move |res| {
                    if let Err(e) = res {
                        log::error!("settings: failed to save use_system_dosbox: {e}");
                        use_system.set_quiet(!next);
                    }
                    refresh_dosbox();
                },
            );
        }
    });

    install.widget.connect_clicked({
        let (install, dosbox_error, refresh_dosbox) = (install.clone(), dosbox_error.clone(), refresh_dosbox.clone());
        move |_| {
            dosbox_error.borrow_mut().clear();
            install.set_busy(true);
            let core = app::core();
            let (install, dosbox_error, refresh_dosbox) = (install.clone(), dosbox_error.clone(), refresh_dosbox.clone());
            // Idempotent: starts the pack job when nothing resolves; progress
            // arrives through the job store's event.
            app::spawn(async move { emulator_cmds::ensure_dosbox_staging(core.clone(), core.state()).await }, move |res| {
                install.set_busy(false);
                if let Err(e) = res {
                    *dosbox_error.borrow_mut() = format!("Could not start the download: {e}");
                }
                packs::adopt_running("eXoDOS", "dosbox-staging");
                refresh_dosbox();
            });
        }
    });

    page.upcast()
}
