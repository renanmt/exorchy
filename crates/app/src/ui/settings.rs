//! The Settings page: shown in place of the library (gear button, Ctrl+,),
//! a list of sections beside a page stack. Each section is a module under
//! `settings/`; this root owns the frame, the navigation and the pure logic
//! the pages share (collection set normalisation, rate-limit parsing, labels).
//!
//! Sections: general, collections, hidden, emulators, appearance, storage,
//! network, packs, about.

mod about;
mod appearance;
mod collections;
mod emulators;
mod general;
mod hidden;
mod network;
pub mod packs;
pub(crate) mod storage;
mod widgets;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::ui::util::format_bytes;
pub use widgets::Ctx;

/// Pages that want to know when they come into view, by section id.
type Shown = HashMap<String, Rc<dyn Fn()>>;

/// The sections in sidebar order: id, label, icon.
const SECTIONS: [(&str, &str, &str); 9] = [
    ("general", "General", "preferences-system-symbolic"),
    ("collections", "Collections", "view-list-symbolic"),
    ("hidden", "Hidden titles", "view-conceal-symbolic"),
    ("emulators", "Emulators", "applications-games-symbolic"),
    ("appearance", "Appearance", "preferences-desktop-appearance-symbolic"),
    ("storage", "Storage", "drive-harddisk-symbolic"),
    ("network", "Network", "network-workgroup-symbolic"),
    ("packs", "Content Packs", "package-x-generic-symbolic"),
    ("about", "About", "help-about-symbolic"),
];

/// The base collection: always enabled, never switchable.
pub const BASE_COLLECTION: &str = "eXoDOS";

/// Display order for the collection switches (the backend lists language
/// packs first; the user reads the base collection first).
pub const COLLECTION_ORDER: [&str; 7] =
    ["eXoDOS", "eXoDOS_GLP", "eXoDOS_SLP", "eXoDOS_PLP", "eXoWin3x", "eXoWin9x", "eXoScummVM"];

/// The content-pack store's job key (`collection:pack`).
pub const DOSBOX_JOB_KEY: &str = "eXoDOS:dosbox-staging";

/// `set_rate_limits` takes u32 KB/s; anything larger would fail as an
/// integer error rather than a speed-limit one.
const MAX_KBPS: u32 = 4_000_000;

/// The window stack's name for the Settings page.
const PAGE: &str = "settings";

/// Open Settings on `section` ("general", "collections", "hidden", "emulators",
/// "appearance", "storage", "network", "packs", "about") in place of the
/// library: the gear button and Ctrl+,. Built fresh each time; closing
/// returns to the library and drops the page.
pub fn open(parent: &impl IsA<gtk::Widget>, section: &str) {
    let Some(window) = parent.root().and_then(|r| r.downcast::<gtk::Window>().ok()) else {
        log::warn!("settings: parent has no window yet");
        return;
    };
    let view = build(&window, section, Rc::new(|| crate::ui::window::close_page(PAGE)));
    crate::ui::window::show_page(PAGE, &view);
}

/// Developer aid for snapshots: `EXORCHY_SNAPSHOT_SETTINGS=<section>` opens
/// Settings on that section. The window calls it once the library page
/// is shown.
pub fn autoopen_for_snapshot(parent: &gtk::Window) {
    if let Ok(section) = std::env::var("EXORCHY_SNAPSHOT_SETTINGS") {
        let section = if section.is_empty() { "general".to_string() } else { section };
        open(parent, &section);
    }
}

/// The Settings page: the section list and the section side by side, or,
/// in a narrow tile, the list then the section with back arrows (an
/// `AdwNavigationSplitView`). `on_close` leaves the page.
fn build(window: &gtk::Window, section: &str, on_close: Rc<dyn Fn()>) -> gtk::Widget {
    packs::init_events();

    let alive = Rc::new(Cell::new(true));
    // Rows stack in a narrow tile; a fresh page starts wide until the
    // breakpoint says otherwise.
    let view_id = widgets::begin_view();

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .hexpand(true)
        .vexpand(true)
        .build();
    let nav = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["settings-nav"])
        .valign(gtk::Align::Start)
        .margin_start(8)
        .margin_end(8)
        .margin_top(4)
        .build();
    for (id, label, icon) in SECTIONS {
        let row = gtk::ListBoxRow::builder().css_classes(["settings-nav-item"]).build();
        let inner = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).build();
        inner.append(&gtk::Image::from_icon_name(icon));
        inner.append(&gtk::Label::builder().label(label).xalign(0.0).build());
        row.set_child(Some(&inner));
        row.set_widget_name(id);
        nav.append(&row);
    }

    // ── the split view: section list | section ──
    let back = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .css_classes(["btn", "icon", "ghost"])
        .tooltip_text("Back to the library (Esc)")
        .build();
    let side_header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .css_classes(["settings-header"])
        .build();
    side_header.pack_start(&back);
    let side_view = adw::ToolbarView::builder().css_classes(["settings-side"]).build();
    side_view.add_top_bar(&side_header);
    side_view.set_content(Some(&gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&nav).build()));
    let sidebar = adw::NavigationPage::builder().title("Settings").tag("sections").child(&side_view).build();

    let content_header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .css_classes(["settings-header"])
        .build();
    let content_view = adw::ToolbarView::new();
    content_view.add_top_bar(&content_header);
    content_view.set_content(Some(&stack));
    let content = adw::NavigationPage::builder().title("General").tag("section").child(&content_view).build();

    let split = adw::NavigationSplitView::builder()
        .sidebar(&sidebar)
        .content(&content)
        // The library sidebar's width, at every window size.
        .min_sidebar_width(crate::ui::sidebar::SIDEBAR_WIDTH as f64)
        .max_sidebar_width(crate::ui::sidebar::SIDEBAR_WIDTH as f64)
        .show_content(true)
        .build();
    let view = adw::BreakpointBin::builder()
        .width_request(360)
        .height_request(300)
        .css_classes(["settings-view"])
        .child(&split)
        .build();
    let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(adw::BreakpointConditionLengthType::MaxWidth, 720.0, adw::LengthUnit::Sp));
    narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
    narrow.connect_apply(move |_| widgets::set_narrow(view_id, true));
    narrow.connect_unapply(move |_| widgets::set_narrow(view_id, false));
    view.add_breakpoint(narrow);

    let ctx = {
        let nav_w = nav.downgrade();
        let go: Rc<dyn Fn(&str)> = Rc::new(move |id: &str| {
            if let Some(nav) = nav_w.upgrade() {
                select_section(&nav, id);
            }
        });
        let open = alive.clone();
        let close: Rc<dyn Fn()> = Rc::new(move || {
            if open.replace(false) {
                on_close();
            }
        });
        Ctx::new(window.clone(), alive.clone(), go, close)
    };
    // Timers stop however the page goes (closed, or the library rebuilt).
    view.connect_unrealize({
        let alive = alive.clone();
        move |_| alive.set(false)
    });
    back.connect_clicked({
        let ctx = ctx.clone();
        move |_| ctx.close()
    });
    // Esc steps back: from a section to the list when collapsed, else out.
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let (ctx, split) = (ctx.clone(), split.clone());
        move |_, key, _, _| {
            if key != gtk::gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            if split.is_collapsed() && split.shows_content() {
                split.set_show_content(false);
            } else {
                ctx.close();
            }
            glib::Propagation::Stop
        }
    });
    view.add_controller(keys);

    // Pages are data-driven widgets; some want to know when they come into view.
    let on_shown: Rc<RefCell<Shown>> = Rc::new(RefCell::new(HashMap::new()));
    stack.add_named(&general::build(&ctx), Some("general"));
    stack.add_named(&collections::build(&ctx), Some("collections"));
    stack.add_named(&hidden::build(&ctx), Some("hidden"));
    stack.add_named(&emulators::build(&ctx), Some("emulators"));
    stack.add_named(&appearance::build(&ctx), Some("appearance"));
    {
        let (page, shown) = storage::build(&ctx);
        stack.add_named(&page, Some("storage"));
        on_shown.borrow_mut().insert("storage".into(), shown);
    }
    stack.add_named(&network::build(&ctx), Some("network"));
    stack.add_named(&packs::build(&ctx), Some("packs"));
    stack.add_named(&about::build(&ctx), Some("about"));

    nav.connect_row_selected(glib::clone!(#[weak] stack, #[weak] content, #[strong] on_shown, move |_, row| {
        let Some(row) = row else { return };
        let id = row.widget_name().to_string();
        stack.set_visible_child_name(&id);
        if let Some((_, label, _)) = SECTIONS.iter().find(|(s, _, _)| *s == id) {
            content.set_title(label);
        }
        let cb = on_shown.borrow().get(&id).cloned();
        if let Some(cb) = cb {
            cb();
        }
    }));
    // A tap opens the section, also the one already selected (after going
    // back to the list in a narrow tile).
    nav.connect_row_activated(glib::clone!(#[weak] split, move |_, _| split.set_show_content(true)));

    select_section(&nav, section);
    view.upcast()
}

fn select_section(nav: &gtk::ListBox, id: &str) {
    let mut i = 0;
    while let Some(row) = nav.row_at_index(i) {
        if row.widget_name() == id {
            nav.select_row(Some(&row));
            return;
        }
        i += 1;
    }
    if let Some(first) = nav.row_at_index(0) {
        nav.select_row(Some(&first));
    }
}

// ── Pure logic shared by the pages ──────────────────────────────────────────

/// The stored `collections` key as a list; empty means the base collection.
pub fn parse_collections(raw: Option<&str>) -> Vec<String> {
    let ids: Vec<String> =
        raw.unwrap_or("").split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    if ids.is_empty() {
        vec![BASE_COLLECTION.to_string()]
    } else {
        ids
    }
}

fn display_rank(id: &str) -> usize {
    COLLECTION_ORDER.iter().position(|c| *c == id).unwrap_or(COLLECTION_ORDER.len())
}

/// Display order: the fixed list first, unknown ids after in their given order.
pub fn sort_by_display_order(ids: &mut [String]) {
    ids.sort_by_key(|id| display_rank(id));
}

/// A wanted set made safe: eXoDOS always in, unknown ids dropped once the
/// available list is known, display order.
pub fn normalise_collections(wanted: &[String], known: &[String]) -> Vec<String> {
    let mut set: Vec<String> = wanted.to_vec();
    if !set.iter().any(|s| s == BASE_COLLECTION) {
        set.push(BASE_COLLECTION.to_string());
    }
    let mut out: Vec<String> = if known.is_empty() {
        set
    } else {
        known.iter().filter(|k| set.contains(k)).cloned().collect()
    };
    out.dedup();
    sort_by_display_order(&mut out);
    out
}

/// One line per collection for the Settings hint.
pub fn collection_blurb(id: &str) -> Option<&'static str> {
    Some(match id {
        "eXoDOS" => "DOS games, the base collection - always on.",
        "eXoDOS_GLP" => "German releases of DOS games, with German metadata.",
        "eXoDOS_SLP" => "Spanish releases of DOS games, with Spanish metadata.",
        "eXoDOS_PLP" => "Polish releases of DOS games, with Polish metadata.",
        "eXoWin3x" => "Windows 3.x games, run in DOSBox Staging with a bundled Windows 3.1.",
        "eXoWin9x" => "Windows 9x games, needs DOSBox-X / 86Box (fetched automatically).",
        "eXoScummVM" => "Adventure games run through ScummVM (fetched automatically).",
        _ => return None,
    })
}

/// "1,234 games · blurb".
pub fn collection_hint(id: &str, count: i64) -> String {
    let games = plural(count, "game");
    match collection_blurb(id) {
        Some(b) => format!("{games} · {b}"),
        None => games,
    }
}

/// "1,234".
pub fn group_thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        out.insert(0, '-');
    }
    out
}

/// "1 game", "2 games".
pub fn plural(n: i64, unit: &str) -> String {
    format!("{} {}{}", group_thousands(n), unit, if n == 1 { "" } else { "s" })
}

/// A speed-limit field: a positive whole number of KB/s, capped; anything
/// else (empty, zero, garbage) is unlimited.
pub fn parse_rate_limit(raw: &str) -> Option<u32> {
    let digits: String = raw.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let v: u64 = digits.parse().unwrap_or(u64::MAX);
    if v == 0 {
        None
    } else {
        Some(v.min(MAX_KBPS as u64) as u32)
    }
}

/// Where the emulator comes from, in words.
pub fn dosbox_source_label(source: &str) -> &'static str {
    match source {
        "pack" => "Downloaded by eXorchy",
        "system" => "System package",
        "custom" => "Custom path",
        _ => "Not found",
    }
}

/// A support payload's state (MT-32 ROMs, Win9x images, ScummVM extras).
pub fn support_label(status: Option<(&str, f32, u64)>) -> String {
    let Some((phase, progress, total_bytes)) = status else { return "Checking…".into() };
    match phase {
        "ready" => "Ready".into(),
        "downloading" => format!("Downloading… {}%", (progress * 100.0).round() as i64),
        "failed" => "Download failed - it is retried on the next start".into(),
        _ if total_bytes > 0 => {
            format!("Not installed - fetched with the first game ({})", format_bytes(total_bytes))
        }
        _ => "Not installed - fetched with the first game".into(),
    }
}

/// What a content-pack job's row says.
pub fn job_status_text(phase: &str, progress: f64, error: Option<&str>) -> String {
    if let Some(e) = error {
        if phase != "failed" {
            return e.to_string();
        }
    }
    match phase {
        "starting" => "Starting…".into(),
        "downloading" => format!("Downloading… {}%", (progress * 100.0).round() as i64),
        "verifying" => "Verifying checksum…".into(),
        "extracting" => "Extracting…".into(),
        "installing" => "Installing…".into(),
        "installed" => "Installed!".into(),
        "failed" => format!("Failed: {}", error.unwrap_or("unknown error")),
        other => other.to_string(),
    }
}

/// The emulator packs have their own rows under Emulators; the Content
/// Packs page hides them so nothing is offered twice.
pub fn is_emulator_pack(id: &str) -> bool {
    id == "dosbox-staging" || id == "dosbox-x" || id == "86box" || id.starts_with("scummvm")
}

/// "Played 12/03/2024" from an ISO / SQLite timestamp, else "Never played".
pub fn last_played_label(iso: Option<&str>) -> String {
    let Some(iso) = iso.map(str::trim).filter(|s| !s.is_empty()) else { return "Never played".into() };
    let normalised = if iso.len() >= 19 && iso.as_bytes()[10] == b' ' {
        let mut s = iso.to_string();
        s.replace_range(10..11, "T");
        s
    } else {
        iso.to_string()
    };
    let with_zone = if normalised.len() == 19 { format!("{normalised}Z") } else { normalised };
    match glib::DateTime::from_iso8601(&with_zone, Some(&glib::TimeZone::utc())) {
        Ok(dt) => match dt.to_local().ok().and_then(|l| l.format("%x").ok()) {
            Some(s) => format!("Played {s}"),
            None => "Never played".into(),
        },
        Err(_) => "Never played".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn collections_parse_and_default_to_the_base() {
        assert_eq!(parse_collections(None), s(&["eXoDOS"]));
        assert_eq!(parse_collections(Some("")), s(&["eXoDOS"]));
        assert_eq!(parse_collections(Some(" eXoWin9x, eXoDOS ,")), s(&["eXoWin9x", "eXoDOS"]));
    }

    #[test]
    fn normalise_keeps_the_base_drops_unknown_and_orders() {
        let known = s(&["eXoDOS_GLP", "eXoDOS", "eXoWin9x", "eXoScummVM"]);
        assert_eq!(normalise_collections(&s(&["eXoWin9x", "bogus"]), &known), s(&["eXoDOS", "eXoWin9x"]));
        assert_eq!(normalise_collections(&s(&["eXoScummVM", "eXoDOS_GLP"]), &known), s(&["eXoDOS", "eXoDOS_GLP", "eXoScummVM"]));
        // Nothing known yet: keep what was asked for, base first.
        assert_eq!(normalise_collections(&s(&["zzz"]), &[]), s(&["eXoDOS", "zzz"]));
    }

    #[test]
    fn rate_limits_parse_like_the_web_field() {
        assert_eq!(parse_rate_limit(""), None);
        assert_eq!(parse_rate_limit("0"), None);
        assert_eq!(parse_rate_limit("abc"), None);
        assert_eq!(parse_rate_limit(" 500 "), Some(500));
        assert_eq!(parse_rate_limit("500kb"), Some(500));
        assert_eq!(parse_rate_limit("99999999999"), Some(MAX_KBPS));
    }

    #[test]
    fn numbers_and_plurals_read_well() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1000), "1,000");
        assert_eq!(group_thousands(7_654_321), "7,654,321");
        assert_eq!(plural(1, "game"), "1 game");
        assert_eq!(plural(2500, "archive"), "2,500 archives");
        assert_eq!(collection_hint("eXoWin9x", 1200), "1,200 games · Windows 9x games, needs DOSBox-X / 86Box (fetched automatically).");
        assert_eq!(collection_hint("other", 1), "1 game");
    }

    #[test]
    fn status_labels_match_the_web_dialog() {
        assert_eq!(support_label(None), "Checking…");
        assert_eq!(support_label(Some(("ready", 1.0, 0))), "Ready");
        assert_eq!(support_label(Some(("downloading", 0.256, 0))), "Downloading… 26%");
        assert_eq!(support_label(Some(("missing", 0.0, 29 * 1024 * 1024))), "Not installed - fetched with the first game (29.0 MB)");
        assert_eq!(dosbox_source_label("pack"), "Downloaded by eXorchy");
        assert_eq!(dosbox_source_label("nope"), "Not found");
        assert_eq!(job_status_text("downloading", 0.5, None), "Downloading… 50%");
        assert_eq!(job_status_text("failed", 0.0, Some("boom")), "Failed: boom");
        assert_eq!(job_status_text("extracting", 0.0, Some("disk full")), "disk full");
        assert!(is_emulator_pack("scummvm-2.8.0"));
        assert!(!is_emulator_pack("posters"));
    }

    #[test]
    fn last_played_handles_sqlite_and_iso_stamps() {
        assert_eq!(last_played_label(None), "Never played");
        assert_eq!(last_played_label(Some("garbage")), "Never played");
        assert!(last_played_label(Some("2024-05-01 12:00:00")).starts_with("Played "));
        assert!(last_played_label(Some("2024-05-01T12:00:00Z")).starts_with("Played "));
    }
}

/// Visual harness: `EXORCHY_SETTINGS_SNAPSHOT_DIR=<dir> cargo test -p exorchy
/// -- --ignored settings_snapshot` renders every section of the Settings page
/// in a plain window into `<dir>/settings-<section>.png`, using the profile the
/// XDG variables point at. Needs a display.
#[cfg(test)]
mod settings_snapshot {
    use super::*;
    use std::time::{Duration, Instant};

    fn pump(ms: u64) {
        let ctx = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < deadline {
            while ctx.pending() {
                ctx.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn render(window: &gtk::Widget, path: &str) -> Result<(), String> {
        // Layout runs on the frame clock, which a compositor only ticks for
        // a visible surface; an obscured test window is laid out by hand.
        let (iw, ih) = if window.width() > 0 { (window.width(), window.height()) } else { (1280, 800) };
        window.allocate(iw, ih, -1, None);
        let (w, h) = (iw as f64, ih as f64);
        // Painting needs frame callbacks too (a sleeping monitor sends
        // none): snapshot the window's children directly, and only fall
        // back to the paintable's last rendered image.
        // The floating sheet reveals its content through an animation that
        // also waits for the frame clock: put it in its open state by hand.
        fn open_sheets(w: &gtk::Widget) {
            if w.type_().name() == "AdwFloatingSheet" {
                let mut c = w.first_child();
                while let Some(ch) = c {
                    ch.set_child_visible(true);
                    ch.set_opacity(1.0);
                    c = ch.next_sibling();
                }
            }
            let mut c = w.first_child();
            while let Some(ch) = c {
                open_sheets(&ch);
                c = ch.next_sibling();
            }
        }
        open_sheets(window);
        window.allocate(iw, ih, -1, None);
        let snapshot = gtk::Snapshot::new();
        snapshot.append_color(&gtk::gdk::RGBA::BLACK, &gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32));
        let mut child = window.first_child();
        while let Some(c) = child {
            c.allocate(iw, ih, -1, None);
            window.snapshot_child(&c, &snapshot);
            child = c.next_sibling();
        }
        let node = match snapshot.to_node() {
            Some(n) => n,
            None => {
                let paintable = gtk::WidgetPaintable::new(Some(window));
                let snapshot = gtk::Snapshot::new();
                paintable.snapshot(&snapshot, w, h);
                snapshot.to_node().ok_or("nothing to render")?
            }
        };
        let renderer = window.native().and_then(|n| n.renderer()).ok_or("window not realized")?;
        renderer.render_texture(&node, None).save_to_png(path).map_err(|e| e.to_string())
    }

    #[test]
    #[ignore]
    fn settings_snapshot() {
        let Ok(out) = std::env::var("EXORCHY_SETTINGS_SNAPSHOT_DIR") else { return };
        let boot = exorchy_core::bootstrap();
        crate::app::install(boot.app.clone());
        adw::init().expect("a display");
        // Presentation animations run on the frame clock, which a sleeping
        // monitor never ticks: the dialog has to land in its final state at once.
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_enable_animations(false);
        }
        crate::theme::init();
        let window = adw::Window::builder().default_width(1280).default_height(800).css_classes(["exorchy"]).build();
        window.set_content(Some(&gtk::Label::new(Some("eXorchy"))));
        window.present();
        pump(800);
        let sections: Vec<String> = match std::env::var("EXORCHY_SNAPSHOT_SETTINGS") {
            Ok(s) if !s.is_empty() => s.split(',').map(String::from).collect(),
            _ => SECTIONS.iter().map(|(id, _, _)| id.to_string()).collect(),
        };
        // The compositor maps the window on its own time.
        for _ in 0..20 {
            if window.is_mapped() && window.width() > 0 {
                break;
            }
            pump(250);
        }
        for section in sections {
            let view = build(window.upcast_ref(), &section, Rc::new(|| {}));
            window.set_content(Some(&view));
            pump(3000);
            let path = format!("{out}/settings-{section}.png");
            let mut result = render(window.upcast_ref(), &path);
            for _ in 0..4 {
                if result.is_ok() {
                    break;
                }
                pump(1000);
                result = render(window.upcast_ref(), &path);
            }
            match result {
                Ok(()) => println!("wrote {path}"),
                Err(e) => println!("snapshot {section} failed: {e} (mapped={}, view mapped={})", window.is_mapped(), view.is_mapped()),
            }
        }
    }
}
