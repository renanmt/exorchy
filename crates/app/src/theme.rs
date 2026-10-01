//! Omarchy's palette as GTK CSS.
//!
//! The backend reads `~/.local/state/omarchy/current/theme/colors.toml` and
//! `shell.toml` and emits `theme-changed`. This module turns the palette into
//! CSS custom properties on `:root` (the same `--om-*` names the web UI used)
//! plus libadwaita's own variables (`--accent-bg-color`, `--window-bg-color`,
//! ...) so stock widgets follow the theme too, and swaps the provider on every
//! change. Everything else in `style.css` derives from those tokens with
//! `color-mix()`; no rule outside this file names a colour.

use std::cell::RefCell;

use exorchy_core::omarchy::{self, Theme};


/// Component rules; tokens only. One sheet per feature module so they can
/// be written independently; all are loaded at the same priority.
const STYLES: [&str; 13] = [
    include_str!("style.css"),
    include_str!("styles/dialogs.css"),
    include_str!("styles/settings.css"),
    include_str!("styles/reading.css"),
    include_str!("styles/media.css"),
    include_str!("styles/logo.css"),
    include_str!("styles/transfers.css"),
    include_str!("styles/backdrop.css"),
    include_str!("styles/updates.css"),
    include_str!("styles/library_location.css"),
    include_str!("styles/detail.css"),
    include_str!("styles/statusbar.css"),
    include_str!("styles/sidebar.css"),
];

thread_local! {
    static PROVIDER: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    static CURRENT: RefCell<Option<Theme>> = const { RefCell::new(None) };
}

/// Install the static stylesheet, apply the current theme and follow changes.
pub fn init() {
    let display = gtk::gdk::Display::default().expect("a display");
    let base = gtk::CssProvider::new();
    base.load_from_string(&STYLES.concat());
    gtk::style_context_add_provider_for_display(&display, &base, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

    apply(omarchy::read_theme_from(omarchy::omarchy_current_dir().as_deref()));
    crate::app::on_event("theme-changed", |payload| {
        match serde_json::from_value::<Theme>(payload.clone()) {
            Ok(t) => apply(t),
            Err(e) => log::warn!("theme-changed payload unreadable: {e}"),
        }
    });
}

/// The theme in effect.
pub fn current() -> Option<Theme> {
    CURRENT.with(|c| c.borrow().clone())
}

fn apply(theme: Theme) {
    let display = gtk::gdk::Display::default().expect("a display");
    let css = theme_css(&theme);
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css);
    // Above the component sheet: tokens win over any fallback it declares.
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    PROVIDER.with(|p| {
        if let Some(old) = p.borrow_mut().replace(provider) {
            gtk::style_context_remove_provider_for_display(&display, &old);
        }
    });

    let manager = adw::StyleManager::default();
    manager.set_color_scheme(if theme.palette.mode == "light" {
        adw::ColorScheme::ForceLight
    } else {
        adw::ColorScheme::ForceDark
    });

    if let Some(settings) = gtk::Settings::default() {
        // shell.toml's base-size is in px; GTK font names take points.
        let pt = (theme.font_base_size as f64 * 0.75).round().max(6.0) as u32;
        let family = theme.mono_font.clone().unwrap_or_else(|| "monospace".into());
        settings.set_gtk_font_name(Some(&format!("{family} {pt}")));
    }
    log::info!(
        "Theme applied: {} ({}), base size {}px, font {:?}",
        theme.name.as_deref().unwrap_or("fallback"),
        theme.source,
        theme.font_base_size,
        theme.mono_font
    );
    CURRENT.with(|c| *c.borrow_mut() = Some(theme));
}

/// The token layer for one theme: `--om-*` from the palette, libadwaita's
/// variables mapped onto them, and the semantic tokens the components use.
pub fn theme_css(theme: &Theme) -> String {
    let p = &theme.palette;
    let pairs: [(&str, &str); 25] = [
        ("accent", &p.accent),
        ("selection", &p.selection),
        ("muted", &p.muted),
        ("background", &p.background),
        ("dark-background", &p.dark_background),
        ("darker-background", &p.darker_background),
        ("lighter-background", &p.lighter_background),
        ("foreground", &p.foreground),
        ("dark-foreground", &p.dark_foreground),
        ("light-foreground", &p.light_foreground),
        ("bright-foreground", &p.bright_foreground),
        ("red", &p.red),
        ("yellow", &p.yellow),
        ("orange", &p.orange),
        ("green", &p.green),
        ("cyan", &p.cyan),
        ("blue", &p.blue),
        ("magenta", &p.magenta),
        ("brown", &p.brown),
        ("bright-red", &p.bright_red),
        ("bright-yellow", &p.bright_yellow),
        ("bright-green", &p.bright_green),
        ("bright-cyan", &p.bright_cyan),
        ("bright-blue", &p.bright_blue),
        ("bright-magenta", &p.bright_magenta),
    ];
    let mut css = String::from(":root {\n");
    for (k, v) in pairs {
        css.push_str(&format!("  --om-{k}: {};\n", sanitize(v)));
    }
    css.push_str(&format!("  --font-size-base: {}px;\n", theme.font_base_size));
    css.push_str(SEMANTIC);
    if p.mode == "light" {
        css.push_str(LIGHT_OVERRIDES);
    }
    css.push_str("}\n");
    css
}

/// A palette value is a `#rrggbb` hex; anything else falls back to a token
/// that always exists, so a malformed theme file cannot inject CSS.
fn sanitize(v: &str) -> &str {
    let ok = v.len() == 7 && v.starts_with('#') && v[1..].chars().all(|c| c.is_ascii_hexdigit());
    if ok {
        v
    } else {
        "var(--om-foreground)"
    }
}

/// Derived tokens. Mirrors the web UI's `main.css :root` so the two designs
/// stay one design; libadwaita's variables are mapped last.
const SEMANTIC: &str = r#"
  /* Surfaces: darker < dark < background < lighter */
  --bg-primary: var(--om-darker-background);
  --bg-secondary: var(--om-dark-background);
  --bg-card: var(--om-background);
  --bg-hover: var(--om-lighter-background);

  /* Text tiers: dark_foreground and muted pulled toward foreground for
     contrast at hint sizes, keeping primary > secondary > muted. */
  --text-primary: var(--om-foreground);
  --text-secondary: color-mix(in srgb, var(--om-dark-foreground) 50%, var(--om-foreground));
  --text-muted: color-mix(in srgb, var(--om-foreground) 50%, var(--om-muted));
  --text-strong: var(--om-bright-foreground);

  /* Accent */
  --accent: var(--om-accent);
  --accent-hover: color-mix(in srgb, var(--om-accent) 78%, var(--om-bright-foreground));
  --accent-glow: color-mix(in srgb, var(--om-accent) 15%, transparent);
  --accent-fill: color-mix(in srgb, var(--om-accent) 28%, transparent);
  --on-accent: var(--om-darker-background);

  /* Status */
  --danger: var(--om-red);
  --danger-glow: color-mix(in srgb, var(--om-red) 12%, transparent);
  --success: var(--om-green);
  --warning: var(--om-yellow);
  --info: var(--om-blue);
  --orange: var(--om-orange);
  --selection: var(--om-selection);

  /* Hairlines and hover fills: foreground over transparent. */
  --line-1: color-mix(in srgb, var(--om-foreground) 4%, transparent);
  --line-2: color-mix(in srgb, var(--om-foreground) 6%, transparent);
  --line-3: color-mix(in srgb, var(--om-foreground) 8%, transparent);
  --line-4: color-mix(in srgb, var(--om-foreground) 12%, transparent);
  --line-frame: color-mix(in srgb, var(--om-foreground) 20%, transparent);
  --fill-1: color-mix(in srgb, var(--om-foreground) 3%, transparent);
  --fill-2: color-mix(in srgb, var(--om-foreground) 6%, transparent);
  --fill-3: color-mix(in srgb, var(--om-foreground) 10%, transparent);
  --scrim: color-mix(in srgb, var(--om-darker-background) 60%, transparent);
  --shadow: color-mix(in srgb, var(--om-darker-background) 55%, transparent);

  /* libadwaita's stock widgets follow the same palette. */
  --accent-bg-color: var(--om-accent);
  --accent-fg-color: var(--om-darker-background);
  --accent-color: var(--om-accent);
  --destructive-bg-color: var(--om-red);
  --destructive-fg-color: var(--om-darker-background);
  --destructive-color: var(--om-red);
  --success-bg-color: var(--om-green);
  --success-fg-color: var(--om-darker-background);
  --success-color: var(--om-green);
  --warning-bg-color: var(--om-yellow);
  --warning-fg-color: var(--om-darker-background);
  --warning-color: var(--om-yellow);
  --error-bg-color: var(--om-red);
  --error-fg-color: var(--om-darker-background);
  --error-color: var(--om-red);
  --window-bg-color: var(--om-darker-background);
  --window-fg-color: var(--om-foreground);
  --view-bg-color: var(--om-dark-background);
  --view-fg-color: var(--om-foreground);
  --headerbar-bg-color: var(--om-darker-background);
  --headerbar-fg-color: var(--om-foreground);
  --headerbar-backdrop-color: var(--om-darker-background);
  --headerbar-shade-color: var(--line-3);
  --headerbar-darker-shade-color: var(--line-4);
  --sidebar-bg-color: var(--om-dark-background);
  --sidebar-fg-color: var(--om-foreground);
  --sidebar-backdrop-color: var(--om-dark-background);
  --sidebar-shade-color: var(--line-3);
  --sidebar-border-color: var(--line-3);
  --secondary-sidebar-bg-color: var(--om-dark-background);
  --secondary-sidebar-fg-color: var(--om-foreground);
  --secondary-sidebar-backdrop-color: var(--om-dark-background);
  --secondary-sidebar-shade-color: var(--line-3);
  --secondary-sidebar-border-color: var(--line-3);
  --card-bg-color: var(--om-background);
  --card-fg-color: var(--om-foreground);
  --card-shade-color: var(--line-3);
  --dialog-bg-color: var(--om-background);
  --dialog-fg-color: var(--om-foreground);
  --popover-bg-color: var(--om-lighter-background);
  --popover-fg-color: var(--om-foreground);
  --popover-shade-color: var(--line-3);
  --thumbnail-bg-color: var(--om-background);
  --thumbnail-fg-color: var(--om-foreground);
  --shade-color: var(--line-3);
  --scrollbar-outline-color: transparent;
"#;

/// Light themes: what must stay "dark over content" regardless of the ladder.
const LIGHT_OVERRIDES: &str = r#"
  --scrim: color-mix(in srgb, var(--om-darker-background) 72%, transparent);
  --shadow: color-mix(in srgb, var(--om-foreground) 18%, transparent);
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use exorchy_core::omarchy::Palette;

    #[test]
    fn palette_lands_as_om_tokens_and_adwaita_variables() {
        let theme = Theme {
            name: Some("ethereal".into()),
            source: "omarchy".into(),
            palette: Palette { accent: "#faa968".into(), ..Palette::default() },
            font_base_size: 14,
            mono_font: None,
        };
        let css = theme_css(&theme);
        assert!(css.contains("--om-accent: #faa968;"));
        assert!(css.contains("--accent-bg-color: var(--om-accent);"));
        assert!(css.contains("--font-size-base: 14px;"));
        assert!(!css.contains("--scrim: color-mix(in srgb, var(--om-darker-background) 72%"));
    }

    #[test]
    fn light_mode_swaps_the_scrim_and_a_bad_value_never_reaches_css() {
        let p = Palette { mode: "light".into(), red: "red; } * { color: red".into(), ..Palette::default() };
        let theme = Theme { name: None, source: "fallback".into(), palette: p, font_base_size: 12, mono_font: None };
        let css = theme_css(&theme);
        assert!(css.contains("72%"));
        assert!(css.contains("--om-red: var(--om-foreground);"));
        assert!(!css.contains("* { color"));
    }
}
