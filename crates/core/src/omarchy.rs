//! The Omarchy theme bridge.
//!
//! Omarchy keeps the active theme at `~/.local/state/omarchy/current/theme/`
//! and swaps that directory atomically on `omarchy theme set`. Exorchy reads
//! `colors.toml` (the palette) and `shell.toml` (`[font] base-size`) from it,
//! watches the `current/` directory with inotify and pushes every change to
//! the webview as a `theme-changed` event. The frontend maps the palette to
//! CSS custom properties. Without Omarchy the built-in fallback palette is
//! used, so the app still runs on any Linux desktop.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use crate::host::AppHandle;

/// The palette keys Omarchy's `colors.toml` defines (see
/// `/usr/share/omarchy/themes/<name>/colors.toml`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Palette {
    pub mode: String,
    pub accent: String,
    pub selection: String,
    pub muted: String,
    pub background: String,
    pub dark_background: String,
    pub darker_background: String,
    pub lighter_background: String,
    pub foreground: String,
    pub dark_foreground: String,
    pub light_foreground: String,
    pub bright_foreground: String,
    pub red: String,
    pub yellow: String,
    pub orange: String,
    pub green: String,
    pub cyan: String,
    pub blue: String,
    pub magenta: String,
    pub brown: String,
    pub bright_red: String,
    pub bright_yellow: String,
    pub bright_green: String,
    pub bright_cyan: String,
    pub bright_blue: String,
    pub bright_magenta: String,
}

impl Default for Palette {
    /// Tokyo Night, Omarchy's default theme: what the app looks like when no
    /// Omarchy theme can be read.
    fn default() -> Self {
        Palette {
            mode: "dark".into(),
            accent: "#7aa2f7".into(),
            selection: "#292e42".into(),
            muted: "#414868".into(),
            background: "#1a1b26".into(),
            dark_background: "#13141c".into(),
            darker_background: "#0e0e14".into(),
            lighter_background: "#24283b".into(),
            foreground: "#a9b1d6".into(),
            dark_foreground: "#565f89".into(),
            light_foreground: "#b4bee6".into(),
            bright_foreground: "#c0caf5".into(),
            red: "#f7768e".into(),
            yellow: "#e0af68".into(),
            orange: "#eb927b".into(),
            green: "#9ece6a".into(),
            cyan: "#449dab".into(),
            blue: "#7aa2f7".into(),
            magenta: "#ad8ee6".into(),
            brown: "#75493d".into(),
            bright_red: "#ff7a93".into(),
            bright_yellow: "#ff9e64".into(),
            bright_green: "#b9f27c".into(),
            bright_cyan: "#0db9d7".into(),
            bright_blue: "#7da6ff".into(),
            bright_magenta: "#bb9af7".into(),
        }
    }
}

/// What the frontend receives: the palette, where it came from, and the
/// shell's typographic base.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    /// Omarchy's theme slug (`theme.name`), or `None` for the fallback.
    pub name: Option<String>,
    /// "omarchy" when read from the current theme dir, "fallback" otherwise.
    pub source: String,
    pub palette: Palette,
    /// `[font] base-size` from `shell.toml`, in px.
    pub font_base_size: u32,
    /// The monospace family fontconfig resolves (what `omarchy font current` prints).
    pub mono_font: Option<String>,
}

/// `~/.local/state/omarchy/current`.
pub fn omarchy_current_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    Some(base.join("omarchy").join("current"))
}

/// The palette a `colors.toml` describes, resolved the way Omarchy's own
/// `omarchy-theme-color` does, so eXorchy agrees with the rest of the desktop.
/// Themes made before Omarchy's named palette (Aether, older user themes)
/// only define `accent`, `foreground`, `background`, `selection_*` and the
/// ANSI `color0`..`color15`; the named keys are derived from those instead
/// of falling back to Tokyo Night piecemeal. `light_mode_file` is the
/// theme's `light.mode` marker.
fn parse_palette(text: &str, light_mode_file: bool) -> Result<Palette, toml::de::Error> {
    let table: toml::Table = toml::from_str(text)?;
    let mut c: std::collections::HashMap<String, String> = table
        .into_iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k, s.trim().to_string())))
        .filter(|(_, v)| !v.is_empty())
        .collect();
    resolve_palette(&mut c, light_mode_file);
    let fallback = Palette::default();
    let get = |k: &str, d: &str| c.get(k).cloned().unwrap_or_else(|| d.to_string());
    Ok(Palette {
        mode: get("mode", &fallback.mode),
        // Omarchy leaves a missing accent unset; the theme's own blue beats
        // Tokyo Night's.
        accent: c.get("accent").or_else(|| c.get("blue")).cloned().unwrap_or(fallback.accent),
        selection: get("selection", &fallback.selection),
        muted: get("muted", &fallback.muted),
        background: get("background", &fallback.background),
        dark_background: get("dark_background", &fallback.dark_background),
        darker_background: get("darker_background", &fallback.darker_background),
        lighter_background: get("lighter_background", &fallback.lighter_background),
        foreground: get("foreground", &fallback.foreground),
        dark_foreground: get("dark_foreground", &fallback.dark_foreground),
        light_foreground: get("light_foreground", &fallback.light_foreground),
        bright_foreground: get("bright_foreground", &fallback.bright_foreground),
        red: get("red", &fallback.red),
        yellow: get("yellow", &fallback.yellow),
        orange: get("orange", &fallback.orange),
        green: get("green", &fallback.green),
        cyan: get("cyan", &fallback.cyan),
        blue: get("blue", &fallback.blue),
        magenta: get("magenta", &fallback.magenta),
        brown: get("brown", &fallback.brown),
        bright_red: get("bright_red", &fallback.bright_red),
        bright_yellow: get("bright_yellow", &fallback.bright_yellow),
        bright_green: get("bright_green", &fallback.bright_green),
        bright_cyan: get("bright_cyan", &fallback.bright_cyan),
        bright_blue: get("bright_blue", &fallback.bright_blue),
        bright_magenta: get("bright_magenta", &fallback.bright_magenta),
    })
}

/// `resolve_theme_colors` + `resolve_theme_mode` from
/// `/usr/share/omarchy/bin/omarchy-theme-color`, in the same order: a key a
/// theme defines always wins, each rule only fills what is still missing.
fn resolve_palette(c: &mut std::collections::HashMap<String, String>, light_mode_file: bool) {
    fn alias(c: &mut std::collections::HashMap<String, String>, key: &str, from: &str) {
        if !c.contains_key(key) {
            if let Some(v) = c.get(from).cloned() {
                c.insert(key.to_string(), v);
            }
        }
    }
    fn set_if_missing(c: &mut std::collections::HashMap<String, String>, key: &str, value: Option<String>) {
        if !c.contains_key(key) {
            if let Some(v) = value {
                c.insert(key.to_string(), v);
            }
        }
    }
    let first = |c: &std::collections::HashMap<String, String>, keys: &[&str]| keys.iter().find_map(|k| c.get(*k).cloned());

    // Short legacy names.
    for (key, from) in [
        ("background", "bg"),
        ("dark_background", "dark_bg"),
        ("darker_background", "darker_bg"),
        ("lighter_background", "lighter_bg"),
        ("foreground", "fg"),
        ("dark_foreground", "dark_fg"),
        ("light_foreground", "light_fg"),
        ("bright_foreground", "bright_fg"),
    ] {
        alias(c, key, from);
    }
    // ANSI-only themes.
    alias(c, "background", "color0");
    alias(c, "foreground", "color7");
    if let Some(bg) = c.get("background").cloned() {
        c.insert("color0".into(), bg);
    }
    if let Some(fg) = c.get("foreground").cloned() {
        c.insert("color7".into(), fg);
    }
    for (key, from) in [
        ("red", "color1"),
        ("green", "color2"),
        ("yellow", "color3"),
        ("blue", "color4"),
        ("magenta", "color5"),
        ("cyan", "color6"),
        ("bright_red", "color9"),
        ("bright_green", "color10"),
        ("bright_yellow", "color11"),
        ("bright_blue", "color12"),
        ("bright_magenta", "color13"),
        ("bright_cyan", "color14"),
    ] {
        alias(c, key, from);
    }
    alias(c, "magenta", "purple");
    alias(c, "bright_magenta", "bright_purple");

    let v = first(c, &["color7", "foreground"]);
    set_if_missing(c, "light_foreground", v);
    let v = first(c, &["color15", "foreground"]);
    set_if_missing(c, "bright_foreground", v);
    let v = first(c, &["color0", "background"]);
    set_if_missing(c, "lighter_background", v);
    let v = first(c, &["color8", "foreground"]);
    set_if_missing(c, "dark_foreground", v);
    let v = first(c, &["color8", "dark_foreground"]);
    set_if_missing(c, "muted", v);
    let v = first(c, &["selection_background", "color8", "color0", "background"]);
    set_if_missing(c, "selection", v);
    let v = c.get("yellow").cloned();
    set_if_missing(c, "orange", v);
    let v = c.get("orange").and_then(|o| mix(o, "#000000", 0.5));
    set_if_missing(c, "brown", v);

    let v = c.get("background").and_then(|b| mix(b, "#000000", 0.25));
    set_if_missing(c, "dark_background", v);
    let v = c.get("background").and_then(|b| mix(b, "#000000", 0.5));
    set_if_missing(c, "darker_background", v);
    for (bright, base) in [
        ("bright_red", "red"),
        ("bright_yellow", "yellow"),
        ("bright_green", "green"),
        ("bright_cyan", "cyan"),
        ("bright_blue", "blue"),
        ("bright_magenta", "magenta"),
    ] {
        let v = c.get(base).and_then(|b| mix(b, "#ffffff", 0.2));
        set_if_missing(c, bright, v);
    }

    // Mode: stated, a `light.mode` marker, else the background's brightness.
    alias(c, "mode", "theme_type");
    if !c.contains_key("mode") {
        let light = light_mode_file
            || c.get("background").and_then(|b| rgb(b)).is_some_and(|(r, g, b)| r as u32 + g as u32 + b as u32 > 382);
        c.insert("mode".into(), if light { "light" } else { "dark" }.into());
    }
}

fn rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some((p(0)?, p(2)?, p(4)?))
}

/// `mix_color`: `amount` of `end` into `start`, rounded like Omarchy's awk.
fn mix(start: &str, end: &str, amount: f64) -> Option<String> {
    let (a, b) = (rgb(start)?, rgb(end)?);
    let ch = |x: u8, y: u8| (x as f64 * (1.0 - amount) + y as f64 * amount + 0.5).floor() as u8;
    Some(format!("#{:02x}{:02x}{:02x}", ch(a.0, b.0), ch(a.1, b.1), ch(a.2, b.2)))
}

/// `[font] base-size` from shell.toml; 12 (Omarchy's default) when missing.
fn parse_font_base_size(text: &str) -> u32 {
    #[derive(Deserialize)]
    struct Shell {
        font: Option<Font>,
    }
    #[derive(Deserialize)]
    struct Font {
        #[serde(rename = "base-size")]
        base_size: Option<f64>,
    }
    toml::from_str::<Shell>(text)
        .ok()
        .and_then(|s| s.font)
        .and_then(|f| f.base_size)
        .map(|v| v.round().max(1.0) as u32)
        .unwrap_or(12)
}

/// Read the current theme from `current_dir`, or the fallback.
pub fn read_theme_from(current_dir: Option<&Path>) -> Theme {
    let mono_font = system_mono_font();
    let Some(dir) = current_dir else {
        return Theme { name: None, source: "fallback".into(), palette: Palette::default(), font_base_size: 12, mono_font };
    };
    let theme_dir = dir.join("theme");
    let colors = std::fs::read_to_string(theme_dir.join("colors.toml"));
    let light_mode_file = theme_dir.join("light.mode").exists();
    let palette = match colors.as_deref().map(|t| parse_palette(t, light_mode_file)) {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            log::warn!("Omarchy colors.toml unreadable ({e}); using the fallback palette");
            return Theme { name: None, source: "fallback".into(), palette: Palette::default(), font_base_size: 12, mono_font };
        }
        Err(_) => {
            return Theme { name: None, source: "fallback".into(), palette: Palette::default(), font_base_size: 12, mono_font };
        }
    };
    let name = std::fs::read_to_string(dir.join("theme.name")).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let font_base_size = std::fs::read_to_string(theme_dir.join("shell.toml"))
        .map(|t| parse_font_base_size(&t))
        .unwrap_or(12);
    Theme { name, source: "omarchy".into(), palette, font_base_size, mono_font }
}

/// The family fontconfig resolves for `monospace` - what `omarchy font set`
/// writes. `fc-match` is on every Omarchy install; elsewhere None.
pub fn system_mono_font() -> Option<String> {
    let out = std::process::Command::new("fc-match")
        .args(["monospace", "-f", "%{family}"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    s.split(',').next().map(|f| f.trim().to_string()).filter(|f| !f.is_empty())
}

static LAST_THEME: Mutex<Option<Theme>> = Mutex::new(None);

/// The theme as it is right now (the frontend's first paint).
pub async fn get_theme() -> Result<Theme, String> {
    Ok(read_theme_from(omarchy_current_dir().as_deref()))
}

/// The monospace font Omarchy set (Settings shows it; the UI uses it).
pub async fn get_system_font() -> Result<Option<String>, String> {
    Ok(system_mono_font())
}

fn emit_if_changed(app: &AppHandle) {
    let theme = read_theme_from(omarchy_current_dir().as_deref());
    let changed = {
        let mut last = LAST_THEME.lock().unwrap_or_else(|e| e.into_inner());
        if last.as_ref() == Some(&theme) {
            false
        } else {
            *last = Some(theme.clone());
            true
        }
    };
    if changed {
        log::info!("Omarchy theme: {} ({})", theme.name.as_deref().unwrap_or("fallback"), theme.source);
        let _ = app.emit("theme-changed", &theme);
    }
}

/// Watch `~/.local/state/omarchy/current` and re-read the theme on change.
/// `omarchy theme set` replaces the `theme` directory (rm + mv) and rewrites
/// `theme.name`, so watching the parent, non-recursively, catches both.
/// Events are debounced: one swap produces a burst.
pub fn start_theme_watcher(app: AppHandle) {
    let Some(current) = omarchy_current_dir() else {
        log::info!("No HOME; Omarchy theme watcher not started");
        return;
    };
    // Prime the cache so the first event after startup only fires on a real change.
    emit_if_changed(&app);
    if !current.is_dir() {
        log::info!("Omarchy state dir {} not found; using the fallback palette", current.display());
        return;
    }
    std::thread::Builder::new()
        .name("omarchy-theme-watch".into())
        .spawn(move || {
            use notify::{RecursiveMode, Watcher};
            let (tx, rx) = std::sync::mpsc::channel::<()>();
            let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if res.is_ok() {
                    let _ = tx.send(());
                }
            }) {
                Ok(w) => w,
                Err(e) => {
                    log::warn!("Omarchy theme watcher failed to start: {e}");
                    return;
                }
            };
            // The theme dir is replaced whole; colors.toml inside it is
            // watched too in case a tool edits it in place.
            if let Err(e) = watcher.watch(&current, RecursiveMode::NonRecursive) {
                log::warn!("Cannot watch {}: {e}", current.display());
                return;
            }
            let theme_dir = current.join("theme");
            if theme_dir.is_dir() {
                let _ = watcher.watch(&theme_dir, RecursiveMode::NonRecursive);
            }
            log::info!("Watching {} for theme changes", current.display());
            loop {
                if rx.recv().is_err() {
                    break;
                }
                // Debounce the burst, then drain what accumulated.
                std::thread::sleep(Duration::from_millis(250));
                while rx.try_recv().is_ok() {}
                emit_if_changed(&app);
                // After a swap the new theme dir has a new inode; re-arm.
                let theme_dir = current.join("theme");
                if theme_dir.is_dir() {
                    let _ = watcher.watch(&theme_dir, RecursiveMode::NonRecursive);
                }
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_omarchy_colors_file() {
        let text = r##"
mode = "dark"
accent = "#7aa2f7"
background = "#1a1b26"
foreground = "#a9b1d6"
red = "#f7768e"
"##;
        let p = parse_palette(text, false).unwrap();
        assert_eq!(p.accent, "#7aa2f7");
        assert_eq!(p.background, "#1a1b26");
        // Missing shades are derived from the theme, as Omarchy does.
        assert_eq!(p.bright_red, "#f991a5");
        assert_eq!(p.darker_background, "#0d0e13");
    }

    /// An ANSI-only theme (the pre-semantic format, e.g. aetheria from Aether):
    /// the expected values are `omarchy-theme-color --all` on that file.
    #[test]
    fn ansi_only_themes_resolve_like_omarchy() {
        let text = r##"
accent = "#BE3F50"
cursor = "#ff7f41"
foreground = "#14B9B5"
background = "#0e091d"
selection_foreground = "#0e091d"
selection_background = "#14B9B5"
color0 = "#000000"
color1 = "#c8e967"
color2 = "#E20342"
color3 = "#7cd699"
color4 = "#BE3F50"
color5 = "#9147a8"
color6 = "#FF7F41"
color7 = "#A60234"
color8 = "#c53253"
color9 = "#CE4F48"
color10 = "#f93d3b"
color11 = "#FD3E6A"
color12 = "#04C5F0"
color13 = "#6C032C"
color14 = "#ffbe74"
color15 = "#11AEB3"
"##;
        let p = parse_palette(text, false).unwrap();
        let expect = [
            (&p.mode, "dark"),
            (&p.accent, "#BE3F50"),
            (&p.background, "#0e091d"),
            (&p.dark_background, "#0b0716"),
            (&p.darker_background, "#07050f"),
            (&p.lighter_background, "#0e091d"),
            (&p.foreground, "#14B9B5"),
            (&p.dark_foreground, "#c53253"),
            (&p.light_foreground, "#14B9B5"),
            (&p.bright_foreground, "#11AEB3"),
            (&p.muted, "#c53253"),
            (&p.selection, "#14B9B5"),
            (&p.red, "#c8e967"),
            (&p.green, "#E20342"),
            (&p.yellow, "#7cd699"),
            (&p.orange, "#7cd699"),
            (&p.brown, "#3e6b4d"),
            (&p.blue, "#BE3F50"),
            (&p.magenta, "#9147a8"),
            (&p.cyan, "#FF7F41"),
            (&p.bright_red, "#CE4F48"),
            (&p.bright_green, "#f93d3b"),
            (&p.bright_blue, "#04C5F0"),
        ];
        for (got, want) in expect {
            assert_eq!(got.as_str(), want);
        }
    }

    /// Every theme on this machine, against Omarchy's own resolver. Needs
    /// Omarchy: `cargo test -p exorchy-core omarchy_parity -- --ignored`.
    #[test]
    #[ignore]
    fn omarchy_parity_on_installed_themes() {
        let home = std::env::var("HOME").unwrap();
        let mut dirs: Vec<PathBuf> = Vec::new();
        for base in ["/usr/share/omarchy/themes".to_string(), format!("{home}/.config/omarchy/themes")] {
            if let Ok(rd) = std::fs::read_dir(base) {
                dirs.extend(rd.flatten().map(|e| e.path()).filter(|d| d.join("colors.toml").is_file()));
            }
        }
        assert!(!dirs.is_empty(), "no Omarchy themes found");
        let mut mismatches = Vec::new();
        for dir in &dirs {
            let file = dir.join("colors.toml");
            let out = std::process::Command::new("omarchy-theme-color").arg("--file").arg(&file).arg("--all").output().unwrap();
            let reference: std::collections::HashMap<String, String> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|l| l.split_once('\t').map(|(k, v)| (k.to_string(), v.to_string())))
                .collect();
            let p = parse_palette(&std::fs::read_to_string(&file).unwrap(), dir.join("light.mode").exists()).unwrap();
            let ours = serde_json::to_value(&p).unwrap();
            for (key, value) in ours.as_object().unwrap() {
                let Some(want) = reference.get(key) else { continue };
                if !value.as_str().unwrap().eq_ignore_ascii_case(want) {
                    mismatches.push(format!("{}: {key} = {} (Omarchy: {want})", dir.display(), value));
                }
            }
        }
        assert!(mismatches.is_empty(), "{} themes checked, mismatches:\n{}", dirs.len(), mismatches.join("\n"));
        eprintln!("{} themes match Omarchy", dirs.len());
    }

    #[test]
    fn mode_comes_from_the_marker_or_the_background() {
        assert_eq!(parse_palette("background = \"#f0f0f0\"\n", false).unwrap().mode, "light");
        assert_eq!(parse_palette("background = \"#101010\"\n", false).unwrap().mode, "dark");
        assert_eq!(parse_palette("background = \"#101010\"\n", true).unwrap().mode, "light");
        assert_eq!(parse_palette("mode = \"dark\"\nbackground = \"#f0f0f0\"\n", true).unwrap().mode, "dark");
    }

    #[test]
    fn reads_font_base_size_from_shell_toml() {
        assert_eq!(parse_font_base_size("[font]\nbase-size = 14\n"), 14);
        assert_eq!(parse_font_base_size("[bar]\nsize-horizontal = 26\n"), 12);
        assert_eq!(parse_font_base_size("not toml at all ["), 12);
    }

    #[test]
    fn a_missing_theme_dir_yields_the_fallback() {
        let dir = std::env::temp_dir().join(format!("exorchy_theme_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let t = read_theme_from(Some(&dir));
        assert_eq!(t.source, "fallback");
        assert_eq!(t.palette, Palette::default());

        std::fs::create_dir_all(dir.join("theme")).unwrap();
        std::fs::write(dir.join("theme/colors.toml"), "mode = \"light\"\naccent = \"#123456\"\n").unwrap();
        std::fs::write(dir.join("theme.name"), "catppuccin-latte\n").unwrap();
        std::fs::write(dir.join("theme/shell.toml"), "[font]\nbase-size = 13\n").unwrap();
        let t = read_theme_from(Some(&dir));
        assert_eq!(t.source, "omarchy");
        assert_eq!(t.name.as_deref(), Some("catppuccin-latte"));
        assert_eq!(t.palette.mode, "light");
        assert_eq!(t.palette.accent, "#123456");
        assert_eq!(t.font_base_size, 13);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
