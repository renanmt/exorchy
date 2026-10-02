//! Art at full resolution: a game's cover (the metadata pack's own
//! "Box - Front" scan when installed, else the poster) or a gallery image,
//! in a dialog centred over the window. It opens at the image's own size,
//! never larger than the window; in a small tile it takes the whole body.
//! Click the image or press Esc to close.

use std::path::{Path, PathBuf};

use adw::prelude::*;
use exorchy_core::host::async_runtime;
use gtk::glib;

use crate::app;
use crate::ui::covers;

/// Room left around the dialog inside the window, per side.
const MARGIN: i32 = 24;

/// The best full-size cover there is: the pack's box scan, else the
/// poster-pack file, else the bundled preview.
pub fn best_cover(gallery: &[String], source: Option<&str>, key: Option<&str>) -> Option<PathBuf> {
    gallery
        .iter()
        .find(|p| p.contains("/Box - Front/"))
        .map(PathBuf::from)
        .or_else(|| covers::candidates(source, key).into_iter().find(|p| p.is_file()))
}

/// The dialog's content size for an image of `w`×`h` in a window of
/// `win_w`×`win_h`: the image's own size, scaled down to fit, keeping its
/// shape. Art shorter than `min_h` (a 120 px preview) is enlarged to it,
/// window permitting. `header` is the title bar's height.
pub fn fit(w: i32, h: i32, win_w: i32, win_h: i32, header: i32, min_h: i32) -> (i32, i32) {
    let max_w = (win_w - 2 * MARGIN).max(100);
    let max_h = (win_h - 2 * MARGIN - header).max(100);
    if w <= 0 || h <= 0 {
        return (max_w, max_h + header);
    }
    let grow = (min_h as f64 / h as f64).max(1.0);
    let k = (max_w as f64 / w as f64).min(max_h as f64 / h as f64).min(grow);
    (((w as f64 * k).round() as i32).max(100), ((h as f64 * k).round() as i32).max(100) + header)
}

/// Show `path` over `window`, titled `title`.
pub fn show(window: &gtk::Window, path: &Path, title: &str) {
    let path = path.to_path_buf();
    let (window, title) = (window.clone(), title.to_string());
    // Decoded at its own size: "fit" in the cover loader also scales up.
    let handle = async_runtime::spawn_blocking(move || gtk::gdk::Texture::from_filename(&path).ok());
    app::local(async move {
        let Ok(Some(texture)) = handle.await else {
            crate::ui::bus::toast("Couldn't open the image");
            return;
        };
        let header = crate::theme::scaled(46);
        let (cw, ch) = fit(texture.width(), texture.height(), window.width(), window.height(), header, crate::theme::scaled(480));
        let picture = gtk::Picture::builder().paintable(&texture).content_fit(gtk::ContentFit::Contain).can_shrink(true).hexpand(true).vexpand(true).css_classes(["image-viewer-picture"]).build();
        let tv = adw::ToolbarView::new();
        tv.add_top_bar(&adw::HeaderBar::new());
        tv.set_content(Some(&picture));
        let dialog = adw::Dialog::builder()
            .title(&title)
            .child(&tv)
            .content_width(cw)
            .content_height(ch)
            .presentation_mode(adw::DialogPresentationMode::Floating)
            .css_classes(["image-viewer"])
            .build();
        let click = gtk::GestureClick::new();
        click.connect_released(glib::clone!(#[weak] dialog, move |_, _, _, _| {
            dialog.close();
        }));
        picture.add_controller(click);
        dialog.present(Some(&window));
    });
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn opens_at_its_own_size_and_shrinks_to_the_window() {
        // A 400 px poster in a big window: its own size.
        assert_eq!(fit(400, 500, 2560, 1440, 40, 480), (400, 540));
        // A tall scan in a small tile: scaled to fit, shape kept.
        assert_eq!(fit(1000, 2000, 800, 600, 40, 480), (256, 552));
        // A wide scan: limited by the width.
        assert_eq!(fit(3000, 1000, 1000, 1000, 40, 480), (952, 357));
        // A 120 px preview is enlarged to the minimum height.
        assert_eq!(fit(96, 120, 2560, 1440, 40, 480), (384, 520));
    }
}
