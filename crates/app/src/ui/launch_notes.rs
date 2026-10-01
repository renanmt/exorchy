//! The one note the detail panel shows above the action bar (the web UI's
//! `launchNotes.ts` + `win9xStatus.ts`): what will run the game and what
//! stands in its way, most actionable first. `launch_note` is pure; `attach`
//! gathers the context for the panel's selected row (engine probes, the
//! Win9x / ScummVM support payload, the emulator pack that could supply a
//! missing engine) and renders the result, blocking Play when a launch
//! cannot work.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use exorchy_core::commands::scummvm::{EngineSource, ScummVmEngineInfo};
use exorchy_core::commands::win9x::{Win9xMultiplayerInfo, Win9xSupportStatus};
use exorchy_core::commands::{content_packs, games, scummvm, win9x};
use exorchy_core::models::Game;
use gtk::glib;
use gtk::prelude::*;

use crate::app;
use crate::ui::detail::DetailPanel;
use crate::ui::util::format_bytes;
use crate::ui::{bus, downloads};

// ── The note and its context ──────────────────────────────────────────────

/// A remedy the app can perform itself: install the emulator pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteAction {
    pub label: String,
    pub pack_id: String,
}

/// `blocking` notes describe a launch that cannot work and are never
/// dismissable; `pending` ones are blocks eXorchy is working on (a pack or
/// support download in flight), so Play shows a spinner instead of failing.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelNote {
    pub key: &'static str,
    pub text: String,
    pub blocking: bool,
    pub pending: bool,
    pub action: Option<NoteAction>,
}

impl PanelNote {
    fn info(key: &'static str, text: String) -> Self {
        PanelNote { key, text, blocking: false, pending: false, action: None }
    }
    fn block(key: &'static str, text: String) -> Self {
        PanelNote { key, text, blocking: true, pending: false, action: None }
    }
}

/// `games::GameEngineInfo`, owned (the backend type is not `Clone`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineInfo {
    pub ece_available: bool,
    pub uses_ece: bool,
    /// DOS / Windows 3.x only: "staging" | "dosbox-x", what the next launch uses.
    pub engine: Option<String>,
    /// The conf uses eXo's virtual printer.
    pub prints: bool,
    /// `engine` resolves on this system.
    pub engine_available: bool,
}

/// The content pack that could supply the missing emulator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmulatorPack {
    pub id: String,
    /// The collection that owns the pack in the manifest (DOSBox-X: eXoWin9x,
    /// for DOS games too); installs and progress key on it.
    pub collection: String,
    pub display_name: String,
    pub size_bytes: u64,
}

/// That pack's running install.
#[derive(Debug, Clone, PartialEq)]
pub struct PackJob {
    pub phase: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub finished: bool,
}

/// Everything the note logic reads, snapshotted from the panel's state.
#[derive(Default)]
pub struct NoteContext {
    pub game: Option<Game>,
    pub is_windows: bool,
    pub offline: bool,
    pub installed: bool,
    pub downloading: bool,
    /// None while the backend is still answering.
    pub svm_engine: Option<ScummVmEngineInfo>,
    /// eXo's note.txt for an installed ScummVM game.
    pub svm_note: Option<String>,
    pub engine_info: Option<EngineInfo>,
    pub win9x_engine_missing: bool,
    pub support: Option<Win9xSupportStatus>,
    pub mp: Option<Win9xMultiplayerInfo>,
    pub printing_unavailable: bool,
    pub video_unsupported: bool,
    pub emulator_pack: Option<EmulatorPack>,
    pub pack_job: Option<PackJob>,
}

/// Win9x variant slugs only exist in the eXoWin9x catalogue, so they double
/// as the collection test.
pub fn is_win9x(g: Option<&Game>) -> bool {
    matches!(g.and_then(|g| g.dosbox_variant.as_deref()), Some(v) if v == "x98" || v == "pcbox" || v.starts_with("86box"))
}

/// eXoScummVM rows carry build names as variant slugs, so the collection id
/// is the marker.
pub fn is_scummvm(g: Option<&Game>) -> bool {
    g.and_then(|g| g.torrent_source.as_deref()) == Some("eXoScummVM")
}

/// What will actually run the game; the backend's own resolver decides ECE
/// vs Staging (`runs_under_ece`), the variant slug the rest.
pub fn emulator_name(g: Option<&Game>, svm_engine: Option<&ScummVmEngineInfo>, runs_under_ece: bool, dos_engine: Option<&str>) -> String {
    if is_scummvm(g) {
        return match svm_engine.map(|e| e.pinned_version.as_str()).filter(|v| !v.is_empty()) {
            Some(v) => format!("ScummVM {v}"),
            None => "ScummVM".into(),
        };
    }
    match dos_engine {
        Some("dosbox-x") => return "DOSBox-X".into(),
        Some("staging") => return "DOSBox Staging".into(),
        _ => {}
    }
    match g.and_then(|g| g.dosbox_variant.as_deref()) {
        Some("x98") => "DOSBox-X".into(),
        Some("pcbox") => "PCBox (not shipped)".into(),
        Some(v) if v.starts_with("86box") => "86Box".into(),
        Some(v) if v.starts_with("ece") => if runs_under_ece { "DOSBox ECE" } else { "DOSBox Staging" }.into(),
        _ => "DOSBox Staging".into(),
    }
}

/// The part of a ScummVM variant folder worth a chip: "Maniac Mansion
/// (DOS v1)" -> "DOS v1"; a name without parentheses stays whole. For the
/// panel's version switcher (not mounted yet).
#[allow(dead_code)]
pub fn svm_variant_label(name: &str) -> String {
    let t = name.trim_end();
    if let Some(stripped) = t.strip_suffix(')') {
        if let Some(open) = stripped.rfind('(') {
            let inner = &stripped[open + 1..];
            if !inner.contains('(') && !inner.contains(')') {
                return inner.to_string();
            }
        }
    }
    name.to_string()
}

/// Backend's `emulator_pack_for_variant`, mirrored.
pub fn emulator_pack_id(variant: Option<&str>) -> Option<&'static str> {
    match variant {
        Some(v) if v.starts_with("86box") => Some("86box"),
        Some("pcbox") => None,
        _ => Some("dosbox-x"),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Win9x,
    ScummVm,
}

impl Family {
    fn label(self) -> &'static str {
        match self {
            Family::Win9x => "Windows 9x",
            Family::ScummVm => "ScummVM",
        }
    }
}

fn support_progress_note(s: &Win9xSupportStatus, family: Family, what: &str) -> PanelNote {
    let pct = (s.progress * 100.0).round() as i64;
    let text = if pct >= 100 {
        format!("Setting up the {} support files ({what})…", family.label())
    } else {
        format!("Downloading the {} support files ({what})… {pct}%", family.label())
    };
    PanelNote {
        key: match family {
            Family::ScummVm => "scummvm-support-progress",
            Family::Win9x => "win9x-support-progress",
        },
        text,
        blocking: true,
        pending: true,
        action: None,
    }
}

fn support_failed_note(family: Family) -> PanelNote {
    PanelNote::block(
        match family {
            Family::ScummVm => "scummvm-support-failed",
            Family::Win9x => "win9x-support-failed",
        },
        format!(
            "Setting up the {} support files failed - make sure the library drive has enough free space, then restart eXorchy to retry.",
            family.label()
        ),
    )
}

fn one_time(bytes: u64) -> String {
    if bytes > 0 {
        format!(" (one-time {})", format_bytes(bytes))
    } else {
        String::new()
    }
}

/// The pack that supplies the missing emulator, as a note: its running
/// install, the offline case, or the download button. None without a pack.
fn pack_remedy(ctx: &NoteContext, key: &'static str, engine: &str, blocking: bool) -> Option<PanelNote> {
    let pack = ctx.emulator_pack.as_ref()?;
    if let Some(job) = ctx.pack_job.as_ref().filter(|j| !j.finished) {
        let pct = if job.total_bytes > 0 { (job.downloaded_bytes as f64 / job.total_bytes as f64 * 100.0).round() as i64 } else { 0 };
        let text = if job.phase == "extracting" {
            format!("Installing {}…", pack.display_name)
        } else {
            format!("Downloading {}… {pct}%", pack.display_name)
        };
        return Some(PanelNote { key, text, blocking, pending: true, action: None });
    }
    if ctx.offline {
        return Some(PanelNote {
            key,
            text: format!("This game needs {engine}, which is not downloaded yet. Go online (Settings → Network) to download it."),
            blocking,
            pending: false,
            action: None,
        });
    }
    Some(PanelNote {
        key,
        text: format!("This game needs {engine}, which is not downloaded yet."),
        blocking,
        pending: false,
        action: Some(NoteAction { label: format!("Download emulator ({})", format_bytes(pack.size_bytes)), pack_id: pack.id.clone() }),
    })
}

fn scummvm_note(ctx: &NoteContext, engine: &str) -> Option<PanelNote> {
    let e = ctx.svm_engine.as_ref()?;
    if !e.available {
        // The game's own Download button already fetches the engine with it
        // (download_game queues the pack), so an uninstalled game gets the
        // price rather than a second, competing action next to it.
        if !ctx.installed && !ctx.downloading {
            if let Some(pack) = &ctx.emulator_pack {
                return Some(PanelNote::info(
                    "scummvm-engine-size",
                    format!("Downloading this game also fetches {engine}{} - every game eXo pins to that build uses it.", one_time(pack.size_bytes)),
                ));
            }
        }
        // Windows has no pack: eXo's own builds come out of utilSVM.zip, which
        // download_game queues with the game. The note follows that payload.
        if ctx.is_windows && ctx.offline {
            return Some(PanelNote::block(
                "engine-missing",
                format!("This game needs eXo's {engine}, which downloads with the game. Go online (Settings → Network) to fetch it."),
            ));
        }
        if ctx.is_windows {
            if let Some(support) = &ctx.support {
                return Some(match support.phase.as_str() {
                    "failed" => support_failed_note(Family::ScummVm),
                    "downloading" => support_progress_note(support, Family::ScummVm, engine),
                    "missing" if !ctx.installed => PanelNote::info(
                        "scummvm-engine-size",
                        format!("Downloading this game also fetches eXo's {engine}{} - every game eXo pins to that build uses it.", one_time(support.total_bytes)),
                    ),
                    _ => PanelNote::block(
                        "engine-missing",
                        format!("{engine} was not found in the ScummVM support files (eXo\\emulators\\scmvm inside your library folder). Restore that folder, or delete it and download any ScummVM game to fetch it again."),
                    ),
                });
            }
        }
        return Some(pack_remedy(ctx, "engine-missing", engine, true).unwrap_or_else(|| {
            PanelNote::block(
                "engine-missing",
                format!(
                    "This game runs under {engine}, which was not found on this system. Install ScummVM from scummvm.org{} and try again.",
                    if ctx.is_windows { "" } else { " (Linux: the Flatpak org.scummvm.ScummVM works too)" }
                ),
            )
        }));
    }
    if let Some(note) = ctx.svm_note.as_deref().filter(|n| !n.trim().is_empty()) {
        return Some(PanelNote::info("svm-note", note.to_string()));
    }
    if matches!(e.source, Some(EngineSource::Path) | Some(EngineSource::Flatpak)) {
        // A system ScummVM runs the game, but not the build eXo tested it
        // with; the pack fixes that, so the note offers it rather than only
        // warning.
        let text = format!("eXo pins this game to {engine}. Your system's ScummVM will run it instead, which may behave differently.");
        let remedy = pack_remedy(ctx, "scummvm-version", engine, false);
        return Some(match remedy {
            Some(mut r) => {
                r.text = text;
                if let (Some(a), Some(pack)) = (r.action.as_mut(), ctx.emulator_pack.as_ref()) {
                    a.label = format!("Download {engine} ({})", format_bytes(pack.size_bytes));
                }
                r
            }
            None => PanelNote::info("scummvm-version", text),
        });
    }
    None
}

/// The Win9x emulator does not resolve. A pack that can supply it is the
/// remedy; otherwise Windows reports the shared payload's state (the engine
/// comes out of it), and elsewhere the advice is an install hint.
fn engine_missing_note(ctx: &NoteContext, engine: &str) -> Option<PanelNote> {
    let v = ctx.game.as_ref().and_then(|g| g.dosbox_variant.as_deref());
    if let Some(remedy) = pack_remedy(ctx, "engine-missing", engine, true) {
        return Some(remedy);
    }
    if ctx.is_windows {
        let support = ctx.support.as_ref()?;
        return Some(match support.phase.as_str() {
            "failed" => support_failed_note(Family::Win9x),
            "downloading" => support_progress_note(support, Family::Win9x, "OS images + emulators"),
            "missing" if !ctx.installed => PanelNote::info(
                "win9x-support-size",
                format!("{engine} and the shared Windows 9x OS images download automatically with this game{}.", one_time(support.total_bytes)),
            ),
            // "ready" with the emulator gone, or "missing" for an installed game.
            _ => PanelNote::block(
                "engine-missing",
                "The emulator this game needs was not found in the Windows 9x support files (eXo\\emulators inside your library folder). Restore that folder, or delete it and download any Windows 9x game to fetch it again.".into(),
            ),
        });
    }
    Some(PanelNote::block(
        "engine-missing",
        if v == Some("x98") {
            "The emulator this game needs was not found on this system. Install DOSBox-X via your package manager or Flatpak (com.dosbox_x.DOSBox-X)."
        } else {
            "The emulator this game needs was not found on this system. Re-run the installer or place 86Box on your PATH."
        }
        .into(),
    ))
}

/// Most actionable first: a launch that cannot work, then a missing
/// feature, then what merely differs from a DOS game. None when there is
/// nothing to say.
pub fn launch_note(ctx: &NoteContext) -> Option<PanelNote> {
    let g = ctx.game.as_ref();
    let v = g.and_then(|g| g.dosbox_variant.as_deref());
    let runs_under_ece = ctx.engine_info.as_ref().map(|e| e.uses_ece).unwrap_or(ctx.is_windows);
    let dos_engine = ctx.engine_info.as_ref().and_then(|e| e.engine.as_deref());
    let engine = emulator_name(g, ctx.svm_engine.as_ref(), runs_under_ece, dos_engine);

    // This localized variant is a patch; the English game installs with it
    // and stays playable on its own. Informational, and only while it would
    // still cost something.
    if g.map(|g| g.requires_base).unwrap_or(false) && !ctx.installed && !ctx.downloading {
        return Some(PanelNote::info(
            "lp-requires-base",
            "This translation is a patch for the English game, so the English version downloads and installs with it. Both appear in My Games.".into(),
        ));
    }
    // The English row of a group whose translation needed it.
    if let Some(with) = g.and_then(|g| g.installed_with.as_deref()).filter(|_| ctx.installed) {
        return Some(PanelNote::info(
            "lp-installed-with",
            format!("Installed together with {with}, which needed it. Removing this one keeps that version working - it has its own copy."),
        ));
    }
    if is_scummvm(g) {
        return scummvm_note(ctx, &engine);
    }
    if v == Some("pcbox") {
        return Some(PanelNote::block(
            "pcbox",
            "This game needs PCBox, a Windows-only emulator eXorchy does not ship yet - launching it will fail for now.".into(),
        ));
    }
    if ctx.win9x_engine_missing {
        return engine_missing_note(ctx, &engine);
    }
    if ctx.engine_info.as_ref().is_some_and(|e| e.engine.as_deref() == Some("dosbox-x") && !e.engine_available) {
        // The game's own Download fetches DOSBox-X with it (download_game
        // queues the pack): before that, the price; after, the button.
        if !ctx.installed && !ctx.downloading && ctx.pack_job.is_none() {
            if let Some(pack) = &ctx.emulator_pack {
                return Some(PanelNote::info(
                    "dosbox-x-size",
                    format!("eXo runs this game under DOSBox-X: downloading the game also fetches it{}.", one_time(pack.size_bytes)),
                ));
            }
        }
        if let Some(remedy) = pack_remedy(ctx, "engine-missing", &engine, true) {
            return Some(remedy);
        }
        return Some(PanelNote::block(
            "engine-missing",
            "This game runs under DOSBox-X, which was not found. Install dosbox-x (AUR) or its Flatpak (com.dosbox_x.DOSBox-X), or pick DOSBox Staging in Game Settings.".into(),
        ));
    }

    // Engine resolves, but the shared payload (parent OS images, needed on
    // every platform) may still be on its way.
    if let Some(support) = &ctx.support {
        match support.phase.as_str() {
            "failed" => return Some(support_failed_note(Family::Win9x)),
            "downloading" => {
                return Some(support_progress_note(support, Family::Win9x, if ctx.is_windows { "OS images + emulators" } else { "OS images" }));
            }
            "missing" if !ctx.installed && !ctx.downloading => {
                return Some(PanelNote::info(
                    "win9x-support-size",
                    format!("Downloading this game also fetches the shared Windows 9x support files{} - every Windows 9x game uses them.", one_time(support.total_bytes)),
                ));
            }
            _ => {}
        }
    }
    if ctx.printing_unavailable {
        return Some(PanelNote::info(
            "printing",
            "This game can print, but you set it to run under DOSBox Staging, which has no printer. Pick DOSBox-X in Game Settings to print.".into(),
        ));
    }
    if ctx.engine_info.as_ref().is_some_and(|e| e.prints && e.engine.as_deref() == Some("dosbox-x")) {
        return Some(PanelNote::info(
            "printing-x",
            "This game can print: each printed page is saved as a PNG image in the game's !prints folder.".into(),
        ));
    }
    if let Some(mp) = ctx.mp.as_ref().filter(|m| m.multiplayer) {
        match mp.state.as_str() {
            "needs_wired" => {
                return Some(PanelNote::info(
                    "mp-wired",
                    "This game can play online, but that needs a wired network connection - a Wi-Fi link cannot carry the emulated network card's own hardware address, on any system. Single player works either way.".into(),
                ));
            }
            "needs_permission" => {
                return Some(PanelNote::info(
                    "mp-permission",
                    "This game can play online once you allow it in Settings → Network. Single player works either way.".into(),
                ));
            }
            _ => {}
        }
    }
    if let (Some(true), Some(info)) = (v.map(|v| v.starts_with("ece")), ctx.engine_info.as_ref()) {
        if !runs_under_ece && info.engine.as_deref() != Some("dosbox-x") {
            // An override the user chose, a build not yet extracted (Windows),
            // or a platform ECE was never built for.
            let text = if info.ece_available {
                "This game is tuned for DOSBox ECE, but you set it to run under DOSBox Staging - the experience may vary slightly."
            } else if ctx.is_windows {
                "This game is tuned for DOSBox ECE, which eXorchy has not unpacked yet. It runs under DOSBox Staging until then - the experience may vary slightly."
            } else {
                "This game is tuned for DOSBox ECE, which only exists on Windows. eXorchy runs it with DOSBox Staging - the experience may vary slightly."
            };
            return Some(PanelNote::info("ece", text.into()));
        }
    }
    if v == Some("x98") {
        return Some(PanelNote::info(
            "x98-boot",
            "This game boots Windows 98 inside DOSBox-X - the first start takes noticeably longer than a DOS game.".into(),
        ));
    }
    if v.map(|v| v.starts_with("86box")).unwrap_or(false) {
        return Some(PanelNote::info(
            "86box-perf",
            "This game runs under 86Box, a full PC hardware emulator - startup is slower and the system requirements are higher than for other games.".into(),
        ));
    }
    // Last, because it is the least about THIS game.
    if ctx.video_unsupported {
        return Some(PanelNote::info(
            "no-gstreamer",
            "Preview videos are turned off: this system is missing GStreamer plugins. Install gstreamer1.0-plugins-good and gstreamer1.0-libav (names vary by distribution), then restart eXorchy.".into(),
        ));
    }
    None
}

// ── The panel integration ─────────────────────────────────────────────────

/// Native builds only run on Linux; kept as a field so the note logic stays
/// the web UI's, tests included.
const IS_WINDOWS: bool = false;

const DISMISSED_KEY: &str = "dismissed_notes";

/// What the panel knows about the selected row. Probe results land here
/// from the backend callbacks; the tick re-reads the live bits (installed,
/// downloading, offline) and renders.
#[derive(Default)]
struct Live {
    panel: Weak<DetailPanel>,
    row_id: Option<i64>,
    ctx: NoteContext,
    collection: String,
    /// The support payload poll: when the next probe is due, or None once
    /// the phase is terminal for this row.
    support_next: Option<Instant>,
    /// Dismissed note KINDS; None until the stored list has arrived, so a
    /// note silenced weeks ago does not flash on the first open.
    dismissed: Option<Vec<String>>,
    shown: Option<PanelNote>,
    tick: Option<glib::SourceId>,
}

type Shared = Rc<RefCell<Live>>;

/// Mount the note into `panel.note_slot`: probes on every (re)open and row
/// change, re-probes the support payload while it downloads, and follows
/// the emulator pack's install.
pub fn attach(panel: &Rc<DetailPanel>) {
    let live: Shared = Rc::new(RefCell::new(Live { panel: Rc::downgrade(panel), ..Default::default() }));

    {
        let core = app::core();
        let live = live.clone();
        app::spawn(async move { games::get_config(core.state(), DISMISSED_KEY.into()).await }, move |res| {
            let list = res.ok().flatten().map(|s| s.split(',').filter(|k| !k.is_empty()).map(String::from).collect()).unwrap_or_default();
            live.borrow_mut().dismissed = Some(list);
            refresh(&live);
        });
    }

    // Jobs the backend starts by itself (download_game queues the emulator
    // pack) must show their progress without the panel being reopened.
    {
        let live = live.clone();
        app::on_event("content-pack-install-started", move |payload| {
            let collection = payload.get("collection").and_then(|v| v.as_str()).unwrap_or_default();
            let pack_id = payload.get("pack_id").and_then(|v| v.as_str()).unwrap_or_default();
            let matches = {
                let l = live.borrow();
                l.ctx.emulator_pack.as_ref().map(|p| p.collection == collection && p.id == pack_id).unwrap_or(false)
            };
            if matches {
                live.borrow_mut().ctx.pack_job = Some(PackJob { phase: "starting".into(), downloaded_bytes: 0, total_bytes: 0, finished: false });
                refresh(&live);
            }
        });
    }

    let l2 = live.clone();
    panel.on_shown(move |g| {
        if g.is_some() {
            start_tick(&l2);
        } else {
            stop(&l2);
        }
    });
}

fn start_tick(live: &Shared) {
    // Probe right away; the source only keeps following.
    tick(live);
    if live.borrow().tick.is_some() {
        return;
    }
    let l = live.clone();
    let id = glib::timeout_add_local(Duration::from_secs(1), move || {
        if tick(&l) {
            glib::ControlFlow::Continue
        } else {
            l.borrow_mut().tick = None;
            glib::ControlFlow::Break
        }
    });
    live.borrow_mut().tick = Some(id);
}

/// The panel closed: forget the row, clear the slot, retire the tick.
fn stop(live: &Shared) {
    clear(live);
    if let Some(id) = live.borrow_mut().tick.take() {
        id.remove();
    }
}

fn clear(live: &Shared) {
    let panel = {
        let mut l = live.borrow_mut();
        l.row_id = None;
        l.ctx = NoteContext::default();
        l.shown = None;
        l.support_next = None;
        l.panel.upgrade()
    };
    if let Some(p) = panel {
        render(&p, None);
    }
}

/// One second of the panel's life: follow the selected row, run the polls
/// that are due, render. False once the panel closed.
fn tick(live: &Shared) -> bool {
    let Some(panel) = live.borrow().panel.upgrade() else { return false };
    if !panel.is_open() {
        // Returning false retires the source; `stop` must not remove it
        // from inside its own callback.
        clear(live);
        return false;
    }
    let row = panel.selected_game();
    let id = row.as_ref().and_then(|g| g.id);
    let changed = live.borrow().row_id != id;
    if changed {
        {
            let mut l = live.borrow_mut();
            l.row_id = id;
            l.ctx = NoteContext {
                game: row.clone(),
                is_windows: IS_WINDOWS,
                video_unsupported: crate::ui::media::video_supported() == Some(false),
                ..Default::default()
            };
            l.collection = row.as_ref().and_then(|g| g.torrent_source.clone()).unwrap_or_else(|| "eXoWin9x".into());
            l.support_next = None;
        }
        if let (Some(row), Some(id)) = (row.as_ref(), id) {
            probe_row(live, row, id);
        }
    } else {
        live.borrow_mut().ctx.game = row;
    }
    let support_due = live.borrow().support_next.map(|t| t <= Instant::now()).unwrap_or(false);
    if support_due {
        probe_support(live);
    }
    let job_running = live.borrow().ctx.pack_job.as_ref().map(|j| !j.finished).unwrap_or(false);
    if job_running {
        poll_pack(live);
    }
    refresh(live);
    true
}

fn still(live: &Shared, id: i64) -> bool {
    live.borrow().row_id == Some(id)
}

fn probe_row(live: &Shared, row: &Game, id: i64) {
    let win9x = is_win9x(Some(row));
    let svm = is_scummvm(Some(row));
    let variant = row.dosbox_variant.clone();

    {
        let (core, l) = (app::core(), live.clone());
        probe_dos_engine(&l, id);
        let l = live.clone();
        app::spawn(async move { games::game_printing_unavailable(core.state(), id).await }, move |res| {
            if let Ok(p) = res {
                if still(&l, id) {
                    l.borrow_mut().ctx.printing_unavailable = p;
                    refresh(&l);
                }
            }
        });
    }
    if win9x {
        probe_engine(live, id, variant.clone());
        let (core, l) = (app::core(), live.clone());
        app::spawn(async move { win9x::win9x_multiplayer_info(core.state(), id).await }, move |res| {
            if let Ok(mp) = res {
                if still(&l, id) {
                    l.borrow_mut().ctx.mp = Some(mp);
                    refresh(&l);
                }
            }
        });
        if let Some(pack_id) = emulator_pack_id(variant.as_deref()) {
            let collection = live.borrow().collection.clone();
            load_pack(live, id, pack_id.to_string(), collection);
        }
    }
    if svm {
        let (core, l) = (app::core(), live.clone());
        app::spawn(async move { scummvm::scummvm_engine_info(core.state(), id).await }, move |res| {
            if let Ok(e) = res {
                if still(&l, id) {
                    let pack_id = e.pack_id.clone();
                    l.borrow_mut().ctx.svm_engine = Some(e);
                    refresh(&l);
                    if let Some(pid) = pack_id {
                        let collection = l.borrow().collection.clone();
                        load_pack(&l, id, pid, collection);
                    }
                }
            }
        });
        let (core, l) = (app::core(), live.clone());
        app::spawn(async move { scummvm::scummvm_variants(core.state(), id).await }, move |res| {
            if let Ok(v) = res {
                if still(&l, id) {
                    l.borrow_mut().ctx.svm_note = v.and_then(|v| v.note);
                    refresh(&l);
                }
            }
        });
    }
    // The payload matters most when the game installed first and Play would
    // otherwise fail bare, so this is not gated on the engine or install state.
    if (win9x || svm) && !bus::offline() {
        probe_support(live);
    }
}

/// Is the Win9x emulator resolvable? Re-run when a pack lands or the payload
/// turns ready.
fn probe_engine(live: &Shared, id: i64, variant: Option<String>) {
    let (core, l) = (app::core(), live.clone());
    app::spawn(async move { win9x::win9x_engine_available(core.clone(), core.state(), variant).await }, move |res| {
        if let Ok(ok) = res {
            if still(&l, id) {
                l.borrow_mut().ctx.win9x_engine_missing = !ok;
                refresh(&l);
            }
        }
    });
}

fn probe_support(live: &Shared) {
    let (id, svm, variant) = {
        let mut l = live.borrow_mut();
        l.support_next = None;
        let Some(id) = l.row_id else { return };
        let g = l.ctx.game.as_ref();
        (id, is_scummvm(g), g.and_then(|g| g.dosbox_variant.clone()))
    };
    let (core, l) = (app::core(), live.clone());
    app::spawn(
        async move {
            if svm {
                scummvm::get_scummvm_support_status(core.state()).await
            } else {
                win9x::get_win9x_support_status(core.state(), variant.clone()).await
            }
        },
        move |res| {
            let Ok(s) = res else { return };
            if !still(&l, id) {
                return;
            }
            let phase = s.phase.clone();
            let (engine_missing, variant) = {
                let mut b = l.borrow_mut();
                b.ctx.support = Some(s);
                // A live bar for an active download; a steady "missing" only
                // needs to notice a download started elsewhere eventually.
                b.support_next = match phase.as_str() {
                    "failed" | "ready" => None,
                    "downloading" => Some(Instant::now() + Duration::from_secs(3)),
                    _ => Some(Instant::now() + Duration::from_secs(10)),
                };
                (b.ctx.win9x_engine_missing, b.ctx.game.as_ref().and_then(|g| g.dosbox_variant.clone()))
            };
            if phase == "ready" && !svm && engine_missing {
                probe_engine(&l, id, variant);
            }
            refresh(&l);
        },
    );
}

/// Which emulator a DOS / Windows 3.x game runs under and whether it is
/// there; offers the DOSBox-X pack when it is not.
fn probe_dos_engine(live: &Shared, id: i64) {
    let (core, l) = (app::core(), live.clone());
    app::spawn(async move { games::game_engine_info(core.state(), id).await }, move |res| {
        let Ok(e) = res else { return };
        if !still(&l, id) {
            return;
        }
        let needs_x = e.engine.as_deref() == Some("dosbox-x") && !e.engine_available;
        l.borrow_mut().ctx.engine_info = Some(EngineInfo {
            ece_available: e.ece_available,
            uses_ece: e.uses_ece,
            engine: e.engine,
            prints: e.prints,
            engine_available: e.engine_available,
        });
        refresh(&l);
        if needs_x {
            load_home_pack(&l, id, "dosbox-x");
        }
    });
}

/// `load_pack` for a pack another collection owns (DOSBox-X for DOS games).
fn load_home_pack(live: &Shared, id: i64, pack_id: &'static str) {
    let l = live.clone();
    app::spawn(async move { content_packs::content_pack_collection(pack_id.to_string()).await }, move |res| {
        if let Ok(Some(collection)) = res {
            if still(&l, id) {
                load_pack(&l, id, pack_id.to_string(), collection);
            }
        }
    });
}

/// The pack that could supply the emulator (available, not installed), and
/// its install if one is already running.
fn load_pack(live: &Shared, id: i64, pack_id: String, collection: String) {
    let (core, l) = (app::core(), live.clone());
    let col = collection.clone();
    app::spawn(async move { content_packs::list_content_packs(core.state(), col).await }, move |res| {
        if !still(&l, id) {
            return;
        }
        let pack = res.ok().and_then(|packs| {
            packs.into_iter().find(|p| p.id == pack_id && p.available && !p.installed).map(|p| EmulatorPack { id: p.id, collection: collection.clone(), display_name: p.display_name, size_bytes: p.size_bytes })
        });
        let has = pack.is_some();
        l.borrow_mut().ctx.emulator_pack = pack;
        if has {
            // A job the backend started before the panel opened (an earlier
            // Download) shows as progress rather than as a second button.
            poll_pack(&l);
        }
        refresh(&l);
    });
}

fn poll_pack(live: &Shared) {
    let (id, collection, pack_id) = {
        let l = live.borrow();
        let Some(id) = l.row_id else { return };
        let Some(p) = l.ctx.emulator_pack.as_ref() else { return };
        (id, p.collection.clone(), p.id.clone())
    };
    let (core, l) = (app::core(), live.clone());
    let (col, pid) = (collection.clone(), pack_id.clone());
    app::spawn(async move { content_packs::get_content_pack_progress(core.state(), col, pid).await }, move |res| {
        if !still(&l, id) {
            return;
        }
        let Ok(progress) = res else { return };
        match progress {
            None => {
                // No job on the backend: drop the entry, or the note keeps
                // showing a download nobody is running.
                l.borrow_mut().ctx.pack_job = None;
            }
            Some(p) => {
                let finished = p.finished;
                l.borrow_mut().ctx.pack_job = Some(PackJob { phase: p.phase, downloaded_bytes: p.downloaded_bytes, total_bytes: p.total_bytes, finished });
                if finished {
                    l.borrow_mut().ctx.pack_job = None;
                    if p.installed {
                        // The pack landed: the engine resolves now and the
                        // pack is no longer on offer.
                        let variant = l.borrow().ctx.game.as_ref().and_then(|g| g.dosbox_variant.clone());
                        l.borrow_mut().ctx.emulator_pack = None;
                        let game = l.borrow().ctx.game.clone();
                        if is_win9x(game.as_ref()) {
                            probe_engine(&l, id, variant);
                        } else if !is_scummvm(game.as_ref()) {
                            probe_dos_engine(&l, id);
                        } else {
                            let (core, l2) = (app::core(), l.clone());
                            app::spawn(async move { scummvm::scummvm_engine_info(core.state(), id).await }, move |res| {
                                if let Ok(e) = res {
                                    if still(&l2, id) {
                                        l2.borrow_mut().ctx.svm_engine = Some(e);
                                        refresh(&l2);
                                    }
                                }
                            });
                        }
                    } else if let Some(err) = p.error.filter(|e| e != "Cancelled") {
                        let name = l.borrow().ctx.emulator_pack.as_ref().map(|p| p.display_name.clone()).unwrap_or(pack_id.clone());
                        bus::toast_with(&format!("Couldn't install {name}"), Some(&err), None);
                    }
                }
            }
        }
        refresh(&l);
    });
}

/// The note's remedy: start the emulator pack's install and follow it.
fn install_pack(live: &Shared, pack_id: String) {
    let (collection, name) = {
        let l = live.borrow();
        (
            l.ctx.emulator_pack.as_ref().map(|p| p.collection.clone()).unwrap_or_else(|| l.collection.clone()),
            l.ctx.emulator_pack.as_ref().map(|p| p.display_name.clone()).unwrap_or_else(|| pack_id.clone()),
        )
    };
    // Claim the row now; the first poll is a second out.
    live.borrow_mut().ctx.pack_job = Some(PackJob { phase: "starting".into(), downloaded_bytes: 0, total_bytes: 0, finished: false });
    refresh(live);
    let (core, l) = (app::core(), live.clone());
    let col = collection.clone();
    app::spawn(async move { content_packs::install_content_pack(core.clone(), col, pack_id).await }, move |res| {
        if let Err(e) = res {
            l.borrow_mut().ctx.pack_job = None;
            bus::toast_with(&format!("Couldn't start the {name} download"), Some(&e), None);
            refresh(&l);
        }
    });
}

fn dismiss(live: &Shared, key: &str) {
    let list = {
        let mut l = live.borrow_mut();
        let list = l.dismissed.get_or_insert_with(Vec::new);
        if !list.iter().any(|k| k == key) {
            list.push(key.to_string());
        }
        list.join(",")
    };
    refresh(live);
    let core = app::core();
    app::spawn(async move { games::set_config(core.clone(), core.state(), DISMISSED_KEY.into(), list).await }, |res| {
        if let Err(e) = res {
            log::warn!("dismissed_notes: {e}");
        }
    });
}

/// Compute the note for the current context and render it if it changed.
/// Dismissal is remembered per note KIND (the ECE note is the same sentence
/// on ~2,000 titles).
fn refresh(live: &Shared) {
    let (panel, note) = {
        let mut l = live.borrow_mut();
        let Some(panel) = l.panel.upgrade() else { return };
        let Some(id) = l.row_id else { return };
        let dl = downloads::state(id);
        l.ctx.offline = bus::offline();
        l.ctx.installed = l.ctx.game.as_ref().map(|g| g.installed).unwrap_or(false) || dl.as_ref().map(|d| d.installed).unwrap_or(false);
        l.ctx.downloading = dl.as_ref().map(|d| d.downloading).unwrap_or(false);
        let note = launch_note(&l.ctx).filter(|n| n.blocking || l.dismissed.as_ref().map(|d| !d.iter().any(|k| k == n.key)).unwrap_or(false));
        if l.shown == note {
            return;
        }
        l.shown = note.clone();
        (panel, note)
    };
    render_with(&panel, live, note);
}

fn render(panel: &Rc<DetailPanel>, note: Option<PanelNote>) {
    render_note(panel, note, None);
}

fn render_with(panel: &Rc<DetailPanel>, live: &Shared, note: Option<PanelNote>) {
    render_note(panel, note, Some(live));
}

fn render_note(panel: &Rc<DetailPanel>, note: Option<PanelNote>, live: Option<&Shared>) {
    let slot = &panel.note_slot;
    while let Some(c) = slot.first_child() {
        slot.remove(&c);
    }
    panel.set_play_blocked(note.as_ref().filter(|n| n.blocking).map(|n| n.text.clone()));
    panel.set_play_pending(note.as_ref().map(|n| n.pending).unwrap_or(false));
    let Some(n) = note else {
        slot.set_visible(false);
        return;
    };
    let card = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(9).css_classes(["launch-note"]).build();
    if n.blocking {
        card.add_css_class("is-blocking");
    }
    let mark = gtk::Label::builder().label(if n.blocking { "!" } else { "i" }).css_classes(["note-mark"]).valign(gtk::Align::Start).build();
    card.append(&mark);
    let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).hexpand(true).build();
    let text = gtk::Label::builder().label(&n.text).xalign(0.0).wrap(true).wrap_mode(gtk::pango::WrapMode::WordChar).selectable(true).css_classes(["note-text"]).build();
    column.append(&text);
    if let (Some(a), Some(live)) = (&n.action, live) {
        let b = gtk::Button::builder().label(&a.label).halign(gtk::Align::Start).css_classes(["btn", "primary", "note-action"]).build();
        let (l, pid) = (live.clone(), a.pack_id.clone());
        b.connect_clicked(move |_| install_pack(&l, pid.clone()));
        column.append(&b);
    }
    card.append(&column);
    if !n.blocking {
        if let Some(live) = live {
            let x = gtk::Button::builder().label("✕").valign(gtk::Align::Start).css_classes(["note-dismiss"]).tooltip_text("Don't show this note again").build();
            let (l, key) = (live.clone(), n.key);
            x.connect_clicked(move |_| dismiss(&l, key));
            card.append(&x);
        }
    }
    slot.append(&card);
    slot.set_visible(true);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(source: &str, variant: Option<&str>) -> Game {
        Game { id: Some(1), title: "T".into(), torrent_source: Some(source.into()), dosbox_variant: variant.map(String::from), ..Default::default() }
    }

    fn ctx() -> NoteContext {
        NoteContext { game: Some(game("eXoDOS", None)), ..Default::default() }
    }

    fn pack() -> EmulatorPack {
        EmulatorPack { id: "dosbox-x".into(), collection: "eXoWin9x".into(), display_name: "DOSBox-X".into(), size_bytes: 50_000_000 }
    }

    fn support(phase: &str, progress: f32, total: u64) -> Win9xSupportStatus {
        Win9xSupportStatus { phase: phase.into(), progress, total_bytes: total }
    }

    fn svm_engine(available: bool, version: &str, source: Option<EngineSource>, pack_id: Option<&str>) -> ScummVmEngineInfo {
        ScummVmEngineInfo { available, pinned_version: version.into(), source, pack_id: pack_id.map(String::from) }
    }

    fn job(phase: &str, done: u64, total: u64) -> PackJob {
        PackJob { phase: phase.into(), downloaded_bytes: done, total_bytes: total, finished: false }
    }

    fn mp(state: &str) -> Win9xMultiplayerInfo {
        Win9xMultiplayerInfo { multiplayer: true, state: state.into(), prompt: false }
    }

    fn key(c: &NoteContext) -> Option<&'static str> {
        launch_note(c).map(|n| n.key)
    }

    fn svm() -> NoteContext {
        NoteContext { game: Some(game("eXoScummVM", Some("2.9.0"))), ..Default::default() }
    }

    fn x98(missing: bool) -> NoteContext {
        NoteContext { game: Some(game("eXoWin9x", Some("x98"))), win9x_engine_missing: missing, ..Default::default() }
    }

    #[test]
    fn plain_dos_game_says_nothing() {
        assert!(launch_note(&ctx()).is_none());
    }

    #[test]
    fn scummvm_quiet_while_probe_open_and_blocks_without_engine() {
        assert!(launch_note(&svm()).is_none());
        let mut c = svm();
        c.svm_engine = Some(svm_engine(false, "2.9.0", None, None));
        let n = launch_note(&c).unwrap();
        assert_eq!(n.key, "engine-missing");
        assert!(n.blocking);
        assert!(n.text.contains("ScummVM 2.9.0"));
        assert!(n.text.contains("org.scummvm.ScummVM"));
        c.is_windows = true;
        assert!(!launch_note(&c).unwrap().text.contains("Flatpak"));
    }

    #[test]
    fn uninstalled_scummvm_game_states_the_engine_cost() {
        let svm_pack = EmulatorPack { id: "scummvm-2.8.0".into(), collection: "eXoScummVM".into(), display_name: "ScummVM 2.8.0".into(), size_bytes: 127_745_114 };
        let mut c = svm();
        c.svm_engine = Some(svm_engine(false, "2.8.0", None, Some("scummvm-2.8.0")));
        c.emulator_pack = Some(svm_pack);
        let n = launch_note(&c).unwrap();
        assert_eq!(n.key, "scummvm-engine-size");
        assert!(!n.blocking);
        assert!(n.action.is_none());
        assert!(n.text.contains(&format!("one-time {}", format_bytes(127_745_114))));
        c.installed = true;
        assert_eq!(key(&c), Some("engine-missing"));
        c.installed = false;
        c.downloading = true;
        assert_eq!(key(&c), Some("engine-missing"));
        let mut bare = svm();
        bare.svm_engine = Some(svm_engine(false, "2.8.0", None, None));
        assert!(launch_note(&bare).unwrap().text.contains("scummvm.org"));
    }

    #[test]
    fn on_windows_the_scummvm_engine_follows_the_payload() {
        let win = |support: Option<Win9xSupportStatus>, installed: bool, offline: bool| {
            let mut c = svm();
            c.is_windows = true;
            c.svm_engine = Some(svm_engine(false, "2.8.0", None, None));
            c.support = support;
            c.installed = installed;
            c.offline = offline;
            launch_note(&c)
        };
        let size = win(Some(support("missing", 0.0, 200_000_000)), false, false).unwrap();
        assert_eq!(size.key, "scummvm-engine-size");
        assert!(!size.blocking);
        assert!(size.text.contains("eXo's ScummVM 2.8.0"));
        assert!(!size.text.contains("scummvm.org"));
        let dl = win(Some(support("downloading", 0.42, 1)), false, false).unwrap();
        assert_eq!(dl.key, "scummvm-support-progress");
        assert_eq!(dl.text, "Downloading the ScummVM support files (ScummVM 2.8.0)… 42%");
        assert_eq!(win(Some(support("failed", 1.0, 1)), false, false).unwrap().key, "scummvm-support-failed");
        let gone = win(Some(support("missing", 0.0, 1)), true, false).unwrap();
        assert!(gone.blocking);
        assert!(gone.text.contains("scmvm"));
        let off = win(None, false, true).unwrap();
        assert!(off.text.contains("Go online"));
        assert!(win(None, false, false).unwrap().text.contains("scummvm.org"));
    }

    #[test]
    fn offers_the_pinned_build_pack_and_on_a_system_scummvm_too() {
        let svm_pack = EmulatorPack { id: "scummvm-2.9.0".into(), collection: "eXoScummVM".into(), display_name: "ScummVM 2.9.0".into(), size_bytes: 50_000_000 };
        let mut c = svm();
        c.svm_engine = Some(svm_engine(false, "2.9.0", None, Some("scummvm-2.9.0")));
        c.emulator_pack = Some(svm_pack.clone());
        c.installed = true;
        let n = launch_note(&c).unwrap();
        assert_eq!(n.key, "engine-missing");
        assert!(n.blocking);
        assert!(!n.text.contains("scummvm.org"));
        assert_eq!(n.action.as_ref().unwrap().pack_id, "scummvm-2.9.0");
        c.pack_job = Some(job("downloading", 20, 100));
        assert_eq!(launch_note(&c).unwrap().text, "Downloading ScummVM 2.9.0… 20%");
        let mut sys = svm();
        sys.svm_engine = Some(svm_engine(true, "2.9.0", Some(EngineSource::Path), Some("scummvm-2.9.0")));
        sys.emulator_pack = Some(svm_pack);
        let v = launch_note(&sys).unwrap();
        assert_eq!(v.key, "scummvm-version");
        assert!(!v.blocking);
        assert!(v.text.contains("eXo pins this game"));
        assert_eq!(v.action.unwrap().label, format!("Download ScummVM 2.9.0 ({})", format_bytes(50_000_000)));
        sys.emulator_pack = None;
        assert!(launch_note(&sys).unwrap().action.is_none());
    }

    #[test]
    fn exo_note_outranks_version_warning_never_missing_engine() {
        let mut c = svm();
        c.svm_engine = Some(svm_engine(true, "2.9.0", Some(EngineSource::Path), None));
        c.svm_note = Some("Audio is off in this port.".into());
        let n = launch_note(&c).unwrap();
        assert_eq!(n.key, "svm-note");
        assert!(!n.blocking);
        c.svm_engine = Some(svm_engine(false, "2.9.0", Some(EngineSource::Path), None));
        assert_eq!(key(&c), Some("engine-missing"));
    }

    #[test]
    fn warns_only_when_a_system_scummvm_ignores_the_pin() {
        for (source, expect) in [
            (EngineSource::Path, Some("scummvm-version")),
            (EngineSource::Flatpak, Some("scummvm-version")),
            (EngineSource::Pack, None),
            (EngineSource::Exo, None),
        ] {
            let mut c = svm();
            c.svm_engine = Some(svm_engine(true, "2.9.0", Some(source), None));
            assert_eq!(key(&c), expect);
        }
    }

    #[test]
    fn language_pack_notes() {
        let mut c = ctx();
        c.game.as_mut().unwrap().requires_base = true;
        assert_eq!(key(&c), Some("lp-requires-base"));
        assert!(!launch_note(&c).unwrap().blocking);
        c.installed = true;
        assert_ne!(key(&c), Some("lp-requires-base"));
        c.installed = false;
        c.downloading = true;
        assert_ne!(key(&c), Some("lp-requires-base"));

        let mut d = ctx();
        d.game.as_mut().unwrap().installed_with = Some("Alien Odyssey (Deutsch)".into());
        assert_ne!(key(&d), Some("lp-installed-with"));
        d.installed = true;
        assert_eq!(key(&d), Some("lp-installed-with"));
        assert!(!launch_note(&d).unwrap().blocking);
    }

    #[test]
    fn pcbox_blocks_regardless() {
        let c = NoteContext { game: Some(game("eXoWin9x", Some("pcbox"))), ..Default::default() };
        assert_eq!(key(&c), Some("pcbox"));
    }

    #[test]
    fn reports_the_running_pack_install_with_its_phase() {
        let mut c = x98(true);
        c.emulator_pack = Some(pack());
        c.pack_job = Some(job("downloading", 40, 100));
        let n = launch_note(&c).unwrap();
        assert_eq!(n.text, "Downloading DOSBox-X… 40%");
        assert!(n.pending);
        c.pack_job = Some(job("extracting", 100, 100));
        assert_eq!(launch_note(&c).unwrap().text, "Installing DOSBox-X…");
    }

    #[test]
    fn only_work_in_flight_is_pending() {
        let mut c = x98(true);
        c.is_windows = true;
        c.support = Some(support("downloading", 0.5, 1));
        assert!(launch_note(&c).unwrap().pending);
        let mut d = x98(true);
        d.emulator_pack = Some(pack());
        assert!(!launch_note(&d).unwrap().pending);
        c.support = Some(support("failed", 1.0, 1));
        assert!(!launch_note(&c).unwrap().pending);
    }

    #[test]
    fn offers_the_pack_online_and_points_at_settings_offline() {
        let mut c = x98(true);
        c.emulator_pack = Some(pack());
        let n = launch_note(&c).unwrap();
        assert!(n.blocking);
        let a = n.action.unwrap();
        assert_eq!(a.label, format!("Download emulator ({})", format_bytes(50_000_000)));
        assert_eq!(a.pack_id, "dosbox-x");
        c.offline = true;
        let off = launch_note(&c).unwrap();
        assert!(off.action.is_none());
        assert!(off.text.contains("Go online"));
    }

    #[test]
    fn without_a_pack_windows_reports_the_payload_linux_an_install_hint() {
        let mut c = x98(true);
        c.is_windows = true;
        assert!(launch_note(&c).is_none());
        c.support = Some(support("downloading", 0.5, 1));
        assert_eq!(key(&c), Some("win9x-support-progress"));
        c.support = Some(support("failed", 1.0, 1));
        assert_eq!(key(&c), Some("win9x-support-failed"));
        c.support = Some(support("missing", 0.0, 1));
        assert_eq!(key(&c), Some("win9x-support-size"));
        c.installed = true;
        assert!(launch_note(&c).unwrap().text.contains("Restore that folder"));
        assert!(launch_note(&x98(true)).unwrap().text.contains("com.dosbox_x.DOSBox-X"));
        let mut b = x98(true);
        b.game = Some(game("eXoWin9x", Some("86box")));
        assert!(launch_note(&b).unwrap().text.contains("86Box on your PATH"));
    }

    #[test]
    fn announces_the_one_time_payload_before_the_first_download_only() {
        let mut c = x98(false);
        c.support = Some(support("missing", 0.0, 2_500_000_000));
        let n = launch_note(&c).unwrap();
        assert_eq!(n.key, "win9x-support-size");
        assert!(n.text.contains(&format!("one-time {}", format_bytes(2_500_000_000))));
        c.installed = true;
        assert_eq!(key(&c), Some("x98-boot"));
        c.installed = false;
        c.downloading = true;
        assert_eq!(key(&c), Some("x98-boot"));
    }

    #[test]
    fn multiplayer_notes_outrank_the_boot_note() {
        let mut c = x98(false);
        c.mp = Some(mp("needs_wired"));
        assert_eq!(key(&c), Some("mp-wired"));
        c.mp = Some(mp("needs_permission"));
        assert_eq!(key(&c), Some("mp-permission"));
        c.mp = Some(mp("ready"));
        assert_eq!(key(&c), Some("x98-boot"));
        let b = NoteContext { game: Some(game("eXoWin9x", Some("86box"))), ..Default::default() };
        assert_eq!(key(&b), Some("86box-perf"));
    }

    #[test]
    fn dos_printing_and_ece_notes() {
        let ece = |info: Option<EngineInfo>| NoteContext { game: Some(game("eXoDOS", Some("ece4230"))), engine_info: info, ..Default::default() };
        let mut p = ece(Some(EngineInfo { ece_available: false, uses_ece: false, ..Default::default() }));
        p.printing_unavailable = true;
        assert_eq!(key(&p), Some("printing"));
        assert!(launch_note(&ece(None)).is_none());
        assert!(launch_note(&ece(Some(EngineInfo { ece_available: true, uses_ece: true, ..Default::default() }))).is_none());
        assert!(launch_note(&ece(Some(EngineInfo { ece_available: true, uses_ece: false, ..Default::default() }))).unwrap().text.contains("you set it"));
        let mut w = ece(Some(EngineInfo { ece_available: false, uses_ece: false, ..Default::default() }));
        w.is_windows = true;
        assert!(launch_note(&w).unwrap().text.contains("not unpacked yet"));
        assert!(launch_note(&ece(Some(EngineInfo { ece_available: false, uses_ece: false, ..Default::default() }))).unwrap().text.contains("only exists on Windows"));
        let mut v = ctx();
        v.video_unsupported = true;
        assert_eq!(key(&v), Some("no-gstreamer"));
        let mut e = ece(Some(EngineInfo { ece_available: false, uses_ece: false, ..Default::default() }));
        e.video_unsupported = true;
        assert_eq!(key(&e), Some("ece"));
    }

    #[test]
    fn dosbox_x_games_name_it_and_offer_it() {
        let x = |info: EngineInfo| NoteContext { game: Some(game("eXoDOS", Some("x"))), engine_info: Some(info), ..Default::default() };
        let needs = EngineInfo { engine: Some("dosbox-x".into()), engine_available: false, ..Default::default() };
        assert_eq!(emulator_name(Some(&game("eXoDOS", Some("x"))), None, false, Some("dosbox-x")), "DOSBox-X");
        assert_eq!(emulator_name(Some(&game("eXoDOS", Some("x"))), None, false, Some("staging")), "DOSBox Staging");
        // Missing, no pack known: a blocking note that names the way out.
        let c = x(needs.clone());
        let n = launch_note(&c).unwrap();
        assert_eq!(n.key, "engine-missing");
        assert!(n.blocking && n.text.contains("Game Settings"));
        // Pack known, game not downloaded: the price, since Download brings it.
        let pack = EmulatorPack { id: "dosbox-x".into(), collection: "eXoWin9x".into(), display_name: "DOSBox-X".into(), size_bytes: 43_000_000 };
        let mut c = x(needs.clone());
        c.emulator_pack = Some(pack.clone());
        assert_eq!(key(&c), Some("dosbox-x-size"));
        // Installed without it: the download button.
        c.installed = true;
        let n = launch_note(&c).unwrap();
        assert_eq!(n.action.map(|a| a.pack_id), Some("dosbox-x".into()));
        // Present: nothing to say unless it prints.
        let ok = EngineInfo { engine: Some("dosbox-x".into()), engine_available: true, ..Default::default() };
        assert!(launch_note(&x(ok.clone())).is_none());
        assert_eq!(key(&x(EngineInfo { prints: true, ..ok })), Some("printing-x"));
        // Printing game forced onto Staging.
        let mut st = x(EngineInfo { engine: Some("staging".into()), engine_available: true, prints: true, ..Default::default() });
        st.printing_unavailable = true;
        assert_eq!(key(&st), Some("printing"));
    }

    #[test]
    fn variant_label_keeps_the_parenthesised_part() {
        assert_eq!(svm_variant_label("Maniac Mansion (DOS v1)"), "DOS v1");
        assert_eq!(svm_variant_label("Maniac Mansion (NES, English, USA)"), "NES, English, USA");
        assert_eq!(svm_variant_label("Other Languages"), "Other Languages");
    }

    #[test]
    fn emulator_names_and_pack_ids() {
        assert_eq!(emulator_name(Some(&game("eXoDOS", None)), None, false, None), "DOSBox Staging");
        assert_eq!(emulator_name(Some(&game("eXoDOS", Some("ece4230"))), None, true, None), "DOSBox ECE");
        assert_eq!(emulator_name(Some(&game("eXoDOS", Some("ece4230"))), None, false, None), "DOSBox Staging");
        assert_eq!(emulator_name(Some(&game("eXoWin9x", Some("x98"))), None, false, None), "DOSBox-X");
        assert_eq!(emulator_name(Some(&game("eXoWin9x", Some("86boxME"))), None, false, None), "86Box");
        assert_eq!(emulator_name(Some(&game("eXoScummVM", None)), None, false, None), "ScummVM");
        let e = svm_engine(true, "2.8.0", Some(EngineSource::Pack), None);
        assert_eq!(emulator_name(Some(&game("eXoScummVM", None)), Some(&e), false, None), "ScummVM 2.8.0");
        assert_eq!(emulator_pack_id(Some("x98")), Some("dosbox-x"));
        assert_eq!(emulator_pack_id(Some("86boxME")), Some("86box"));
        assert_eq!(emulator_pack_id(Some("pcbox")), None);
        assert!(is_win9x(Some(&game("eXoWin9x", Some("86boxME")))));
        assert!(!is_win9x(Some(&game("eXoDOS", None))));
    }
}
