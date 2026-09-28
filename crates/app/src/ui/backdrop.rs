//! The user's background image (Settings → Appearance): a picture behind
//! every page, drawn at the chosen opacity over the theme's background
//! colour. While one is set the window wears `has-backdrop`, which turns the
//! full-width bars and pages translucent (`styles/backdrop.css`); cards,
//! panels and dialogs stay solid so text keeps its contrast.
//!
//! The chosen file is copied into the app's data folder, so moving or
//! deleting the original does not lose it. Config keys: `background_image`
//! (the copy's path, empty = none) and `background_opacity` (0–100).

use std::cell::RefCell;
use std::path::PathBuf;

use exorchy_core::commands::games;
use gtk::prelude::*;

use crate::app;

/// Opacity of a freshly chosen image, in percent.
pub const DEFAULT_OPACITY: u32 = 30;

thread_local! {
    static SHELL: RefCell<Option<(gtk::Window, gtk::Picture)>> = const { RefCell::new(None) };
}

/// The picture for the window to put behind its pages; loads the saved
/// choice once the backend answers.
pub fn install(window: &impl IsA<gtk::Window>) -> gtk::Picture {
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .can_target(false)
        .hexpand(true)
        .vexpand(true)
        .visible(false)
        .css_classes(["backdrop"])
        .build();
    SHELL.with(|s| *s.borrow_mut() = Some((window.clone().upcast(), picture.clone())));
    let core = app::core();
    app::spawn(
        async move {
            let image = games::get_config(core.state(), "background_image".into()).await.ok().flatten();
            let opacity = games::get_config(core.state(), "background_opacity".into()).await.ok().flatten();
            (image, opacity)
        },
        |(image, opacity)| {
            show(image.filter(|p| !p.is_empty()).map(PathBuf::from));
            set_opacity(parse_opacity(opacity.as_deref()));
        },
    );
    picture
}

/// Stored opacity in percent; unset or unreadable = the default.
pub fn parse_opacity(v: Option<&str>) -> u32 {
    v.and_then(|s| s.parse::<u32>().ok()).map(|n| n.min(100)).unwrap_or(DEFAULT_OPACITY)
}

fn show(path: Option<PathBuf>) {
    let Some((window, picture)) = SHELL.with(|s| s.borrow().clone()) else { return };
    let texture = path.as_ref().and_then(|p| match gtk::gdk::Texture::from_filename(p) {
        Ok(t) => Some(t),
        Err(e) => {
            log::warn!("background image {}: {e}", p.display());
            None
        }
    });
    let on = texture.is_some();
    picture.set_paintable(texture.as_ref());
    picture.set_visible(on);
    if on {
        window.add_css_class("has-backdrop");
    } else {
        window.remove_css_class("has-backdrop");
    }
}

pub fn set_opacity(percent: u32) {
    if let Some((_, picture)) = SHELL.with(|s| s.borrow().clone()) {
        picture.set_opacity(percent.min(100) as f64 / 100.0);
    }
}

/// Copy `source` into the data folder, store and show it. `done` gets the
/// stored copy's path or the error.
pub fn choose(source: PathBuf, done: impl FnOnce(Result<String, String>) + 'static) {
    let core = app::core();
    app::spawn(
        async move {
            let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("img").to_lowercase();
            let dir = exorchy_core::commands::paths::app_data_dir();
            // A new name per choice: the texture cache must not hand back the
            // previous image for the same path.
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let target = dir.join(format!("background-{stamp}.{ext}"));
            tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
            tokio::fs::copy(&source, &target).await.map_err(|e| format!("could not copy the image: {e}"))?;
            let target_s = target.to_string_lossy().into_owned();
            let old = games::get_config(core.state(), "background_image".into()).await.ok().flatten();
            games::set_config(core.clone(), core.state(), "background_image".into(), target_s.clone()).await?;
            if let Some(old) = old.filter(|o| !o.is_empty() && *o != target_s) {
                let _ = tokio::fs::remove_file(old).await;
            }
            Ok(target_s)
        },
        move |res: Result<String, String>| {
            if let Ok(path) = &res {
                show(Some(PathBuf::from(path)));
            }
            done(res);
        },
    );
}

/// Drop the image (and its copy).
pub fn clear(done: impl FnOnce(Result<(), String>) + 'static) {
    let core = app::core();
    app::spawn(
        async move {
            let old = games::get_config(core.state(), "background_image".into()).await.ok().flatten();
            games::set_config(core.clone(), core.state(), "background_image".into(), String::new()).await?;
            if let Some(old) = old.filter(|o| !o.is_empty()) {
                let _ = tokio::fs::remove_file(old).await;
            }
            Ok(())
        },
        move |res: Result<(), String>| {
            if res.is_ok() {
                show(None);
            }
            done(res);
        },
    );
}

/// Store the opacity (the slider shows it live through `set_opacity`).
pub fn save_opacity(percent: u32) {
    let core = app::core();
    app::spawn(
        async move { games::set_config(core.clone(), core.state(), "background_opacity".into(), percent.min(100).to_string()).await },
        |res| {
            if let Err(e) = res {
                log::error!("background opacity: {e}");
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_reads_percent_with_a_default() {
        assert_eq!(parse_opacity(None), DEFAULT_OPACITY);
        assert_eq!(parse_opacity(Some("x")), DEFAULT_OPACITY);
        assert_eq!(parse_opacity(Some("55")), 55);
        assert_eq!(parse_opacity(Some("250")), 100);
    }
}
