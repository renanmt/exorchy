//! Formatting helpers shared by the pages (the web UI's `util.ts`).

use gtk::glib;

/// "1.2 GB", "340 MB", "12 KB".
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".into();
    }
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// The short tag a card shows when the collection is not implied by a filter.
pub fn platform_tag(source: Option<&str>) -> Option<&'static str> {
    match source? {
        "eXoDOS" | "GLP" | "SLP" | "PLP" => Some("DOS"),
        "eXoWin3x" => Some("Win3x"),
        "eXoWin9x" => Some("Win9x"),
        "eXoScummVM" => Some("ScummVM"),
        _ => None,
    }
}

/// `available_languages` is `"EN:2,DE:0"`: language code and install state
/// (0 none, 1 in library, 2 installed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LangEntry {
    pub lang: String,
    pub state: u8,
}

pub fn parse_lang_entries(available: Option<&str>) -> Vec<LangEntry> {
    available
        .unwrap_or("")
        .split(',')
        .filter_map(|e| {
            let (lang, state) = e.split_once(':')?;
            Some(LangEntry { lang: lang.trim().to_string(), state: state.trim().parse().unwrap_or(0) })
        })
        .filter(|e| !e.lang.is_empty())
        .collect()
}


/// Escape for Pango markup.
pub fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_read_like_the_web_ui() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(29 * 1024 * 1024), "29.0 MB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024), "5.0 GB");
    }

    #[test]
    fn language_map_parses_states() {
        let v = parse_lang_entries(Some("EN:2,DE:0, PL:1"));
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], LangEntry { lang: "EN".into(), state: 2 });
        assert_eq!(v[2], LangEntry { lang: "PL".into(), state: 1 });
        assert!(parse_lang_entries(None).is_empty());
    }
}
