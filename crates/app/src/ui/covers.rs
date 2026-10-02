//! Cover art: Tier 1 poster pack (400 px) then Tier 0 bundled preview
//! (120 px), both `<dir>/<thumbnail_key>.jpg`. Directories are resolved once
//! per collection (and again after a pack lands); textures are decoded off
//! the main thread and cached by path.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use exorchy_core::commands::{assets, setup};
use exorchy_core::host::async_runtime;
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;

use crate::app;

/// The Media Pack's cover set, keyed like a collection's.
pub const MEDIA_SOURCE: &str = "eXoMedia";

type Done = Box<dyn FnOnce(Option<gdk::Texture>)>;

/// Target pixel size of a decoded cover. `Fill` crops to exactly (w, h);
/// `Fit` scales to fit inside (w, h), keeping the aspect ratio; `Boxed(w)`
/// keeps the art's own shape at a box-like size around width `w`, with DOS
/// screens shown as a CRT showed them (`boxed_size`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Size {
    Fill(u32, u32),
    Fit(u32, u32),
    Boxed(u32),
}

/// The display size of a cover of `w`×`h` px in `Boxed(base)`. eXo's
/// covers are box scans (portrait, about 4:5) or title screens; a 320×200
/// DOS screen is stored at 1.6:1 but a CRT drew it at 4:3, so that shape
/// is shown at 4:3. Portrait art is `base` wide, as a box stood on a shelf;
/// landscape art gets a little more width so it is not a thumbnail beside
/// the boxes. Extreme shapes are clamped.
pub fn boxed_size(w: u32, h: u32, base: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (base, base * 5 / 4);
    }
    let mut ratio = w as f64 / h as f64;
    if (1.55..=1.65).contains(&ratio) {
        ratio = 4.0 / 3.0;
    }
    let ratio = ratio.clamp(0.6, 1.8);
    let width = if ratio <= 1.0 { base as f64 } else { base as f64 * 1.3 };
    (width.round() as u32, (width / ratio).round() as u32)
}

struct Covers {
    preview_dirs: HashMap<String, String>,
    poster_dirs: HashMap<String, String>,
    loaded: bool,
    cache: HashMap<(PathBuf, Size), Option<gdk::Texture>>,
    in_flight: HashMap<(PathBuf, Size), Vec<Done>>,
    /// Callbacks for a dir refresh (a poster pack was installed).
    dirs_changed: Vec<Rc<dyn Fn()>>,
}

thread_local! {
    static COVERS: RefCell<Covers> = RefCell::new(Covers {
        preview_dirs: HashMap::new(),
        poster_dirs: HashMap::new(),
        loaded: false,
        cache: HashMap::new(),
        in_flight: HashMap::new(),
        dirs_changed: Vec::new(),
    });
}

/// Cap on cached textures (a card-sized cover is ~150 KB).
const CACHE_MAX: usize = 900;

/// Resolve every collection's preview and poster dir. Called at startup and
/// after content-pack changes; listeners re-request their covers.
pub fn load_dirs() {
    let core = app::core();
    app::spawn(
        async move {
            let available = setup::get_available_collections(core.state()).await.unwrap_or_default();
            let mut sources: Vec<String> = available.into_iter().map(|c| c.id).collect();
            sources.push(MEDIA_SOURCE.to_string());
            let mut previews = HashMap::new();
            let mut posters = HashMap::new();
            for id in sources {
                if let Ok(d) = assets::get_preview_dir(id.clone()).await {
                    previews.insert(id.clone(), d);
                }
                if let Ok(d) = assets::get_poster_dir(core.state(), id.clone()).await {
                    posters.insert(id, d);
                }
            }
            (previews, posters)
        },
        |(previews, posters)| {
            let changed = COVERS.with(|c| {
                let mut c = c.borrow_mut();
                let changed = c.preview_dirs != previews || c.poster_dirs != posters || !c.loaded;
                c.preview_dirs = previews;
                c.poster_dirs = posters;
                c.loaded = true;
                if changed {
                    // A poster pack landed (or went): cached misses for its
                    // paths are stale, and so are the lower-tier hits.
                    c.cache.clear();
                }
                changed
            });
            if changed {
                let cbs: Vec<Rc<dyn Fn()>> = COVERS.with(|c| c.borrow().dirs_changed.clone());
                for cb in cbs {
                    cb();
                }
            }
        },
    );
}

pub fn on_dirs_changed(f: impl Fn() + 'static) {
    COVERS.with(|c| c.borrow_mut().dirs_changed.push(Rc::new(f)));
}

/// Candidate paths, best tier first.
pub fn candidates(source: Option<&str>, key: Option<&str>) -> Vec<PathBuf> {
    let Some(key) = key.filter(|k| !k.is_empty()) else { return vec![] };
    let source = source.unwrap_or("eXoDOS");
    COVERS.with(|c| {
        let c = c.borrow();
        let mut out = Vec::new();
        if let Some(d) = c.poster_dirs.get(source) {
            out.push(Path::new(d).join(format!("{key}.jpg")));
        }
        if let Some(d) = c.preview_dirs.get(source) {
            out.push(Path::new(d).join(format!("{key}.jpg")));
        }
        out
    })
}

/// Load the best available cover at `size`; `done` runs on the main thread
/// with the texture or `None` when every tier failed.
pub fn request(source: Option<&str>, key: Option<&str>, size: Size, done: impl FnOnce(Option<gdk::Texture>) + 'static) {
    let paths = candidates(source, key);
    walk(paths, 0, size, Box::new(done));
}

fn walk(paths: Vec<PathBuf>, idx: usize, size: Size, done: Done) {
    let Some(path) = paths.get(idx).cloned() else {
        done(None);
        return;
    };
    let cached = COVERS.with(|c| c.borrow().cache.get(&(path.clone(), size)).cloned());
    match cached {
        Some(Some(t)) => done(Some(t)),
        Some(None) => walk(paths, idx + 1, size, done),
        None => {
            let next: Done = Box::new(move |t| match t {
                Some(t) => done(Some(t)),
                None => walk(paths, idx + 1, size, done),
            });
            load(path, size, next);
        }
    }
}

/// Decode and scale on a blocking thread; the texture is uploaded lazily by
/// GTK when first drawn.
pub fn load_scaled(path: &Path, size: Size) -> Option<gdk::Texture> {
    if !path.is_file() {
        return None;
    }
    let img = image::open(path).ok()?;
    let scaled = match size {
        Size::Fill(w, h) => img.resize_to_fill(w, h, image::imageops::FilterType::Triangle),
        Size::Fit(w, h) => img.resize(w, h, image::imageops::FilterType::Triangle),
        Size::Boxed(base) => {
            let (w, h) = boxed_size(img.width(), img.height(), base);
            img.resize_exact(w, h, image::imageops::FilterType::Triangle)
        }
    };
    let rgba = scaled.to_rgba8();
    let (w, h) = rgba.dimensions();
    let bytes = glib::Bytes::from_owned(rgba.into_raw());
    Some(gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, (w * 4) as usize).upcast())
}

fn load(path: PathBuf, size: Size, done: Done) {
    let key = (path.clone(), size);
    let start = COVERS.with(|c| {
        let mut c = c.borrow_mut();
        let waiting = c.in_flight.entry(key.clone()).or_default();
        waiting.push(done);
        waiting.len() == 1
    });
    if !start {
        return;
    }
    let p = path.clone();
    let handle = async_runtime::spawn_blocking(move || load_scaled(&p, size));
    app::local(async move {
        let texture = handle.await.ok().flatten();
        let waiting = COVERS.with(|c| {
            let mut c = c.borrow_mut();
            if c.cache.len() >= CACHE_MAX {
                c.cache.clear();
            }
            c.cache.insert(key.clone(), texture.clone());
            c.in_flight.remove(&key).unwrap_or_default()
        });
        for cb in waiting {
            cb(texture.clone());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::boxed_size;

    #[test]
    fn boxed_covers_keep_their_shape() {
        // A box scan keeps its proportions at the base width.
        assert_eq!(boxed_size(316, 400, 150), (150, 190));
        // A 320x200 DOS screen (stored 400x250) is drawn at a CRT's 4:3.
        assert_eq!(boxed_size(400, 250, 150), (195, 146));
        // A 4:3 screen stays 4:3.
        assert_eq!(boxed_size(400, 300, 150), (195, 146));
        // A banner is clamped rather than drawn as a sliver.
        assert_eq!(boxed_size(800, 100, 150), (195, 108));
        assert_eq!(boxed_size(0, 0, 150), (150, 187));
    }
}
