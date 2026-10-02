//! The in-app document viewer shared by game manuals and Reading Room
//! issues: a header (title, subtitle, caller actions, "Open externally",
//! close) over a body for each document kind.
//!
//! PDFs are rendered with poppler on a dedicated thread that owns the
//! document; the main thread only turns finished bitmaps into textures.
//! Pages scroll continuously: every page has a placeholder of its final
//! size, only the pages within [`RENDER_MARGIN`] of the viewport are
//! rendered, at most [`RENDER_BUDGET`] stay decoded, and a zoom or width
//! change bumps a generation so a late bitmap is dropped. Images, plain
//! text and HTML manuals (tag-stripped: there is no web view in the GTK
//! shell) have their own bodies.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

use adw::prelude::*;
use exorchy_core::commands::games;
use exorchy_core::host::async_runtime;
use gtk::{gdk, glib};

use crate::app;
use crate::ui::covers;

/// How far beyond the viewport a page starts rendering, in logical pixels.
/// A scan is ~1400 px tall at fit width, so this is about one page of lead.
const RENDER_MARGIN: f64 = 1600.0;
/// Rendered pages held at once. A page is ~6 MB of bitmap at fit width, so
/// this is the memory budget as much as the cache size.
const RENDER_BUDGET: usize = 6;
/// Renders queued on the thread at once: it draws strictly in request
/// order, so a queue of every page that scrolled past is what puts the one
/// on screen far behind the picture.
const MAX_IN_FLIGHT: usize = 2;
/// Low enough for "Fit page" on a tall scan in a short window.
const MIN_ZOOM: f64 = 0.2;
const MAX_ZOOM: f64 = 4.0;
const ZOOM_STEP: f64 = 0.2;
/// Cap on a rendered bitmap's width; beyond it the texture is upscaled.
const MAX_RENDER_PX: i32 = 4096;
/// Space between pages and above the first one, in logical pixels.
const PAGE_GAP: i32 = 12;
/// Horizontal padding around the page column.
const COLUMN_PAD: i32 = 32;

/// A 1-based page-change callback.
pub type PageCb = Box<dyn Fn(u32)>;

/// What a document is, from its file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pdf,
    Image,
    Text,
    Html,
    Unknown,
}

pub fn kind_of(path: &str) -> Kind {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pdf" => Kind::Pdf,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" => Kind::Image,
        "txt" | "text" | "nfo" | "doc" | "md" => Kind::Text,
        "html" | "htm" => Kind::Html,
        _ => Kind::Unknown,
    }
}

pub fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Pdf => "PDF",
        Kind::Image => "Image",
        Kind::Text => "Text",
        Kind::Html => "HTML",
        Kind::Unknown => "Document",
    }
}

/// Which rendered pages to release so that at most `budget` stay: least
/// recently seen first, and never one that is currently in view.
pub fn evictable(rendered: &[i32], visible: &HashSet<i32>, recent: &[i32], budget: usize) -> Vec<i32> {
    if rendered.len() <= budget {
        return Vec::new();
    }
    let over = rendered.len() - budget;
    let rank = |p: i32| recent.iter().position(|r| *r == p).unwrap_or(usize::MAX);
    let mut held: Vec<i32> = rendered.iter().copied().filter(|p| !visible.contains(p)).collect();
    held.sort_by_key(|a| std::cmp::Reverse(rank(*a)));
    held.truncate(over);
    held
}

/// Which of the `wanted` pages to start next: the nearest to `focus` first,
/// a page below it before the one above at equal distance, at most `slots`.
pub fn next_to_render(wanted: &[i32], drawn: &HashSet<i32>, in_flight: &HashSet<i32>, focus: i32, slots: usize) -> Vec<i32> {
    if slots == 0 {
        return Vec::new();
    }
    let mut out: Vec<i32> = wanted.iter().copied().filter(|p| !drawn.contains(p) && !in_flight.contains(p)).collect();
    out.sort_by(|a, b| (a - focus).abs().cmp(&(b - focus).abs()).then_with(|| b.cmp(a)));
    out.truncate(slots);
    out
}

/// A manual's HTML as readable text: block tags become line breaks, scripts
/// and styles are dropped, the common entities are decoded.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let mut skip_until: Option<&str> = None;
    while let Some(start) = rest.find('<') {
        let text = &rest[..start];
        if skip_until.is_none() {
            out.push_str(&decode_entities(text));
        }
        let Some(end) = rest[start..].find('>') else { break };
        let tag = &rest[start + 1..start + end];
        let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("").to_ascii_lowercase();
        let closing = tag.starts_with('/');
        if let Some(until) = skip_until {
            if closing && name == until {
                skip_until = None;
            }
        } else if !closing && (name == "script" || name == "style") {
            skip_until = Some(if name == "script" { "script" } else { "style" });
        } else if name == "br"
            || matches!(
                name.as_str(),
                "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "pre" | "table" | "ul" | "ol" | "hr" | "blockquote" | "title"
            )
        {
            out.push('\n');
        } else if name == "td" || name == "th" {
            out.push(' ');
        }
        rest = &rest[start + end + 1..];
    }
    if skip_until.is_none() {
        out.push_str(&decode_entities(rest));
    }
    // Collapse runs of blank lines the block tags leave behind.
    let mut lines: Vec<&str> = Vec::new();
    let mut blank = 0;
    for line in out.lines() {
        let t = line.trim_end();
        if t.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        lines.push(t);
    }
    lines.join("\n").trim().to_string()
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(semi) = tail.find(';').filter(|n| *n <= 8) else {
            out.push('&');
            rest = &rest[i + 1..];
            continue;
        };
        let entity = &tail[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            "copy" => Some('©'),
            "reg" => Some('®'),
            "trade" => Some('™'),
            "hellip" => Some('…'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ── The render thread ────────────────────────────────────────────────────────

struct Rendered {
    width: i32,
    height: i32,
    stride: usize,
    /// Cairo ARGB32: BGRA premultiplied on little-endian hosts.
    bytes: Vec<u8>,
}

struct Opened {
    n_pages: i32,
    /// Page sizes in points.
    sizes: Vec<(f64, f64)>,
}

enum Req {
    Render { page: i32, width_px: i32, generation: u64, reply: tokio::sync::oneshot::Sender<Option<Rendered>> },
    Search { needle: String, generation: u64, reply: tokio::sync::oneshot::Sender<Vec<i32>> },
}

fn render_page(page: &poppler::Page, width_px: i32) -> Result<Rendered, String> {
    let (pw, ph) = page.size();
    let scale = width_px as f64 / pw.max(1.0);
    let height_px = ((ph * scale).ceil() as i32).max(1);
    let mut surface =
        gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width_px, height_px).map_err(|e| e.to_string())?;
    {
        let cr = gtk::cairo::Context::new(&surface).map_err(|e| e.to_string())?;
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.paint().map_err(|e| e.to_string())?;
        cr.scale(scale, scale);
        page.render(&cr);
    }
    surface.flush();
    let stride = surface.stride() as usize;
    let data = surface.data().map_err(|e| e.to_string())?;
    Ok(Rendered { width: width_px, height: height_px, stride, bytes: data.to_vec() })
}

/// Open the document on its own thread and serve render and search
/// requests from there. The thread ends when the sender is dropped.
fn spawn_renderer(
    path: String,
    opened: tokio::sync::oneshot::Sender<Result<Opened, String>>,
    generation: Arc<AtomicU64>,
    search_generation: Arc<AtomicU64>,
) -> mpsc::Sender<Req> {
    let (tx, rx) = mpsc::channel::<Req>();
    let spawned = std::thread::Builder::new().name("pdf-render".into()).spawn(move || {
        let uri = match glib::filename_to_uri(&path, None) {
            Ok(u) => u,
            Err(e) => {
                let _ = opened.send(Err(e.to_string()));
                return;
            }
        };
        let doc = match poppler::Document::from_file(&uri, None) {
            Ok(d) => d,
            Err(e) => {
                let _ = opened.send(Err(e.to_string()));
                return;
            }
        };
        let n_pages = doc.n_pages();
        let sizes = (0..n_pages).map(|i| doc.page(i).map(|p| p.size()).unwrap_or((612.0, 792.0))).collect();
        if opened.send(Ok(Opened { n_pages, sizes })).is_err() {
            return;
        }
        while let Ok(req) = rx.recv() {
            match req {
                Req::Render { page, width_px, generation: g, reply } => {
                    if generation.load(Ordering::Relaxed) != g {
                        let _ = reply.send(None);
                        continue;
                    }
                    let result = doc.page(page).and_then(|p| match render_page(&p, width_px) {
                        Ok(r) => Some(r),
                        Err(e) => {
                            log::error!("page {} render failed: {e}", page + 1);
                            None
                        }
                    });
                    let _ = reply.send(result);
                }
                Req::Search { needle, generation: g, reply } => {
                    let mut hits = Vec::new();
                    for i in 0..n_pages {
                        if search_generation.load(Ordering::Relaxed) != g {
                            break;
                        }
                        if let Some(p) = doc.page(i) {
                            if !p.find_text(&needle).is_empty() {
                                hits.push(i + 1);
                            }
                        }
                    }
                    let _ = reply.send(hits);
                }
            }
        }
    });
    if let Err(e) = spawned {
        log::error!("could not start the PDF thread: {e}");
    }
    tx
}

// ── PDF pages ────────────────────────────────────────────────────────────────

/// How the zoom follows the viewport: a whole page in view (the default),
/// the column's width, or wherever the reader zoomed to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fit {
    Page,
    Width,
    Free,
}

struct PdfPages {
    tx: mpsc::Sender<Req>,
    fit: Cell<Fit>,
    /// A refit is queued for after the current size pass.
    fit_queued: Cell<bool>,
    n_pages: i32,
    sizes: Vec<(f64, f64)>,
    /// Logical page heights at the current zoom, in page order.
    heights: RefCell<Vec<i32>>,
    widths: RefCell<Vec<i32>>,
    placeholders: Vec<gtk::Box>,
    pictures: Vec<gtk::Picture>,
    scroller: gtk::ScrolledWindow,
    /// 1.0 spans the column; a scan's own geometry is never the reference.
    zoom: Cell<f64>,
    column_width: Cell<i32>,
    generation: Arc<AtomicU64>,
    search_generation: Arc<AtomicU64>,
    /// Page (0-based) → the generation its texture was rendered at.
    drawn: RefCell<HashMap<i32, u64>>,
    in_flight: RefCell<HashSet<i32>>,
    /// Most recently seen first: what the budget evicts by.
    recent: RefCell<Vec<i32>>,
    scheduled: Cell<bool>,
    /// 1-based page under the top of the viewport.
    current: Cell<i32>,
    on_page: RefCell<Option<PageCb>>,
    page_entry: gtk::Entry,
    zoom_label: gtk::Label,
    prev_btn: gtk::Button,
    next_btn: gtk::Button,
    hits_bar: gtk::Box,
    hits_row: gtk::Box,
    hits_label: gtk::Label,
    search_entry: gtk::SearchEntry,
}

impl PdfPages {
    fn page_top(&self, page: i32) -> i32 {
        let heights = self.heights.borrow();
        PAGE_GAP + heights.iter().take(page.max(0) as usize).map(|h| h + PAGE_GAP).sum::<i32>()
    }

    /// Size every placeholder for the current zoom and column width.
    fn relayout(self: &Rc<Self>) {
        let cw = (self.column_width.get() - COLUMN_PAD).max(50);
        let zoom = self.zoom.get();
        let mut heights = Vec::with_capacity(self.sizes.len());
        let mut widths = Vec::with_capacity(self.sizes.len());
        for (i, (pw, ph)) in self.sizes.iter().enumerate() {
            let w = ((cw as f64 * zoom).round() as i32).max(50);
            let h = ((w as f64 * ph / pw.max(1.0)).round() as i32).max(20);
            self.placeholders[i].set_size_request(w, h);
            widths.push(w);
            heights.push(h);
        }
        self.heights.replace(heights);
        self.widths.replace(widths);
        log::debug!("pdf: relayout cw={cw} zoom={zoom}");
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.zoom_label.set_label(&format!("{}%", (zoom * 100.0).round() as i32));
        self.schedule();
    }

    /// The pages whose placeholders intersect the viewport plus its margin,
    /// and the page under the viewport's top.
    fn visible(&self) -> (Vec<i32>, i32) {
        let vadj = self.scroller.vadjustment();
        let top = vadj.value();
        let bottom = top + vadj.page_size();
        let heights = self.heights.borrow();
        let mut y = PAGE_GAP as f64;
        let mut visible = Vec::new();
        let mut current = self.current.get();
        let mut found = false;
        for (i, h) in heights.iter().enumerate() {
            let h = *h as f64;
            if y + h > top - RENDER_MARGIN && y < bottom + RENDER_MARGIN {
                visible.push(i as i32);
            }
            if !found && y <= top + 80.0 && y + h > top {
                current = i as i32 + 1;
                found = true;
            }
            y += h + PAGE_GAP as f64;
        }
        if !found && top < PAGE_GAP as f64 {
            current = 1;
        }
        (visible, current)
    }

    fn set_current(&self, page: i32) {
        if page == self.current.get() {
            return;
        }
        self.current.set(page);
        self.page_entry.set_text(&page.to_string());
        self.prev_btn.set_sensitive(page > 1);
        self.next_btn.set_sensitive(page < self.n_pages);
        if let Some(cb) = self.on_page.borrow().as_ref() {
            cb(page as u32);
        }
    }

    /// One pass on the next idle: a page that only scrolls past never reaches
    /// the thread, and after a stop the nearest page goes first.
    fn schedule(self: &Rc<Self>) {
        if self.scheduled.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(p) = weak.upgrade() {
                p.scheduled.set(false);
                p.run_queue();
            }
        });
    }

    fn run_queue(self: &Rc<Self>) {
        let (visible, current) = self.visible();
        log::debug!("pdf: queue visible={visible:?} current={current} in_flight={:?}", self.in_flight.borrow());
        self.set_current(current);
        {
            let mut recent = self.recent.borrow_mut();
            for p in &visible {
                recent.retain(|r| r != p);
                recent.insert(0, *p);
            }
            recent.truncate(64);
        }
        let generation = self.generation.load(Ordering::Relaxed);
        let visible_set: HashSet<i32> = visible.iter().copied().collect();
        // Trim: a page leaving the viewport is kept until the budget needs
        // its memory, so a short scroll back finds it already drawn.
        let evict = {
            let drawn = self.drawn.borrow();
            let held: Vec<i32> = drawn.keys().copied().collect();
            evictable(&held, &visible_set, &self.recent.borrow(), RENDER_BUDGET)
        };
        for p in evict {
            self.drawn.borrow_mut().remove(&p);
            self.pictures[p as usize].set_paintable(gdk::Paintable::NONE);
        }
        let drawn_now: HashSet<i32> = self.drawn.borrow().iter().filter(|(_, g)| **g == generation).map(|(p, _)| *p).collect();
        let slots = MAX_IN_FLIGHT.saturating_sub(self.in_flight.borrow().len());
        let next = next_to_render(&visible, &drawn_now, &self.in_flight.borrow(), current - 1, slots);
        for p in next {
            self.render(p, generation);
        }
    }

    fn render(self: &Rc<Self>, page: i32, generation: u64) {
        let scale = self.scroller.scale_factor().max(1);
        let width_px = (self.widths.borrow()[page as usize] * scale).min(MAX_RENDER_PX);
        let (reply, rx) = tokio::sync::oneshot::channel();
        if self.tx.send(Req::Render { page, width_px, generation, reply }).is_err() {
            return;
        }
        self.in_flight.borrow_mut().insert(page);
        let weak = Rc::downgrade(self);
        app::local(async move {
            let result = rx.await.ok().flatten();
            log::debug!("pdf: page {} rendered: {}", page + 1, result.is_some());
            let Some(pages) = weak.upgrade() else { return };
            pages.in_flight.borrow_mut().remove(&page);
            if let Some(r) = result {
                if pages.generation.load(Ordering::Relaxed) == generation {
                    let bytes = glib::Bytes::from_owned(r.bytes);
                    let texture = gdk::MemoryTexture::new(r.width, r.height, gdk::MemoryFormat::B8g8r8a8Premultiplied, &bytes, r.stride);
                    pages.pictures[page as usize].set_paintable(Some(&texture));
                    pages.drawn.borrow_mut().insert(page, generation);
                }
            }
            pages.schedule();
        });
    }

    fn scroll_to_page(self: &Rc<Self>, page: i32) {
        let page = page.clamp(1, self.n_pages.max(1));
        let top = (self.page_top(page - 1) - 8).max(0) as f64;
        let vadj = self.scroller.vadjustment();
        vadj.set_value(top.min((vadj.upper() - vadj.page_size()).max(0.0)));
        self.set_current(page);
        self.schedule();
    }

    /// Re-scroll to the current page once the new sizes have been laid out
    /// (the adjustment's range follows the next layout pass).
    fn keep_page(self: &Rc<Self>) {
        let page = self.current.get();
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(p) = weak.upgrade() {
                p.scroll_to_page(page);
            }
        });
    }

    /// A zoom the reader chose: the fit mode lets go.
    fn set_zoom(self: &Rc<Self>, zoom: f64) {
        self.fit.set(Fit::Free);
        self.apply_zoom(zoom);
    }

    /// Refit after a viewport resize. The size notifications arrive in the
    /// middle of GTK's allocation, where resizing the pages is not allowed.
    fn queue_fit(self: &Rc<Self>) {
        if self.fit.get() != Fit::Page || self.fit_queued.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(p) = weak.upgrade() {
                p.fit_queued.set(false);
                if p.fit.get() == Fit::Page {
                    p.fit_page();
                }
            }
        });
    }

    fn set_fit(self: &Rc<Self>, fit: Fit) {
        self.fit.set(fit);
        match fit {
            Fit::Page => self.fit_page(),
            Fit::Width => self.apply_zoom(1.0),
            Fit::Free => {}
        }
    }

    fn apply_zoom(self: &Rc<Self>, zoom: f64) {
        let zoom = (zoom * 100.0).round() / 100.0;
        self.zoom.set(zoom.clamp(MIN_ZOOM, MAX_ZOOM));
        self.relayout();
        self.keep_page();
    }

    fn fit_page(self: &Rc<Self>) {
        let cw = (self.column_width.get() - COLUMN_PAD).max(50) as f64;
        let vh = (self.scroller.vadjustment().page_size() - 2.0 * PAGE_GAP as f64).max(50.0);
        let idx = (self.current.get() - 1).clamp(0, self.n_pages.max(1) - 1) as usize;
        let (pw, ph) = self.sizes.get(idx).copied().unwrap_or((612.0, 792.0));
        let zoom = vh / (cw * ph / pw.max(1.0));
        self.apply_zoom(zoom.min(1.0));
    }

    fn search(self: &Rc<Self>) {
        let needle = self.search_entry.text().trim().to_string();
        let g = self.search_generation.fetch_add(1, Ordering::Relaxed) + 1;
        if needle.chars().count() < 2 {
            self.hits_bar.set_visible(false);
            self.hits_label.set_label("");
            return;
        }
        self.hits_label.set_label("searching…");
        let (reply, rx) = tokio::sync::oneshot::channel();
        if self.tx.send(Req::Search { needle, generation: g, reply }).is_err() {
            return;
        }
        let weak = Rc::downgrade(self);
        app::local(async move {
            let hits = rx.await.unwrap_or_default();
            let Some(pages) = weak.upgrade() else { return };
            if pages.search_generation.load(Ordering::Relaxed) != g {
                return;
            }
            pages.hits_label.set_label(&format!("{} page{}", hits.len(), if hits.len() == 1 { "" } else { "s" }));
            while let Some(c) = pages.hits_row.first_child() {
                pages.hits_row.remove(&c);
            }
            for p in &hits {
                let b = gtk::Button::builder().label(p.to_string()).css_classes(["btn", "small", "doc-hit"]).build();
                let page = *p;
                b.connect_clicked(glib::clone!(#[weak] pages, move |_| pages.scroll_to_page(page)));
                pages.hits_row.append(&b);
            }
            pages.hits_bar.set_visible(!hits.is_empty());
        });
    }
}

// ── The viewer ───────────────────────────────────────────────────────────────

/// The viewer widget: header and body. Present it with [`present`] or embed
/// it in a caller's own dialog (the Reading Room's issue reader does).
pub struct DocumentView {
    pub widget: gtk::Box,
    /// Caller-specific header buttons, before "Open externally".
    pub actions: gtk::Box,
    path: String,
    close_cb: RefCell<Option<Box<dyn Fn()>>>,
    pdf: RefCell<Option<Rc<PdfPages>>>,
}

impl DocumentView {
    /// `start_page` is 1-based; `on_page` fires when the page under the top
    /// of the viewport changes (PDF only).
    pub fn new(path: &str, title: &str, subtitle: Option<&str>, on_page: Option<PageCb>, start_page: u32) -> Rc<Self> {
        let widget = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["doc-viewer"]).hexpand(true).vexpand(true).build();

        // Header.
        let header = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).css_classes(["doc-header"]).build();
        let titles = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(1).hexpand(true).valign(gtk::Align::Center).build();
        titles.append(&gtk::Label::builder().label(title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["doc-title"]).build());
        if let Some(sub) = subtitle.filter(|s| !s.is_empty()) {
            titles.append(&gtk::Label::builder().label(sub).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["muted", "small"]).build());
        }
        header.append(&titles);
        let actions = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(6).build();
        header.append(&actions);
        let text_zoom = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(2).visible(false).build();
        header.append(&text_zoom);
        let external = gtk::Button::builder().label("↗ Open externally").css_classes(["btn", "small"]).tooltip_text("Open in the system viewer").build();
        header.append(&external);
        let close = gtk::Button::builder().icon_name("window-close-symbolic").css_classes(["btn", "icon", "ghost"]).tooltip_text("Close (Esc)").build();
        header.append(&close);
        widget.append(&header);

        let notice = gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).css_classes(["doc-notice"]).visible(false).build();
        widget.append(&notice);

        let body = gtk::Stack::builder().vexpand(true).hexpand(true).build();
        let status = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).css_classes(["muted"]).build();
        let status_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).valign(gtk::Align::Center).halign(gtk::Align::Center).build();
        let spinner = adw::Spinner::new();
        spinner.set_size_request(28, 28);
        status_box.append(&spinner);
        status_box.append(&status);
        body.add_named(&status_box, Some("status"));
        widget.append(&body);

        let view = Rc::new(DocumentView {
            widget,
            actions,
            path: path.to_string(),
            close_cb: RefCell::new(None),
            pdf: RefCell::new(None),
        });

        close.connect_clicked(glib::clone!(#[weak] view, move |_| view.close()));
        external.connect_clicked(glib::clone!(#[weak] view, #[weak] notice, move |_| {
            let core = app::core();
            let path = view.path.clone();
            let shown = path.clone();
            app::spawn(async move { games::open_document(core.clone(), core.state(), path).await }, move |res| {
                if let Err(e) = res {
                    log::error!("open_document failed: {e} (path {shown})");
                    notice.set_label(&format!("Could not open externally: {e}\nThe file is at {shown}"));
                    notice.set_visible(true);
                }
            });
        }));

        let kind = match kind_of(path) {
            Kind::Unknown => sniff_kind(path),
            k => k,
        };
        match kind {
            Kind::Pdf | Kind::Unknown => {
                status.set_label("Opening…");
                view.open_pdf(&body, &status, &spinner, on_page, start_page);
            }
            Kind::Image => {
                status.set_label("Loading…");
                view.open_image(&body, &status, &spinner);
            }
            Kind::Text | Kind::Html => {
                status.set_label("Loading…");
                view.open_text(&body, &status, &spinner, kind == Kind::Html, &text_zoom);
            }
        }
        view
    }

    /// Runs when the header's close button is pressed.
    pub fn connect_close(&self, f: impl Fn() + 'static) {
        self.close_cb.replace(Some(Box::new(f)));
    }

    fn close(&self) {
        if let Some(f) = self.close_cb.borrow().as_ref() {
            f();
        }
    }

    fn show_error(body: &gtk::Stack, status: &gtk::Label, spinner: &adw::Spinner, msg: &str) {
        spinner.set_visible(false);
        status.set_label(msg);
        body.set_visible_child_name("status");
    }

    fn open_pdf(self: &Rc<Self>, body: &gtk::Stack, status: &gtk::Label, spinner: &adw::Spinner, on_page: Option<PageCb>, start_page: u32) {
        let generation = Arc::new(AtomicU64::new(1));
        let search_generation = Arc::new(AtomicU64::new(0));
        let (opened_tx, opened_rx) = tokio::sync::oneshot::channel();
        let tx = spawn_renderer(self.path.clone(), opened_tx, generation.clone(), search_generation.clone());
        let view = Rc::downgrade(self);
        let (body, status, spinner) = (body.clone(), status.clone(), spinner.clone());
        app::local(async move {
            let opened = opened_rx.await.unwrap_or_else(|_| Err("the viewer stopped".into()));
            let Some(view) = view.upgrade() else { return };
            match opened {
                Err(e) => Self::show_error(&body, &status, &spinner, &format!("Could not open this document: {e}")),
                Ok(o) if o.n_pages < 1 => Self::show_error(&body, &status, &spinner, "This document has no pages."),
                Ok(o) => {
                    let (pages, root) = view.build_pdf(tx, o, generation, search_generation, on_page);
                    body.add_named(&root, Some("content"));
                    body.set_visible_child_name("content");
                    view.pdf.replace(Some(pages.clone()));
                    // The column has no width until it is mapped; the first
                    // layout runs from the width notification, and the start
                    // page after it.
                    let start = start_page.max(1) as i32;
                    pages.current.set(0);
                    let weak = Rc::downgrade(&pages);
                    glib::idle_add_local_once(move || {
                        if let Some(p) = weak.upgrade() {
                            p.column_width.set(p.scroller.hadjustment().page_size().max(0.0) as i32);
                            // Documents open with a whole page in view.
                            if p.fit.get() == Fit::Page {
                                p.fit_page();
                            } else {
                                p.relayout();
                            }
                            let weak = Rc::downgrade(&p);
                            glib::idle_add_local_once(move || {
                                if let Some(p) = weak.upgrade() {
                                    p.scroll_to_page(start);
                                    p.scroller.grab_focus();
                                }
                            });
                        }
                    });
                }
            }
        });
    }

    fn build_pdf(
        self: &Rc<Self>,
        tx: mpsc::Sender<Req>,
        opened: Opened,
        generation: Arc<AtomicU64>,
        search_generation: Arc<AtomicU64>,
        on_page: Option<PageCb>,
    ) -> (Rc<PdfPages>, gtk::Box) {
        let n = opened.n_pages;
        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(PAGE_GAP).margin_top(PAGE_GAP).margin_bottom(PAGE_GAP).halign(gtk::Align::Center).css_classes(["doc-pages"]).build();
        let mut placeholders = Vec::with_capacity(n as usize);
        let mut pictures = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let placeholder = gtk::Box::builder().css_classes(["doc-page"]).build();
            let picture = gtk::Picture::builder().can_shrink(true).content_fit(gtk::ContentFit::Fill).build();
            // The overlay child gets exactly the placeholder's allocation, so
            // a texture rendered at 2× for a HiDPI display lays out at its
            // logical size.
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&placeholder));
            overlay.add_overlay(&picture);
            column.append(&overlay);
            placeholders.push(placeholder);
            pictures.push(picture);
        }
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .focusable(true)
            .child(&column)
            .css_classes(["doc-scroller"])
            .build();

        // Toolbar: pager, zoom, search.
        let toolbar = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).css_classes(["doc-toolbar"]).build();
        let pager = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let prev_btn = gtk::Button::builder().icon_name("go-previous-symbolic").css_classes(["btn", "icon"]).tooltip_text("Previous page (Page Up)").build();
        let page_entry = gtk::Entry::builder().width_chars(4).max_width_chars(5).xalign(1.0).css_classes(["field", "doc-page-entry"]).input_purpose(gtk::InputPurpose::Digits).build();
        let total = gtk::Label::builder().label(format!("/ {n}")).css_classes(["muted"]).build();
        let next_btn = gtk::Button::builder().icon_name("go-next-symbolic").css_classes(["btn", "icon"]).tooltip_text("Next page (Page Down)").build();
        pager.append(&prev_btn);
        pager.append(&page_entry);
        pager.append(&total);
        pager.append(&next_btn);
        toolbar.append(&pager);
        let zoom_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let zoom_out = gtk::Button::builder().label("−").css_classes(["btn", "icon"]).tooltip_text("Zoom out (−)").build();
        let zoom_label = gtk::Label::builder().label("100%").width_chars(5).css_classes(["muted"]).build();
        let zoom_in = gtk::Button::builder().label("+").css_classes(["btn", "icon"]).tooltip_text("Zoom in (+)").build();
        let fit_w = gtk::Button::builder().label("Fit width").css_classes(["btn", "small"]).build();
        let fit_p = gtk::Button::builder().label("Fit page").css_classes(["btn", "small"]).build();
        zoom_box.append(&zoom_out);
        zoom_box.append(&zoom_label);
        zoom_box.append(&zoom_in);
        zoom_box.append(&fit_w);
        zoom_box.append(&fit_p);
        toolbar.append(&zoom_box);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        toolbar.append(&spacer);
        let search_entry = gtk::SearchEntry::builder().placeholder_text("Search text - Enter").css_classes(["search", "doc-search"]).build();
        let hits_label = gtk::Label::builder().css_classes(["muted", "small"]).build();
        toolbar.append(&search_entry);
        toolbar.append(&hits_label);

        let hits_bar = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(6).css_classes(["doc-hitbar"]).visible(false).build();
        hits_bar.append(&gtk::Label::builder().label("Found on pages").css_classes(["muted", "small"]).build());
        let hits_row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(4).build();
        let hits_scroller = gtk::ScrolledWindow::builder().vscrollbar_policy(gtk::PolicyType::Never).hscrollbar_policy(gtk::PolicyType::Automatic).hexpand(true).child(&hits_row).build();
        hits_bar.append(&hits_scroller);

        let pages = Rc::new(PdfPages {
            tx,
            fit: Cell::new(Fit::Page),
            fit_queued: Cell::new(false),
            n_pages: n,
            sizes: opened.sizes,
            heights: RefCell::new(vec![0; n as usize]),
            widths: RefCell::new(vec![0; n as usize]),
            placeholders,
            pictures,
            scroller,
            zoom: Cell::new(1.0),
            column_width: Cell::new(0),
            generation,
            search_generation,
            drawn: RefCell::new(HashMap::new()),
            in_flight: RefCell::new(HashSet::new()),
            recent: RefCell::new(Vec::new()),
            scheduled: Cell::new(false),
            current: Cell::new(0),
            on_page: RefCell::new(on_page),
            page_entry,
            zoom_label,
            prev_btn,
            next_btn,
            hits_bar,
            hits_row,
            hits_label,
            search_entry,
        });

        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        root.append(&toolbar);
        root.append(&pages.hits_bar);
        root.append(&pages.scroller);

        // "Fit page" follows the viewport's height too (a resize, full screen).
        pages.scroller.vadjustment().connect_page_size_notify(glib::clone!(#[weak] pages, move |_| pages.queue_fit()));
        // Scrolling picks the pages to render; a width change relays out.
        pages.scroller.vadjustment().connect_value_changed(glib::clone!(#[weak] pages, move |_| pages.schedule()));
        pages.scroller.hadjustment().connect_page_size_notify(glib::clone!(#[weak] pages, move |h| {
            let w = h.page_size().max(0.0) as i32;
            if (w - pages.column_width.get()).abs() > 2 && w > 0 {
                pages.column_width.set(w);
                if pages.fit.get() == Fit::Page {
                    pages.queue_fit();
                } else {
                    pages.relayout();
                    pages.keep_page();
                }
            }
        }));
        pages.scroller.connect_scale_factor_notify(glib::clone!(#[weak] pages, move |_| pages.relayout()));

        pages.prev_btn.connect_clicked(glib::clone!(#[weak] pages, move |_| pages.scroll_to_page(pages.current.get() - 1)));
        pages.next_btn.connect_clicked(glib::clone!(#[weak] pages, move |_| pages.scroll_to_page(pages.current.get() + 1)));
        pages.page_entry.connect_activate(glib::clone!(#[weak] pages, move |e| {
            if let Ok(p) = e.text().trim().parse::<i32>() {
                pages.scroll_to_page(p);
            } else {
                e.set_text(&pages.current.get().to_string());
            }
            pages.scroller.grab_focus();
        }));
        zoom_out.connect_clicked(glib::clone!(#[weak] pages, move |_| pages.set_zoom(pages.zoom.get() - ZOOM_STEP)));
        zoom_in.connect_clicked(glib::clone!(#[weak] pages, move |_| pages.set_zoom(pages.zoom.get() + ZOOM_STEP)));
        fit_w.connect_clicked(glib::clone!(#[weak] pages, move |_| pages.set_fit(Fit::Width)));
        fit_p.connect_clicked(glib::clone!(#[weak] pages, move |_| pages.set_fit(Fit::Page)));
        pages.zoom_label.set_tooltip_text(Some("100% spans the column"));
        pages.search_entry.connect_activate(glib::clone!(#[weak] pages, move |_| pages.search()));
        pages.search_entry.connect_search_changed(glib::clone!(#[weak] pages, move |e| {
            // Emptying the box takes the hit bar with it.
            if e.text().trim().chars().count() < 2 {
                pages.search_generation.fetch_add(1, Ordering::Relaxed);
                pages.hits_bar.set_visible(false);
                pages.hits_label.set_label("");
            }
        }));

        // Keys on the scroller, captured: without this GTK's own bindings
        // scroll by a viewport and land between pages.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(#[weak] pages, #[upgrade_or] glib::Propagation::Proceed, move |_, key, _, state| {
            use gtk::gdk::Key;
            let ctrl = state.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            match key {
                Key::Page_Down | Key::Right | Key::space => pages.scroll_to_page(pages.current.get() + 1),
                Key::Page_Up | Key::Left | Key::BackSpace => pages.scroll_to_page(pages.current.get() - 1),
                Key::Home => pages.scroll_to_page(1),
                Key::End => pages.scroll_to_page(pages.n_pages),
                Key::plus | Key::equal | Key::KP_Add => pages.set_zoom(pages.zoom.get() + ZOOM_STEP),
                Key::minus | Key::KP_Subtract => pages.set_zoom(pages.zoom.get() - ZOOM_STEP),
                Key::_0 | Key::KP_0 if ctrl => pages.set_fit(Fit::Width),
                Key::f if ctrl => {
                    pages.search_entry.grab_focus();
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }));
        pages.scroller.add_controller(keys);
        // Ctrl+F from anywhere in the viewer.
        let root_keys = gtk::EventControllerKey::new();
        root_keys.connect_key_pressed(glib::clone!(#[weak] pages, #[upgrade_or] glib::Propagation::Proceed, move |_, key, _, state| {
            if key == gtk::gdk::Key::f && state.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                pages.search_entry.grab_focus();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }));
        self.widget.add_controller(root_keys);
        // Ctrl+wheel zooms.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        scroll.connect_scroll(glib::clone!(#[weak] pages, #[upgrade_or] glib::Propagation::Proceed, move |c, _, dy| {
            if !c.current_event_state().contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            pages.set_zoom(pages.zoom.get() - dy.signum() * ZOOM_STEP);
            glib::Propagation::Stop
        }));
        pages.scroller.add_controller(scroll);
        (pages, root)
    }

    fn open_image(self: &Rc<Self>, body: &gtk::Stack, status: &gtk::Label, spinner: &adw::Spinner) {
        let path = std::path::PathBuf::from(&self.path);
        let handle = async_runtime::spawn_blocking(move || covers::load_scaled(&path, covers::Size::Fit(4096, 4096)));
        let (body, status, spinner) = (body.clone(), status.clone(), spinner.clone());
        app::local(async move {
            match handle.await.ok().flatten() {
                Some(texture) => {
                    let picture = gtk::Picture::builder().paintable(&texture).can_shrink(true).content_fit(gtk::ContentFit::Contain).hexpand(true).vexpand(true).css_classes(["doc-image"]).build();
                    let scroller = gtk::ScrolledWindow::builder().child(&picture).vexpand(true).hexpand(true).build();
                    body.add_named(&scroller, Some("content"));
                    body.set_visible_child_name("content");
                }
                None => Self::show_error(&body, &status, &spinner, "Could not open this image."),
            }
        });
    }

    fn open_text(self: &Rc<Self>, body: &gtk::Stack, status: &gtk::Label, spinner: &adw::Spinner, html: bool, zoom_box: &gtk::Box) {
        let path = self.path.clone();
        let handle = async_runtime::spawn_blocking(move || std::fs::read(&path).map(|b| String::from_utf8_lossy(&b).into_owned()));
        let (body, status, spinner, zoom_box) = (body.clone(), status.clone(), spinner.clone(), zoom_box.clone());
        app::local(async move {
            let text = match handle.await {
                Ok(Ok(t)) => t,
                _ => {
                    Self::show_error(&body, &status, &spinner, "Failed to load this document.");
                    return;
                }
            };
            let text = if html { html_to_text(&text) } else { text };
            let buffer = gtk::TextBuffer::new(None);
            buffer.set_text(&text);
            let tag = gtk::TextTag::builder().name("zoom").scale(1.0).build();
            buffer.tag_table().add(&tag);
            buffer.apply_tag(&tag, &buffer.start_iter(), &buffer.end_iter());
            let view = gtk::TextView::builder()
                .buffer(&buffer)
                .editable(false)
                .cursor_visible(false)
                .monospace(!html)
                .wrap_mode(if html { gtk::WrapMode::WordChar } else { gtk::WrapMode::None })
                .left_margin(24)
                .right_margin(24)
                .top_margin(16)
                .bottom_margin(24)
                .css_classes(["doc-text"])
                .build();
            let scroller = gtk::ScrolledWindow::builder().child(&view).vexpand(true).hexpand(true).build();
            body.add_named(&scroller, Some("content"));
            body.set_visible_child_name("content");

            // Zoom for the text kinds: the tag's scale, live.
            let scale = Rc::new(Cell::new(1.0_f64));
            let pct = gtk::Button::builder().label("100%").css_classes(["btn", "small"]).tooltip_text("Reset zoom").build();
            let out = gtk::Button::builder().label("−").css_classes(["btn", "icon"]).tooltip_text("Zoom out").build();
            let inn = gtk::Button::builder().label("+").css_classes(["btn", "icon"]).tooltip_text("Zoom in").build();
            let apply = {
                let (tag, pct, scale) = (tag.clone(), pct.clone(), scale.clone());
                Rc::new(move |z: f64| {
                    let z = z.clamp(0.5, 3.0);
                    scale.set(z);
                    tag.set_scale(z);
                    pct.set_label(&format!("{}%", (z * 100.0).round() as i32));
                })
            };
            let a = apply.clone();
            let s = scale.clone();
            out.connect_clicked(move |_| a(s.get() - 0.25));
            let a = apply.clone();
            let s = scale.clone();
            inn.connect_clicked(move |_| a(s.get() + 0.25));
            let a = apply.clone();
            pct.connect_clicked(move |_| a(1.0));
            zoom_box.append(&out);
            zoom_box.append(&pct);
            zoom_box.append(&inn);
            zoom_box.set_visible(true);
        });
    }
}

/// An unknown extension: a PDF by its magic, else text.
fn sniff_kind(path: &str) -> Kind {
    let mut head = [0u8; 5];
    let is_pdf = std::fs::File::open(path)
        .and_then(|mut f| {
            use std::io::Read;
            f.read_exact(&mut head)
        })
        .map(|_| &head == b"%PDF-")
        .unwrap_or(false);
    if is_pdf {
        Kind::Pdf
    } else {
        Kind::Text
    }
}

/// Present a viewer (or any reader body) as a full-size in-window dialog.
/// The `holder` keeps the caller's state alive until the dialog closes.
pub fn present(parent: &gtk::Window, child: &impl IsA<gtk::Widget>, title: &str, holder: Box<dyn std::any::Any>) -> adw::Dialog {
    let dialog = adw::Dialog::builder()
        .title(title)
        .child(child)
        .content_width(4000)
        .content_height(3000)
        .follows_content_size(false)
        .presentation_mode(adw::DialogPresentationMode::Floating)
        .css_classes(["doc-dialog"])
        .build();
    let holder = RefCell::new(Some(holder));
    dialog.connect_closed(move |_| {
        holder.take();
    });
    dialog.present(Some(parent));
    dialog
}

/// Open `path` in the viewer: an `adw::Dialog` filling the window (no
/// second toplevel). `on_page` reports 1-based page changes; `start_page`
/// is where a PDF opens.
pub fn open_document_viewer(parent: &gtk::Window, path: &str, title: &str, on_page: Option<PageCb>, start_page: u32) {
    let kind = kind_of(path);
    let subtitle = format!("Manual · {}", kind_label(kind));
    let view = DocumentView::new(path, title, Some(&subtitle), on_page, start_page);
    let dialog = present(parent, &view.widget, title, Box::new(view.clone()));
    view.connect_close(move || {
        dialog.close();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_come_from_the_extension() {
        assert_eq!(kind_of("/x/manual.PDF"), Kind::Pdf);
        assert_eq!(kind_of("/x/cover.jpg"), Kind::Image);
        assert_eq!(kind_of("/x/readme.txt"), Kind::Text);
        assert_eq!(kind_of("/x/manual.htm"), Kind::Html);
        assert_eq!(kind_of("/x/noext"), Kind::Unknown);
        assert_eq!(kind_label(Kind::Html), "HTML");
    }

    #[test]
    fn eviction_spares_visible_and_recent_pages() {
        let rendered = [1, 2, 3, 4, 5, 6, 7, 8];
        let visible: HashSet<i32> = [4, 5].into_iter().collect();
        let recent = [5, 4, 3, 6, 2];
        let out = evictable(&rendered, &visible, &recent, 6);
        assert_eq!(out.len(), 2);
        assert!(out.contains(&1) || out.contains(&7) || out.contains(&8));
        assert!(!out.contains(&4) && !out.contains(&5));
        assert!(evictable(&[1, 2], &visible, &recent, 6).is_empty());
    }

    #[test]
    fn render_order_is_nearest_first_below_before_above() {
        let wanted = [1, 2, 3, 4, 5, 6];
        let drawn: HashSet<i32> = [3].into_iter().collect();
        let in_flight: HashSet<i32> = [4].into_iter().collect();
        assert_eq!(next_to_render(&wanted, &drawn, &in_flight, 3, 2), [2, 5]);
        assert_eq!(next_to_render(&wanted, &drawn, &in_flight, 3, 0), Vec::<i32>::new());
        assert_eq!(next_to_render(&[3, 4, 5], &HashSet::new(), &HashSet::new(), 4, 3), [4, 5, 3]);
    }

    #[test]
    fn html_becomes_readable_text() {
        let html = "<html><head><title>Manual</title><style>p{}</style></head><body><h1>Play &amp; Win</h1><p>Press <b>F1</b>.<br>Then &lt;Esc&gt;.</p><script>x()</script><ul><li>one</li><li>two</li></ul></body></html>";
        let text = html_to_text(html);
        assert_eq!(text, "Manual\n\nPlay & Win\n\nPress F1.\nThen <Esc>.\n\none\n\ntwo");
        assert_eq!(decode_entities("a &#65;&#x42; &unknown; & b"), "a AB &unknown; & b");
    }
}
