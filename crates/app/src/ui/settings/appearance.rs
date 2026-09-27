//! Appearance: read-only. The palette follows Omarchy's current theme; the
//! page says which one, the mode and the type settings it took.

use adw::prelude::*;

use super::widgets::{self, Ctx, Row};

pub fn build(_ctx: &Ctx) -> gtk::Widget {
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
    page.add(&type_group);

    page.upcast()
}
