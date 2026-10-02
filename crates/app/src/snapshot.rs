//! Developer aid: `EXORCHY_SNAPSHOT=<file.png>[:<delay ms>]` renders the
//! window to a PNG once it has settled and quits. It replaces the web UI's
//! headless smoke script: a session can check what the app looks like on
//! this machine's theme without a compositor screenshot.

use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

/// Arm the timer; default delay 3 s.
pub fn arm(window: &adw::ApplicationWindow) {
    let Ok(spec) = std::env::var("EXORCHY_SNAPSHOT") else { return };
    let (path, delay_ms): (String, u64) = match spec.rsplit_once(':') {
        Some((p, ms)) if !ms.is_empty() && ms.chars().all(|c| c.is_ascii_digit()) => {
            (p.to_string(), ms.parse().unwrap_or(3000))
        }
        _ => (spec.clone(), 3000),
    };
    let window = window.clone();
    // EXORCHY_SNAPSHOT_SIZE=<w>x<h> sizes the window first (tile simulation).
    if let Some((w, h)) = std::env::var("EXORCHY_SNAPSHOT_SIZE").ok().and_then(|v| {
        let (w, h) = v.split_once('x')?;
        Some((w.parse::<i32>().ok()?, h.parse::<i32>().ok()?))
    }) {
        window.set_default_size(w, h);
    }
    // EXORCHY_SNAPSHOT_TAB=<browse|library|reading> switches the tab first.
    if let Ok(tab) = std::env::var("EXORCHY_SNAPSHOT_TAB") {
        glib::timeout_add_local_once(Duration::from_millis(delay_ms.saturating_sub(2500)), move || {
            if let Some(lib) = crate::ui::window::library() {
                lib.set_tab(&tab);
            }
        });
    }
    // EXORCHY_SNAPSHOT_GAME=<id> opens that game's detail panel first.
    if let Some(id) = std::env::var("EXORCHY_SNAPSHOT_GAME").ok().and_then(|v| v.parse::<i64>().ok()) {
        glib::timeout_add_local_once(Duration::from_millis(delay_ms.saturating_sub(1500)), move || {
            let core = crate::app::core();
            crate::app::spawn(async move { exorchy_core::commands::games::get_game(core.state(), id).await }, move |res| {
                if let (Ok(Some(g)), Some(lib)) = (res, crate::ui::window::library()) {
                    lib.detail().show(g);
                }
            });
        });
    }
    // EXORCHY_SNAPSHOT_SEQUENCE=<step,step,...> drives the app before the
    // shot, one step every 250 ms: the open panel (`uninstall`, `close`,
    // `run`, `exit`, `reopen`), `click:<label>` (the first button whose text
    // is <label>) and `activate:<label>` (the first list row showing <label>).
    if let Ok(seq) = std::env::var("EXORCHY_SNAPSHOT_SEQUENCE") {
        let steps: Vec<String> = seq.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        let base = delay_ms.saturating_sub(1500) + 400;
        for (i, step) in steps.into_iter().enumerate() {
            glib::timeout_add_local_once(Duration::from_millis(base + 250 * i as u64), move || {
                let Some(lib) = crate::ui::window::library() else { return };
                let panel = lib.detail();
                let game = panel.selected_game();
                log::info!("sequence step {step}: open={} game={:?}", panel.is_open(), game.as_ref().and_then(|g| g.id));
                if let Some(label) = step.strip_prefix("click:") {
                    let root = lib.widget.clone();
                    let hit = find(&root, &|w| w.is::<gtk::Button>() && find(w, &|l| l.downcast_ref::<gtk::Label>().is_some_and(|l| l.label() == label)).is_some());
                    match hit.and_downcast::<gtk::Button>() {
                        Some(b) => b.emit_clicked(),
                        None => log::warn!("snapshot: no button \"{label}\""),
                    }
                    return;
                }
                if let Some(label) = step.strip_prefix("activate:") {
                    let root = lib.widget.clone();
                    let mut done = false;
                    let mut stack = vec![root];
                    while let Some(w) = stack.pop() {
                        if let Some(lv) = w.downcast_ref::<gtk::ListView>() {
                            if let Some(model) = lv.model() {
                                for i in 0..model.n_items() {
                                    if model.item(i).and_downcast::<gtk::StringObject>().is_some_and(|o| o.string() == label) {
                                        lv.emit_by_name::<()>("activate", &[&i]);
                                        done = true;
                                        break;
                                    }
                                }
                            }
                        }
                        let mut c = w.first_child();
                        while let Some(ch) = c {
                            c = ch.next_sibling();
                            stack.push(ch);
                        }
                    }
                    if !done {
                        log::warn!("snapshot: no row \"{label}\"");
                    }
                    return;
                }
                match step.as_str() {
                    "close" => panel.close(),
                    "uninstall" => {
                        if let Some(g) = game {
                            crate::ui::actions::uninstall(g.id.unwrap_or(0), g.title.clone(), std::rc::Rc::new(|_| {}), std::rc::Rc::new(|_| {}));
                        }
                    }
                    "run" => {
                        if let Some(id) = game.and_then(|g| g.id) {
                            crate::ui::bus::mark_running(id, true);
                        }
                    }
                    "exit" => {
                        if let Some(id) = game.and_then(|g| g.id) {
                            crate::ui::bus::mark_running(id, false);
                        }
                    }
                    "reopen" => {
                        if let Some(g) = game {
                            panel.show(g);
                        }
                    }
                    // The Reading Room's full screen (the reader's button / F11).
                    "fullscreen" => {
                        if let Some(lib) = crate::ui::window::library() {
                            lib.set_reading_fullscreen(true);
                        }
                    }
                    "wait" => {}
                    _ => log::warn!("unknown sequence step {step}"),
                }
            });
        }
    }
    // EXORCHY_SNAPSHOT_SCROLL=<css class>:<px> scrolls the first scrolled
    // window inside the first widget with that class (Broadway's screen is
    // 1024x768, so a tall panel is checked by scrolling, not by growing).
    if let Some((class, px)) = std::env::var("EXORCHY_SNAPSHOT_SCROLL").ok().and_then(|v| {
        let (c, p) = v.rsplit_once(':')?;
        Some((c.to_string(), p.parse::<f64>().ok()?))
    }) {
        let w = window.clone();
        glib::timeout_add_local_once(Duration::from_millis(delay_ms.saturating_sub(1000)), move || {
            let target = find(w.upcast_ref(), &|x| x.has_css_class(&class)).and_then(|host| find(&host, &|x| x.is::<gtk::ScrolledWindow>()));
            match target.and_downcast::<gtk::ScrolledWindow>() {
                Some(sw) => sw.vadjustment().set_value(px),
                None => log::warn!("snapshot: nothing scrollable inside .{class}"),
            }
        });
    }
    glib::timeout_add_local_once(Duration::from_millis(delay_ms), move || {
        if std::env::var_os("EXORCHY_DUMP_TREE").is_some() {
            dump(window.upcast_ref::<gtk::Widget>(), 0);
        }
        match render(&window, &path) {
            Ok(()) => log::info!("Snapshot written to {path}"),
            Err(e) => log::error!("Snapshot failed: {e}"),
        }
        if let Some(app) = window.application() {
            app.quit();
        }
    });
}

/// Depth-first search for the first descendant (or `root` itself) matching `pred`.
fn find(root: &gtk::Widget, pred: &dyn Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if pred(root) {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(c) = child {
        if let Some(hit) = find(&c, pred) {
            return Some(hit);
        }
        child = c.next_sibling();
    }
    None
}

fn render(window: &adw::ApplicationWindow, path: &str) -> Result<(), String> {
    let widget: gtk::Widget = window.clone().upcast();
    let (w, h) = (widget.width() as f64, widget.height() as f64);
    if w < 1.0 || h < 1.0 {
        return Err("window has no size yet".into());
    }
    let paintable = gtk::WidgetPaintable::new(Some(&widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, w, h);
    let node = snapshot.to_node().ok_or("nothing to render")?;
    let renderer = widget.native().and_then(|n| n.renderer()).ok_or("window not realized")?;
    let texture = renderer.render_texture(&node, None);
    texture.save_to_png(path).map_err(|e| e.to_string())
}

/// Log the widget tree with allocations and natural sizes (EXORCHY_DUMP_TREE).
fn dump(w: &gtk::Widget, depth: usize) {
    let (_, nat_h, _, _) = w.measure(gtk::Orientation::Vertical, -1);
    let (_, nat_w, _, _) = w.measure(gtk::Orientation::Horizontal, -1);
    if w.type_().name() == "AdwOverlaySplitView" {
        log::info!("split collapsed={} show_sidebar={}", w.property::<bool>("collapsed"), w.property::<bool>("show-sidebar"));
    }
    if w.type_().name() == "AdwBreakpointBin" {
        log::info!("breakpoint bin current={:?}", w.property::<Option<adw::Breakpoint>>("current-breakpoint").map(|b| b.condition().map(|c| c.to_str().to_string())));
    }
    log::info!(
        "{}{} css={:?} alloc={}x{} nat={}x{} visible={}",
        "  ".repeat(depth),
        w.type_().name(),
        w.css_classes().iter().map(|c| c.to_string()).collect::<Vec<_>>().join(","),
        w.width(),
        w.height(),
        nat_w,
        nat_h,
        w.is_visible()
    );
    if depth > 22 {
        return;
    }
    let mut c = w.first_child();
    let mut n = 0;
    let limit = if w.type_().name() == "GtkGridView" { 40 } else { 3 };
    while let Some(child) = c {
        if w.type_().name() == "GtkGridView" {
            let b = child.compute_bounds(w).map(|b| (b.x() as i32, b.y() as i32, b.width() as i32, b.height() as i32));
            log::info!("{}cell bounds={:?} nat={:?} h@203={:?} h@172={:?}", "  ".repeat(depth + 1), b, child.measure(gtk::Orientation::Vertical, -1).1, child.measure(gtk::Orientation::Vertical, 203), child.measure(gtk::Orientation::Vertical, 172));
            if n == 0 {
                dump_hfw(&child, depth + 2);
            }
        } else {
            dump(&child, depth + 1);
        }
        n += 1;
        if n > limit {
            break;
        }
        c = child.next_sibling();
    }
}

fn dump_hfw(w: &gtk::Widget, depth: usize) {
    let width = w.width().max(1);
    let m = w.measure(gtk::Orientation::Vertical, width);
    log::info!("{}{} css={:?} w={} min_h={} nat_h={} visible={}", "  ".repeat(depth), w.type_().name(), w.css_classes().iter().map(|c| c.to_string()).collect::<Vec<_>>().join(","), width, m.0, m.1, w.is_visible());
    let mut c = w.first_child();
    while let Some(child) = c {
        dump_hfw(&child, depth + 1);
        c = child.next_sibling();
    }
}
