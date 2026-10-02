//! The splash: a small floating window centred on the screen while the app
//! starts, then the app's own window, which Hyprland tiles. Built, not a
//! picture: the computer graphic (`assets/splash_computer.png`, its accents
//! hue-shifted to the theme in `theme.rs`), the pixel wordmark
//! (`logo::ascii`), "THE EXODOS LAUNCHER FOR OMARCHY" and the slogan, all in
//! the Omarchy theme's colours (`styles/splash.css`).
//!
//! On Hyprland a runtime window rule (`hyprctl eval`, matched on the splash's
//! title) floats and centres it; the user's config is not touched. Elsewhere
//! it is an ordinary small window. Snapshot runs skip it and show the app at
//! once; `EXORCHY_SNAPSHOT_SPLASH=<png>` captures the splash instead.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;

const COMPUTER: &[u8] = include_bytes!("../../assets/splash_computer.png");
/// The splash stays at least this long, so it reads as a splash, not a flicker.
const MIN_MS: u64 = 5000;
/// The fade before the splash window closes.
const FADE_MS: u32 = 350;
/// The title the Hyprland rule matches.
const TITLE: &str = "eXorchy splash";

pub struct Splash {
    window: Option<gtk::Window>,
    content: gtk::Revealer,
    shown_at: Instant,
    released: Cell<bool>,
    main: RefCell<Option<gtk::Window>>,
}

/// The splash's size: the key art's 3:2, at the interface scale.
fn size() -> (i32, i32) {
    (crate::theme::scaled(600), crate::theme::scaled(400))
}

/// The splash's content, on its own (the snapshot captures it too).
fn content() -> gtk::Box {
    let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).halign(gtk::Align::Center).valign(gtk::Align::Center).css_classes(["splash-body"]).build();

    // The computer stands on the wordmark, right of centre, as in the key art.
    let (w, _) = size();
    let art_w = (w as f64 * 0.366).round() as i32;
    let art_h = (art_w as f64 * 338.0 / 563.0).round() as i32;
    // Scaled to its place here: a picture's natural size is its texture's,
    // and the full 563 px art grew the window. Twice over for HiDPI.
    let computer = match art_texture(art_w * 2, art_h * 2) {
        Some(t) => gtk::Picture::for_paintable(&t),
        None => gtk::Picture::new(),
    };
    computer.set_content_fit(gtk::ContentFit::Contain);
    computer.set_can_shrink(true);
    computer.set_size_request(art_w, art_h);
    computer.set_hexpand(false);
    computer.set_halign(gtk::Align::Center);
    computer.set_margin_start((w as f64 * 0.12).round() as i32);
    computer.add_css_class("splash-art");
    root.append(&computer);

    let logo = crate::ui::logo::ascii(5.0 * crate::theme::ui_scale());
    logo.set_halign(gtk::Align::Center);
    logo.add_css_class("splash-logo");
    root.append(&logo);

    root.append(&gtk::Label::builder().label("THE EXODOS LAUNCHER FOR OMARCHY").css_classes(["splash-subtitle"]).build());
    root.append(&gtk::Label::builder().label("thousands of games. endless nostalgia.").css_classes(["splash-slogan"]).build());
    root
}

/// The computer graphic at `w`×`h` px.
fn art_texture(w: i32, h: i32) -> Option<gtk::gdk::Texture> {
    let img = image::load_from_memory(COMPUTER).map_err(|e| log::warn!("splash art unreadable: {e}")).ok()?;
    let rgba = img.resize_exact(w.max(1) as u32, h.max(1) as u32, image::imageops::FilterType::Triangle).to_rgba8();
    let (w, h) = rgba.dimensions();
    let bytes = glib::Bytes::from_owned(rgba.into_raw());
    Some(gtk::gdk::MemoryTexture::new(w as i32, h as i32, gtk::gdk::MemoryFormat::R8g8b8a8, &bytes, (w * 4) as usize).upcast())
}

impl Splash {
    /// Show the splash (unless a snapshot run wants the app at once).
    pub fn new(application: &adw::Application) -> Rc<Self> {
        let content = gtk::Revealer::builder().transition_type(gtk::RevealerTransitionType::Crossfade).transition_duration(FADE_MS).reveal_child(true).child(&content()).build();
        let snapshot_app = std::env::var_os("EXORCHY_SNAPSHOT").is_some();
        let window = (!snapshot_app || std::env::var_os("EXORCHY_SNAPSHOT_SPLASH").is_some()).then(|| {
            let (w, h) = size();
            float_on_hyprland();
            let window = gtk::Window::builder()
                .application(application)
                .title(TITLE)
                .default_width(w)
                .default_height(h)
                .resizable(false)
                .decorated(false)
                .css_classes(["splash-window"])
                .child(&content)
                .build();
            window.present();
            window
        });
        let splash = Rc::new(Splash { window, content, shown_at: Instant::now(), released: Cell::new(false), main: RefCell::new(None) });
        if let Some(path) = std::env::var("EXORCHY_SNAPSHOT_SPLASH").ok().filter(|p| !p.is_empty()) {
            let s = splash.clone();
            glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                if let Some(w) = &s.window {
                    match crate::snapshot::render(w.upcast_ref(), &path) {
                        Ok(()) => log::info!("Snapshot written to {path}"),
                        Err(e) => log::error!("Snapshot failed: {e}"),
                    }
                }
                if let Some(app) = s.window.as_ref().and_then(|w| w.application()) {
                    app.quit();
                }
            });
        }
        splash
    }

    /// The app is ready: once the minimum time has passed, the app's window
    /// appears (Hyprland tiles it) and the splash fades and closes.
    pub fn release(self: &Rc<Self>, main: &impl IsA<gtk::Window>) {
        if self.released.replace(true) {
            return;
        }
        self.main.replace(Some(main.clone().upcast()));
        let Some(window) = self.window.clone() else {
            main.present();
            return;
        };
        if std::env::var_os("EXORCHY_SNAPSHOT_SPLASH").is_some() {
            return;
        }
        let wait = Duration::from_millis(MIN_MS).saturating_sub(self.shown_at.elapsed());
        let splash = self.clone();
        glib::timeout_add_local_once(wait, move || {
            if let Some(m) = splash.main.borrow().as_ref() {
                m.present();
            }
            splash.content.set_reveal_child(false);
            glib::timeout_add_local_once(Duration::from_millis(FADE_MS as u64), move || window.close());
        });
    }
}

/// Ask Hyprland to float and centre the splash. A runtime rule, matched on
/// the splash's title; nothing is written to the user's config.
fn float_on_hyprland() {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").filter(|v| !v.is_empty()).is_none() {
        return;
    }
    let rule = format!("hl.window_rule({{ match = {{ title = \"^({TITLE})$\" }}, float = true, center = true }})");
    let ok = std::process::Command::new("hyprctl")
        .args(["eval", &rule])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "ok")
        .unwrap_or(false);
    if !ok {
        log::warn!("Hyprland did not accept the splash's float rule; it will tile");
    }
}
