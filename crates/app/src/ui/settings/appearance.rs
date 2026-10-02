//! Appearance: the palette follows Omarchy's current theme (read-only: the
//! page says which one, the mode and the type settings it took), and the
//! user's own background image with its opacity (`ui::backdrop`).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use exorchy_core::commands::games;
use gtk::glib;

use super::widgets::{self, Ctx, Row};
use crate::app;
use crate::ui::{backdrop, dialogs};

pub fn build(ctx: &Ctx) -> gtk::Widget {
    let page = widgets::page("Appearance");
    let theme = crate::theme::current();
    let from_omarchy = theme.as_ref().map(|t| t.source == "omarchy").unwrap_or(false);

    let palette = widgets::group(
        "Omarchy theme",
        Some("The palette follows the desktop: switch with `omarchy theme set` and eXorchy changes with it, live."),
    );
    let name = match &theme {
        Some(t) if from_omarchy => t.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| "Built-in fallback".into()),
        _ => "Built-in fallback".into(),
    };
    palette.add(
        &Row::new("Theme")
            .value(&name)
            .hint(if from_omarchy { "Read from Omarchy's current theme." } else { "No Omarchy theme found - the built-in colours are used." })
            .widget,
    );
    let mode = theme.as_ref().map(|t| if t.palette.mode == "light" { "Light" } else { "Dark" }).unwrap_or("Dark");
    palette.add(&Row::new("Mode").value(mode).widget);
    page.add(&palette);

    let type_group = widgets::group("Type", None);
    let mono = theme.as_ref().and_then(|t| t.mono_font.clone()).unwrap_or_else(|| "System default".into());
    type_group.add(&Row::new("Monospace font").value(&mono).hint("Omarchy's terminal font, used for paths and codes.").widget);
    let size = theme.as_ref().map(|t| format!("{}px", t.font_base_size)).unwrap_or_else(|| "—".into());
    type_group.add(&Row::new("Base size").value(&size).hint("Follows Omarchy's font size setting.").widget);
    type_group.add(&interface_size_row(ctx).widget);
    page.add(&type_group);

    page.add(&background_group(ctx));
    page.upcast()
}

/// How large eXorchy draws, text and layout together (`theme::ui_scale`).
/// Sizes are taken when widgets are built, so a change restarts eXorchy.
fn interface_size_row(ctx: &Ctx) -> Row {
    let labels: Vec<&str> = crate::theme::UI_SCALES.iter().map(|(_, l)| *l).collect();
    let drop = gtk::DropDown::from_strings(&labels);
    drop.add_css_class("drop");
    let current = crate::theme::ui_scale();
    let idx = crate::theme::UI_SCALES.iter().position(|(v, _)| (v - current).abs() < 0.01)
        .or_else(|| crate::theme::UI_SCALES.iter().position(|(v, _)| *v == crate::theme::DEFAULT_UI_SCALE))
        .unwrap_or(0);
    drop.set_selected(idx as u32);
    let row = Row::new("Interface size")
        .hint("Text and layout together. Medium suits most screens; pick a smaller size on a small screen, a larger one far from it. eXorchy restarts to apply it.")
        .action(&drop);
    let window = ctx.window.clone();
    drop.connect_selected_notify(move |d| {
        let Some((scale, _)) = crate::theme::UI_SCALES.get(d.selected() as usize).copied() else { return };
        if (scale - crate::theme::ui_scale()).abs() < 0.01 {
            return;
        }
        let core = crate::app::core();
        let window = window.clone();
        crate::app::spawn(
            async move {
                exorchy_core::commands::games::set_config(core.clone(), core.state(), crate::theme::UI_SCALE_KEY.into(), scale.to_string()).await
            },
            move |r| match r {
                Ok(()) => {
                    let w = window.clone();
                    dialogs::confirm(&window, "Restart eXorchy?", "The new interface size applies after a restart.", "Restart now", false, move || {
                        crate::app::spawn(async { exorchy_core::commands::library_location::relaunch_after_exit().await }, move |r| match r {
                            Ok(()) => {
                                if let Some(app) = w.application() {
                                    app.quit();
                                }
                            }
                            Err(e) => crate::ui::bus::toast_with("Could not restart eXorchy", Some(&e), None),
                        });
                    });
                }
                Err(e) => crate::ui::bus::toast_with("Could not save the interface size", Some(&e), None),
            },
        );
    });
    row
}

/// Background image: choose / remove, and how strongly it shows.
fn background_group(ctx: &Ctx) -> adw::PreferencesGroup {
    let group = widgets::group(
        "Background",
        Some("An image of your own behind the library, blended over the theme's background. Cards and panels stay solid."),
    );
    let choose = widgets::button("Choose…");
    let remove = widgets::button("Remove");
    let image_row = Row::new("Image").value("None").action(&choose);
    image_row.add_action(&remove);
    group.add(&image_row.widget);

    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 5.0, 100.0, 5.0);
    scale.set_width_request(200);
    scale.set_valign(gtk::Align::Center);
    let opacity_row = Row::new("Opacity").value(&format!("{}%", backdrop::DEFAULT_OPACITY)).hint("Lower lets more of the theme through.").action(&scale);
    group.add(&opacity_row.widget);

    let set_image = {
        let (row, remove, opacity) = (image_row.clone(), remove.clone(), opacity_row.clone());
        Rc::new(move |path: Option<&str>| {
            let name = path.and_then(|p| std::path::Path::new(p).file_name()).map(|n| n.to_string_lossy().into_owned());
            row.set_value(name.as_deref().unwrap_or("None"));
            remove.set_sensitive(name.is_some());
            opacity.widget.set_sensitive(name.is_some());
        })
    };
    set_image(None);

    // Current values; the slider only reacts once they are in.
    let ready = Rc::new(Cell::new(false));
    {
        let (set_image, scale, opacity_row, ready) = (set_image.clone(), scale.clone(), opacity_row.clone(), ready.clone());
        let core = app::core();
        app::spawn(
            async move {
                let image = games::get_config(core.state(), "background_image".into()).await.ok().flatten();
                let opacity = games::get_config(core.state(), "background_opacity".into()).await.ok().flatten();
                (image, opacity)
            },
            move |(image, opacity)| {
                set_image(image.as_deref().filter(|p| !p.is_empty()));
                let pct = backdrop::parse_opacity(opacity.as_deref());
                scale.set_value(pct as f64);
                opacity_row.set_value(&format!("{pct}%"));
                ready.set(true);
            },
        );
    }

    // Live while dragging, saved once it settles.
    let save_seq = Rc::new(Cell::new(0u64));
    scale.connect_value_changed({
        let opacity_row = opacity_row.clone();
        move |s| {
            if !ready.get() {
                return;
            }
            let pct = s.value().round() as u32;
            opacity_row.set_value(&format!("{pct}%"));
            backdrop::set_opacity(pct);
            let seq = save_seq.get() + 1;
            save_seq.set(seq);
            let save_seq = save_seq.clone();
            glib::timeout_add_local_once(Duration::from_millis(400), move || {
                if save_seq.get() == seq {
                    backdrop::save_opacity(pct);
                }
            });
        }
    });

    choose.connect_clicked({
        let (window, row, set_image) = (ctx.window.clone(), image_row.clone(), set_image.clone());
        move |_| {
            let (row, set_image) = (row.clone(), set_image.clone());
            dialogs::pick_image(&window, "Choose a background image", move |picked| {
                let Some(path) = picked else { return };
                backdrop::choose(path, move |res| match res {
                    Ok(stored) => set_image(Some(&stored)),
                    Err(e) => row.set_error(&e),
                });
            });
        }
    });
    remove.connect_clicked({
        let (row, set_image) = (image_row.clone(), set_image.clone());
        move |_| {
            let (row, set_image) = (row.clone(), set_image.clone());
            backdrop::clear(move |res| match res {
                Ok(()) => set_image(None),
                Err(e) => row.set_error(&e),
            });
        }
    });
    group
}
