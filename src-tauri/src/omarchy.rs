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
use tauri::{AppHandle, Emitter};

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
#[derive(Debug, Clone, Serialize, PartialEq)]
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

fn parse_palette(text: &str) -> Result<Palette, toml::de::Error> {
    toml::from_str(text)
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
    let palette = match colors.as_deref().map(parse_palette) {
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
#[tauri::command]
pub async fn get_theme() -> Result<Theme, String> {
    Ok(read_theme_from(omarchy_current_dir().as_deref()))
}

/// The monospace font Omarchy set (Settings shows it; the UI uses it).
#[tauri::command]
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
        let p = parse_palette(text).unwrap();
        assert_eq!(p.accent, "#7aa2f7");
        assert_eq!(p.background, "#1a1b26");
        // Keys the file omits keep the fallback values.
        assert_eq!(p.bright_blue, Palette::default().bright_blue);
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
