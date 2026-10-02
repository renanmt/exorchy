//! The eXorchy wordmark from `assets/logo.txt`, drawn as pixel art in the
//! theme's colours. The art is half-block text (`▀ ▄ █`), so every character
//! cell is two square pixels stacked; each text row is its own drawing area
//! with a `r<N>` class and takes its colour from `styles/logo.css`, which
//! blends the palette tokens into the logo's top-to-bottom gradient. Drawn,
//! not typeset: no font, line height or glyph gap can break the blocks.

use gtk::prelude::*;

const ART: &str = include_str!("../../assets/logo.txt");

/// The wordmark with `unit` px per art pixel (one character is `unit` wide
/// and `2 * unit` tall). `unit` should keep `2 * unit` whole so the rows
/// butt without seams.
/// The motto beside the toolbar's wordmark, as the concept stacks its
/// "PLAY / PRESERVE / EXPLORE": small, spaced capitals, one phrase a line.
pub fn tagline() -> gtk::Label {
    gtk::Label::builder()
        .label("RETRO GAMES.\nFOREVER.")
        .xalign(0.0)
        .valign(gtk::Align::Center)
        .css_classes(["brand-tagline"])
        .build()
}

pub fn ascii(unit: f64) -> gtk::Box {
    let rows: Vec<Vec<char>> = ART.lines().map(|l| l.trim_end().chars().collect()).collect();
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let logo = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Center)
        .css_classes(["ascii-logo"])
        .accessible_role(gtk::AccessibleRole::Img)
        .build();
    logo.update_property(&[gtk::accessible::Property::Label("eXorchy")]);
    let (width, height) = ((cols as f64 * unit).ceil() as i32, (2.0 * unit).round() as i32);
    for (i, row) in rows.into_iter().enumerate() {
        let area = gtk::DrawingArea::builder()
            .content_width(width)
            .content_height(height)
            .css_classes([format!("r{i}")])
            .build();
        area.set_draw_func(move |area, cr, _, _| {
            let c = area.color();
            cr.set_source_rgba(c.red().into(), c.green().into(), c.blue().into(), c.alpha().into());
            cr.set_antialias(gtk::cairo::Antialias::None);
            for (x, ch) in row.iter().enumerate() {
                let (top, bottom) = match ch {
                    '█' => (true, true),
                    '▀' => (true, false),
                    '▄' => (false, true),
                    _ => continue,
                };
                let y = if top { 0.0 } else { unit };
                let h = if top && bottom { 2.0 * unit } else { unit };
                cr.rectangle(x as f64 * unit, y, unit, h);
            }
            let _ = cr.fill();
        });
        logo.append(&area);
    }
    logo
}
