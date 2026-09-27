//! The application window: a stack of setup ↔ library under a splash
//! overlay, wrapped in a toast overlay. No header bar: Hyprland draws no
//! decorations and tiles the window; the pages carry their own toolbars.

use gtk::glib;
use std::rc::Rc;

use exorchy_core::commands::{emulator_cmds, games, library, setup};
use adw::prelude::*;

use crate::app;
use crate::ui::library::LibraryPage;
use crate::ui::util::format_bytes;
use crate::ui::{bus, covers, dialogs, downloads, setup::SetupPage, splash::Splash};

const ICON_SVG: &[u8] = include_bytes!("../../assets/exorchy.svg");

/// The app icon as a paintable (the packaged icon theme has it too, but a
/// source checkout does not).
pub fn app_icon() -> gtk::gdk::Texture {
    gtk::gdk::Texture::from_bytes(&glib::Bytes::from_static(ICON_SVG)).expect("bundled icon")
}

pub fn build(application: &adw::Application, startup_error: Option<String>) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("eXorchy")
        .default_width(1280)
        .default_height(800)
        .width_request(900)
        .height_request(600)
        .decorated(false)
        .css_classes(["exorchy"])
        .build();

    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
    let splash = Rc::new(Splash::new());
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&stack));
    overlay.add_overlay(&splash.widget);
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&overlay));
    bus::set_toast_overlay(&toasts);
    SHELL.with(|s| *s.borrow_mut() = Some((window.clone(), stack.clone(), toasts.clone())));
    window.set_content(Some(&toasts));

    if let Some(msg) = startup_error {
        let win = window.clone();
        dialogs::error(&win, "eXorchy failed to start", &format!("{msg}\n\nSee the log folder for details."));
        splash.release();
        return window;
    }

    // Decide setup vs library once the backend answers.
    let core = app::core();
    let win = window.clone();
    app::spawn(
        async move { setup::get_setup_status(core.state(), core.state()).await },
        glib::clone!(#[weak] stack, #[strong] splash, move |status| {
            let ready = status.map(|s| s.ready).unwrap_or(false);
            if ready {
                show_library(&win, &stack, &toasts);
            } else {
                show_setup(&win, &stack, &toasts);
            }
            splash.release();
        }),
    );

    window
}

fn show_setup(window: &adw::ApplicationWindow, stack: &gtk::Stack, toasts: &adw::ToastOverlay) {
    let done: Rc<dyn Fn()> = {
        let (window, stack, toasts) = (window.clone(), stack.clone(), toasts.clone());
        Rc::new(move || show_library(&window, &stack, &toasts))
    };
    let page = SetupPage::new(window.upcast_ref(), done);
    if let Some(old) = stack.child_by_name("setup") {
        stack.remove(&old);
    }
    stack.add_named(&page.widget, Some("setup"));
    stack.set_visible_child_name("setup");
}

/// The library, with the web shell's startup order: listeners first, then
/// network mode, cover dirs, the download managers, packs, transfer polling,
/// the emulator, the install scan, then the first fetch and resumed downloads.
fn show_library(window: &adw::ApplicationWindow, stack: &gtk::Stack, _toasts: &adw::ToastOverlay) {
    if let Some(old) = stack.child_by_name("library") {
        stack.remove(&old);
    }
    let page = LibraryPage::new(window.upcast_ref());
    stack.add_named(&page.widget, Some("library"));
    stack.set_visible_child_name("library");
    // The page lives as long as the window.
    LIBRARY.with(|l| *l.borrow_mut() = Some(page.clone()));
    let w = window.clone();
    page.settings_button.connect_clicked(move |_| crate::ui::settings::open(&w, "general"));
    page.set_reading_widget(&crate::ui::reading::build(window.upcast_ref()));
    crate::ui::media::install(window.upcast_ref(), &page, &page.bar_slot);

    downloads::init_dependency_downloads();
    app::on_event("game-exited", |payload| {
        if let Some(id) = payload.get("id").and_then(|v| v.as_i64()) {
            bus::mark_running(id, false);
        }
    });
    let activity = page.activity.clone();
    downloads::start_transfer_polling(move |s| {
        activity.set_label(&if !s.active {
            String::new()
        } else if s.download_bps < 1024 && s.upload_bps < 1024 {
            format!("{} peers", s.peers)
        } else {
            format!("↓ {}/s  ↑ {}/s", format_bytes(s.download_bps), format_bytes(s.upload_bps))
        });
    });

    let core = app::core();
    let p = page.clone();
    let window = window.clone();
    app::local(async move {
        let c = core.clone();
        let mode = app::call(async move { games::get_config(c.state(), "network_mode".into()).await }).await.ok().flatten();
        bus::set_offline(mode.as_deref() == Some("offline"));
        covers::load_dirs();
        let c = core.clone();
        if let Err(e) = app::call(async move { setup::init_download_manager(c.state(), c.state(), c.state()).await }).await {
            log::error!("init_download_manager: {e}");
        }
        let running = app::call(async move { games::running_game_ids().await }).await.unwrap_or_default();
        bus::set_running(running.into_iter().collect());
        let c = core.clone();
        app::spawn(async move { emulator_cmds::ensure_dosbox_staging(c.clone(), c.state()).await }, |res| {
            if let Ok(s) = res {
                if s.path.is_none() && !bus::offline() {
                    bus::toast_with("Downloading DOSBox Staging", Some("The emulator is fetched once (29 MB). Games can be downloaded meanwhile."), None);
                }
            }
        });
        let c = core.clone();
        let _ = app::call(async move { library::scan_installed_games(c.state(), c.state(), None).await }).await;
        p.load_collections();
        downloads::resume_all();
        crate::ui::onboarding::run(&window);
    });
}

thread_local! {
    static LIBRARY: std::cell::RefCell<Option<Rc<LibraryPage>>> = const { std::cell::RefCell::new(None) };
    static SHELL: std::cell::RefCell<Option<(adw::ApplicationWindow, gtk::Stack, adw::ToastOverlay)>> = const { std::cell::RefCell::new(None) };
}

/// After a factory reset: drop the library and show setup again.
pub fn restart_to_setup() {
    let shell = SHELL.with(|s| s.borrow().clone());
    let Some((window, stack, toasts)) = shell else { return };
    downloads::stop_all();
    LIBRARY.with(|l| *l.borrow_mut() = None);
    if let Some(old) = stack.child_by_name("library") {
        stack.remove(&old);
    }
    show_setup(&window, &stack, &toasts);
}

/// Re-run the post-setup startup (data dir changed, collections changed):
/// the download managers, cover dirs, install scan and the first fetch.
pub fn reinit_library() {
    let shell = SHELL.with(|s| s.borrow().clone());
    let Some((window, stack, toasts)) = shell else { return };
    show_library(&window, &stack, &toasts);
}

/// The library page, once shown.
pub fn library() -> Option<Rc<LibraryPage>> {
    LIBRARY.with(|l| l.borrow().clone())
}
