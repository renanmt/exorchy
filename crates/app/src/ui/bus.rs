//! Cross-page state that is not a widget's own: the library-change bus, the
//! running set, the offline flag and toasts. Main-thread only.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

type Cb<T> = Rc<dyn Fn(&T)>;
/// What a toast's button runs.
pub type ToastAction = Rc<dyn Fn()>;

struct Bus {
    library_changed: Vec<Cb<i64>>,
    running_changed: Vec<Cb<HashSet<i64>>>,
    running: HashSet<i64>,
    offline: bool,
    toasts: Option<glib::WeakRef<adw::ToastOverlay>>,
    /// The enabled collections, display order, kept by the settings page.
    collections_changed: Vec<Cb<()>>,
    playlists_changed: Vec<Cb<()>>,
    visibility_changed: Vec<Cb<()>>,
}

thread_local! {
    static BUS: RefCell<Bus> = RefCell::new(Bus {
        library_changed: Vec::new(),
        running_changed: Vec::new(),
        running: HashSet::new(),
        offline: false,
        toasts: None,
        collections_changed: Vec::new(),
        playlists_changed: Vec::new(),
        visibility_changed: Vec::new(),
    });
}

/// A game's installed / in-library state changed (download, uninstall,
/// reset, import). Shelves and the detail panel re-read their rows.
pub fn on_library_changed(f: impl Fn(&i64) + 'static) {
    BUS.with(|b| b.borrow_mut().library_changed.push(Rc::new(f)));
}

pub fn notify_library_changed(id: i64) {
    let cbs: Vec<Cb<i64>> = BUS.with(|b| b.borrow().library_changed.clone());
    for cb in cbs {
        cb(&id);
    }
}

pub fn on_collections_changed(f: impl Fn(&()) + 'static) {
    BUS.with(|b| b.borrow_mut().collections_changed.push(Rc::new(f)));
}

pub fn notify_collections_changed() {
    let cbs: Vec<Cb<()>> = BUS.with(|b| b.borrow().collections_changed.clone());
    for cb in cbs {
        cb(&());
    }
}

/// What the catalogue lists changed: a title hidden or unhidden, adult
/// titles switched, a game taken off Recently played.
pub fn on_visibility_changed(f: impl Fn(&()) + 'static) {
    BUS.with(|b| b.borrow_mut().visibility_changed.push(Rc::new(f)));
}

pub fn notify_visibility_changed() {
    let cbs: Vec<Cb<()>> = BUS.with(|b| b.borrow().visibility_changed.clone());
    for cb in cbs {
        cb(&());
    }
}

/// A playlist was created, renamed, deleted or had its membership changed.
pub fn on_playlists_changed(f: impl Fn(&()) + 'static) {
    BUS.with(|b| b.borrow_mut().playlists_changed.push(Rc::new(f)));
}

pub fn notify_playlists_changed() {
    let cbs: Vec<Cb<()>> = BUS.with(|b| b.borrow().playlists_changed.clone());
    for cb in cbs {
        cb(&());
    }
}

/// Games whose emulator is running.
pub fn running() -> HashSet<i64> {
    BUS.with(|b| b.borrow().running.clone())
}

pub fn is_running(id: i64) -> bool {
    BUS.with(|b| b.borrow().running.contains(&id))
}

pub fn set_running(ids: HashSet<i64>) {
    BUS.with(|b| b.borrow_mut().running = ids.clone());
    let cbs: Vec<Cb<HashSet<i64>>> = BUS.with(|b| b.borrow().running_changed.clone());
    for cb in cbs {
        cb(&ids);
    }
}

pub fn mark_running(id: i64, running: bool) {
    let mut set = self::running();
    if running {
        set.insert(id);
    } else {
        set.remove(&id);
    }
    set_running(set);
}

pub fn on_running_changed(f: impl Fn(&HashSet<i64>) + 'static) {
    BUS.with(|b| b.borrow_mut().running_changed.push(Rc::new(f)));
}

/// Offline mode: no torrent session, so no downloads are offered.
pub fn offline() -> bool {
    BUS.with(|b| b.borrow().offline)
}

pub fn set_offline(v: bool) {
    BUS.with(|b| b.borrow_mut().offline = v);
}

pub fn set_toast_overlay(overlay: &adw::ToastOverlay) {
    BUS.with(|b| b.borrow_mut().toasts = Some(overlay.downgrade()));
}

/// A transient message at the bottom of the window.
pub fn toast(msg: &str) {
    toast_with(msg, None, None);
}

/// A toast with an optional detail line and an optional action button.
pub fn toast_with(msg: &str, detail: Option<&str>, action: Option<(&str, ToastAction)>) {
    let overlay = BUS.with(|b| b.borrow().toasts.as_ref().and_then(|w| w.upgrade()));
    let Some(overlay) = overlay else {
        log::info!("toast (no overlay yet): {msg}");
        return;
    };
    let text = match detail {
        Some(d) if !d.is_empty() => format!("{msg}\n{d}"),
        _ => msg.to_string(),
    };
    let toast = adw::Toast::new(&text);
    toast.set_timeout(if detail.is_some() { 8 } else { 5 });
    if let Some((label, f)) = action {
        toast.set_button_label(Some(label));
        toast.connect_button_clicked(move |_| f());
    }
    overlay.add_toast(toast);
}
