//! Pure presentation logic for the Reading Room (the web UI's
//! `stores/reading.ts` minus the fetch state): filters, sort orders,
//! sections, titles and the user-facing wording for fetch failures.

use std::cmp::Ordering;

use exorchy_core::models::{Issue, Publication};

/// The kind chips. `All` is the "Everything" chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    All,
    Magazine,
    Book,
    Catalog,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::All, Kind::Magazine, Kind::Book, Kind::Catalog];

    /// The catalogue's `kind` column value; `None` for the "Everything" chip.
    pub fn id(self) -> Option<&'static str> {
        match self {
            Kind::All => None,
            Kind::Magazine => Some("magazine"),
            Kind::Book => Some("book"),
            Kind::Catalog => Some("catalog"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::All => "Everything",
            Kind::Magazine => "Magazines",
            Kind::Book => "Books",
            Kind::Catalog => "Catalogs",
        }
    }

    pub fn from_id(id: &str) -> Kind {
        Kind::ALL.into_iter().find(|k| k.id() == Some(id)).unwrap_or(Kind::All)
    }

    /// Singular and plural noun for the results count.
    pub fn nouns(self) -> (&'static str, &'static str) {
        match self {
            Kind::All => ("document", "documents"),
            Kind::Magazine => ("issue", "issues"),
            Kind::Book => ("book", "books"),
            Kind::Catalog => ("catalog", "catalogs"),
        }
    }
}

/// The language chips. eXo files a translated series under its own name,
/// so a language is a property of the publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    All,
    En,
    De,
}

impl Language {
    pub const ALL: [Language; 3] = [Language::All, Language::En, Language::De];

    pub fn id(self) -> Option<&'static str> {
        match self {
            Language::All => None,
            Language::En => Some("EN"),
            Language::De => Some("DE"),
        }
    }

    pub fn from_id(id: &str) -> Language {
        Language::ALL.into_iter().find(|l| l.id() == Some(id)).unwrap_or(Language::All)
    }

    pub fn label(self) -> &'static str {
        match self {
            Language::All => "All",
            Language::En => "English",
            Language::De => "Deutsch",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Publication,
    Title,
    TitleDesc,
    YearDesc,
    YearAsc,
    Size,
    SizeDesc,
}

/// What the grid's sort menu offers; the list's column headers reach the rest.
pub const GRID_SORTS: [(Sort, &str); 4] = [
    (Sort::Publication, "By publication"),
    (Sort::Title, "Title A–Z"),
    (Sort::YearDesc, "Newest first"),
    (Sort::YearAsc, "Oldest first"),
];

/// A list column: label, the sort its first click applies and, when it can
/// sort both ways, the second click's.
pub struct Column {
    pub label: &'static str,
    pub asc: Option<Sort>,
    pub desc: Option<Sort>,
}

pub const LIST_COLUMNS: [Column; 6] = [
    Column { label: "Title", asc: Some(Sort::Title), desc: Some(Sort::TitleDesc) },
    Column { label: "Publication", asc: Some(Sort::Publication), desc: None },
    Column { label: "Year", asc: Some(Sort::YearAsc), desc: Some(Sort::YearDesc) },
    Column { label: "Kind", asc: None, desc: None },
    Column { label: "Size", asc: Some(Sort::Size), desc: Some(Sort::SizeDesc) },
    Column { label: "Status", asc: None, desc: None },
];

impl Column {
    /// The sort a click on this column applies given the current one.
    pub fn next_sort(&self, current: Sort) -> Option<Sort> {
        let asc = self.asc?;
        match self.desc {
            Some(desc) if current == asc => Some(desc),
            _ => Some(asc),
        }
    }

    pub fn indicator(&self, current: Sort) -> &'static str {
        if self.asc == Some(current) {
            " ▲"
        } else if self.desc.is_some() && self.desc == Some(current) {
            " ▼"
        } else {
            ""
        }
    }
}

/// Case-insensitive, numeric-aware text order (the web UI's
/// `localeCompare(..., { numeric: true, sensitivity: "base" })`).
pub fn cmp_text(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().flat_map(char::to_lowercase).peekable();
    let mut bi = b.chars().flat_map(char::to_lowercase).peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    bi.next();
                }
                let na = na.trim_start_matches('0');
                let nb = nb.trim_start_matches('0');
                let ord = na.len().cmp(&nb.len()).then_with(|| na.cmp(nb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

fn title_of(issue: &Issue) -> &str {
    issue.sort_title.as_deref().unwrap_or(&issue.title)
}

/// ISO dates compare as strings; an unknown date sorts after every known one.
fn by_date(a: &Issue, b: &Issue) -> Ordering {
    match (&a.release_date, &b.release_date) {
        (Some(x), Some(y)) => cmp_text(x, y),
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
    }
}

/// An unknown year sorts last in either direction.
fn by_year(a: &Issue, b: &Issue, desc: bool) -> Ordering {
    match (a.year, b.year) {
        (Some(x), Some(y)) => {
            if desc {
                y.cmp(&x)
            } else {
                x.cmp(&y)
            }
        }
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
    }
}

pub fn compare(a: &Issue, b: &Issue, sort: Sort) -> Ordering {
    let titles = || cmp_text(title_of(a), title_of(b));
    match sort {
        Sort::Publication => cmp_text(&a.publication, &b.publication).then_with(|| by_date(a, b)).then_with(titles),
        Sort::Title => titles().then_with(|| by_date(a, b)),
        Sort::TitleDesc => cmp_text(title_of(b), title_of(a)).then_with(|| by_date(a, b)),
        Sort::YearAsc => by_year(a, b, false).then_with(|| by_date(a, b)).then_with(titles),
        Sort::YearDesc => by_year(a, b, true).then_with(|| by_date(b, a)).then_with(titles),
        Sort::Size => a.size_bytes.cmp(&b.size_bytes).then_with(titles),
        Sort::SizeDesc => b.size_bytes.cmp(&a.size_bytes).then_with(titles),
    }
}

pub fn sort_issues(rows: &mut [Issue], sort: Sort) {
    rows.sort_by(|a, b| compare(a, b, sort));
}

/// Section label under `sort`; "" where the order has no natural grouping.
pub fn section_of(issue: &Issue, sort: Sort) -> String {
    match sort {
        Sort::Publication => issue.publication.clone(),
        Sort::Title | Sort::TitleDesc => {
            let first = title_of(issue).chars().next().map(|c| c.to_ascii_uppercase());
            match first {
                Some(c) if c.is_ascii_uppercase() => c.to_string(),
                _ => "#".to_string(),
            }
        }
        Sort::YearAsc | Sort::YearDesc => issue.year.map(|y| y.to_string()).unwrap_or_else(|| "Unknown".into()),
        Sort::Size | Sort::SizeDesc => String::new(),
    }
}

pub struct Section {
    pub label: String,
    pub issues: Vec<Issue>,
}

/// Consecutive runs of one section label, in the rows' order.
pub fn sections(rows: Vec<Issue>, sort: Sort) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for issue in rows {
        let label = section_of(&issue, sort);
        match out.last_mut() {
            Some(s) if s.label == label => s.issues.push(issue),
            _ => out.push(Section { label, issues: vec![issue] }),
        }
    }
    out
}

/// eXo titles repeat the series ("PC World: Issue 22"); with the publication
/// already in view that prefix is noise, without it the full title stays.
pub fn display_title(issue: &Issue, publication_in_view: bool) -> String {
    if !publication_in_view {
        return issue.title.clone();
    }
    let prefix = format!("{}:", issue.publication.to_lowercase());
    if !issue.title.to_lowercase().starts_with(&prefix) {
        return issue.title.clone();
    }
    let rest = issue.title[prefix.len()..].trim();
    if rest.is_empty() {
        issue.title.clone()
    } else {
        rest.to_string()
    }
}

pub fn kind_label(issue: &Issue) -> String {
    if issue.runnable {
        return "Disk magazine".into();
    }
    let mut chars = issue.kind.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

pub struct Filter<'a> {
    pub kind: Kind,
    pub language: Language,
    pub publication_id: Option<i64>,
    pub year: Option<i64>,
    pub favorites: bool,
    pub query: &'a str,
}

fn matches_query(issue: &Issue, needle: &str) -> bool {
    issue.title.to_lowercase().contains(needle)
        || issue.publication.to_lowercase().contains(needle)
        || issue.year.map(|y| y.to_string().contains(needle)).unwrap_or(false)
}

pub fn filter_issues(rows: &[Issue], filter: &Filter) -> Vec<Issue> {
    let needle = filter.query.trim().to_lowercase();
    rows.iter()
        .filter(|issue| {
            filter.kind.id().is_none_or(|k| issue.kind == k)
                && filter.language.id().is_none_or(|l| issue.language == l)
                && filter.publication_id.is_none_or(|p| issue.publication_id == p)
                && filter.year.is_none_or(|y| issue.year == Some(y))
                && (!filter.favorites || issue.favorited)
                && (needle.is_empty() || matches_query(issue, &needle))
        })
        .cloned()
        .collect()
}

/// What the Reading Room's sidebar browses by. `Status` has no page of its
/// own: its one value, "Downloaded", is a sidebar shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Types,
    Publications,
    Years,
    Languages,
    Status,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Types => "Types",
            Category::Publications => "Publications",
            Category::Years => "Years",
            Category::Languages => "Languages",
            Category::Status => "Status",
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            Category::Types => "Type",
            Category::Publications => "Publication",
            Category::Years => "Year",
            Category::Languages => "Language",
            Category::Status => "Status",
        }
    }
}

/// The "Downloaded" shortcut's value.
pub const DOWNLOADED: &str = "downloaded";

/// A category's values with their issue counts: (label, value, count), in
/// the order the values page lists them. A publication name shared by two
/// languages names its language.
pub fn facet_rows(issues: &[Issue], publications: &[Publication], category: Category) -> Vec<(String, String, usize)> {
    use std::collections::HashMap;
    match category {
        Category::Types => Kind::ALL
            .iter()
            .filter_map(|k| {
                let id = k.id()?;
                let n = issues.iter().filter(|i| i.kind == id).count();
                (n > 0).then(|| (k.label().to_string(), id.to_string(), n))
            })
            .collect(),
        Category::Languages => Language::ALL
            .iter()
            .filter_map(|l| {
                let id = l.id()?;
                let n = issues.iter().filter(|i| i.language == id).count();
                (n > 0).then(|| (l.label().to_string(), id.to_string(), n))
            })
            .collect(),
        Category::Years => {
            let mut counts: HashMap<i64, usize> = HashMap::new();
            for y in issues.iter().filter_map(|i| i.year) {
                *counts.entry(y).or_default() += 1;
            }
            let mut rows: Vec<(i64, usize)> = counts.into_iter().collect();
            rows.sort_by_key(|r| std::cmp::Reverse(r.0));
            rows.into_iter().map(|(y, n)| (y.to_string(), y.to_string(), n)).collect()
        }
        Category::Publications => {
            let mut counts: HashMap<i64, usize> = HashMap::new();
            for i in issues {
                *counts.entry(i.publication_id).or_default() += 1;
            }
            let mut names: HashMap<&str, usize> = HashMap::new();
            for p in publications {
                *names.entry(p.name.as_str()).or_default() += 1;
            }
            let mut rows: Vec<(String, String, usize)> = publications
                .iter()
                .filter_map(|p| {
                    let n = *counts.get(&p.id)?;
                    let label = if names.get(p.name.as_str()).copied().unwrap_or(0) > 1 { format!("{} ({})", p.name, p.language) } else { p.name.clone() };
                    Some((label, p.id.to_string(), n))
                })
                .collect();
            rows.sort_by(|a, b| cmp_text(&a.0, &b.0));
            rows
        }
        Category::Status => vec![("Downloaded".into(), DOWNLOADED.into(), 0)],
    }
}

/// Backend failures name torrents, archive paths and byte counts. The panel
/// says what it means for the reader; the raw text goes to the log.
pub fn failure_detail(raw: Option<&str>) -> &'static str {
    let text = raw.unwrap_or("");
    if text.contains("has not enabled") {
        "This issue belongs to a collection this install does not have."
    } else if text.to_lowercase().contains("not enough disk space") {
        "There is not enough free disk space for this issue."
    } else if text.to_lowercase().contains("timed out") {
        "No peers for this file yet. Try again in a moment."
    } else if text.contains("is not in") {
        "This issue is not in the collection's archive."
    } else {
        "Something went wrong while fetching this issue."
    }
}

/// "1,234 issues": the count with thousands separators and the kind's noun.
pub fn results_label(n: usize, kind: Kind) -> String {
    let (one, many) = kind.nouns();
    format!("{} {}", with_thousands(n), if n == 1 { one } else { many })
}

fn with_thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A jump bar of names (publications) reads left-aligned; letters and years
/// centre.
pub fn jump_bar_is_wide(labels: &[String]) -> bool {
    labels.iter().any(|l| l.chars().count() > 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(key: &str, publication: &str, title: &str, year: Option<i64>, date: Option<&str>) -> Issue {
        Issue {
            id: 0,
            key: key.into(),
            publication_id: 1,
            publication: publication.into(),
            kind: "magazine".into(),
            title: title.into(),
            sort_title: None,
            year,
            release_date: date.map(Into::into),
            publisher: None,
            developer: None,
            notes: None,
            zip_file: String::new(),
            entry_path: Some("x.pdf".into()),
            entry_kind: Some("pdf".into()),
            size_bytes: 10,
            cover_key: None,
            runnable: false,
            launch_dir: None,
            issue_dir: None,
            launch_bat: None,
            command_line: None,
            substitutions: None,
            favorited: false,
            installed: false,
            last_page: None,
            last_opened: None,
            source: "eXoMedia".into(),
            inner_zip: None,
            language: "EN".into(),
            extras_count: 0,
        }
    }

    fn publication(id: i64, name: &str, kind: &str, language: &str) -> Publication {
        Publication {
            id,
            kind: kind.into(),
            name: name.into(),
            issue_count: 1,
            first_year: None,
            last_year: None,
            cover_key: None,
            language: language.into(),
        }
    }

    #[test]
    fn text_order_is_numeric_and_case_insensitive() {
        assert_eq!(cmp_text("Issue 2", "issue 10"), Ordering::Less);
        assert_eq!(cmp_text("b", "A"), Ordering::Greater);
        assert_eq!(cmp_text("PC Zone", "pc zone"), Ordering::Equal);
        assert_eq!(cmp_text("", "a"), Ordering::Less);
    }

    #[test]
    fn publication_sort_groups_series_by_date() {
        let mut rows = vec![
            issue("c", "PC Zone", "PC Zone: Issue 2", Some(1993), Some("1993-05")),
            issue("a", "CGW", "CGW 80", Some(1991), Some("1991-03")),
            issue("b", "PC Zone", "PC Zone: Issue 1", Some(1993), Some("1993-04")),
            issue("d", "PC Zone", "PC Zone: Undated", None, None),
        ];
        sort_issues(&mut rows, Sort::Publication);
        let keys: Vec<&str> = rows.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["a", "b", "c", "d"]);
        let s = sections(rows, Sort::Publication);
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].label, "PC Zone");
        assert_eq!(s[1].issues.len(), 3);
    }

    #[test]
    fn year_sorts_put_unknown_last_both_ways() {
        let mut rows = vec![
            issue("a", "P", "A", None, None),
            issue("b", "P", "B", Some(1995), None),
            issue("c", "P", "C", Some(1990), None),
        ];
        sort_issues(&mut rows, Sort::YearDesc);
        assert_eq!(rows.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["b", "c", "a"]);
        sort_issues(&mut rows, Sort::YearAsc);
        assert_eq!(rows.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["c", "b", "a"]);
        assert_eq!(section_of(&rows[2], Sort::YearAsc), "Unknown");
        assert_eq!(section_of(&rows[0], Sort::YearAsc), "1990");
    }

    #[test]
    fn title_sections_are_letters_or_hash() {
        assert_eq!(section_of(&issue("a", "P", "Zork", None, None), Sort::Title), "Z");
        assert_eq!(section_of(&issue("a", "P", "1990 annual", None, None), Sort::Title), "#");
        assert_eq!(section_of(&issue("a", "P", "x", None, None), Sort::Size), "");
    }

    #[test]
    fn display_title_drops_the_series_prefix_only_in_view() {
        let i = issue("a", "PC World", "PC World: Issue 22", None, None);
        assert_eq!(display_title(&i, true), "Issue 22");
        assert_eq!(display_title(&i, false), "PC World: Issue 22");
        let bare = issue("a", "PC World", "PC World:", None, None);
        assert_eq!(display_title(&bare, true), "PC World:");
    }

    #[test]
    fn filters_combine() {
        let mut de = issue("de", "PC Player", "PC Player 3", Some(1994), None);
        de.language = "DE".into();
        de.favorited = true;
        let mut book = issue("book", "Books", "Masters of Doom", Some(2003), None);
        book.kind = "book".into();
        book.publication_id = 2;
        let rows = vec![issue("en", "CGW", "CGW 80", Some(1991), None), de.clone(), book];
        let none = Filter { kind: Kind::All, language: Language::All, publication_id: None, year: None, favorites: false, query: "" };
        let f = Filter { language: Language::De, ..none };
        assert_eq!(filter_issues(&rows, &f).len(), 1);
        let f = Filter { kind: Kind::Book, query: "doom", ..none };
        assert_eq!(filter_issues(&rows, &f)[0].key, "book");
        let f = Filter { publication_id: Some(1), query: "1991", ..none };
        assert_eq!(filter_issues(&rows, &f)[0].key, "en");
        let f = Filter { favorites: true, ..none };
        assert_eq!(filter_issues(&rows, &f)[0].key, "de");
        let f = Filter { year: Some(2003), ..none };
        assert_eq!(filter_issues(&rows, &f)[0].key, "book");
    }

    #[test]
    fn sidebar_values_count_issues() {
        let mut de = issue("de", "PC Player", "PC Player 3", Some(1994), None);
        de.language = "DE".into();
        de.publication_id = 3;
        let mut book = issue("book", "Books", "Masters of Doom", Some(1994), None);
        book.kind = "book".into();
        book.publication_id = 4;
        let issues = vec![issue("a", "CGW", "CGW 80", Some(1991), None), issue("b", "CGW", "CGW 81", Some(1991), None), de, book];
        let pubs = vec![publication(1, "CGW", "magazine", "EN"), publication(3, "PC Player", "magazine", "DE"), publication(4, "Books", "book", "EN"), publication(9, "Empty", "magazine", "EN")];
        assert_eq!(facet_rows(&issues, &pubs, Category::Types), [("Magazines".to_string(), "magazine".to_string(), 3), ("Books".into(), "book".into(), 1)]);
        assert_eq!(facet_rows(&issues, &pubs, Category::Years), [("1994".to_string(), "1994".to_string(), 2), ("1991".into(), "1991".into(), 2)]);
        assert_eq!(facet_rows(&issues, &pubs, Category::Languages)[1], ("Deutsch".to_string(), "DE".to_string(), 1));
        let p = facet_rows(&issues, &pubs, Category::Publications);
        assert_eq!(p.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["Books", "CGW", "PC Player"]);
        assert_eq!(p[1], ("CGW".to_string(), "1".to_string(), 2));
        let twins = vec![publication(1, "CGW", "magazine", "EN"), publication(2, "CGW", "magazine", "DE")];
        assert_eq!(facet_rows(&issues, &twins, Category::Publications)[0].0, "CGW (EN)");
        assert_eq!(Kind::from_id("book"), Kind::Book);
        assert_eq!(Language::from_id("XX"), Language::All);
    }

    #[test]
    fn column_clicks_toggle_direction() {
        let title = &LIST_COLUMNS[0];
        assert_eq!(title.next_sort(Sort::Publication), Some(Sort::Title));
        assert_eq!(title.next_sort(Sort::Title), Some(Sort::TitleDesc));
        assert_eq!(title.next_sort(Sort::TitleDesc), Some(Sort::Title));
        assert_eq!(LIST_COLUMNS[1].next_sort(Sort::Publication), Some(Sort::Publication));
        assert_eq!(LIST_COLUMNS[3].next_sort(Sort::Title), None);
        assert_eq!(title.indicator(Sort::TitleDesc), " ▼");
        assert_eq!(LIST_COLUMNS[1].indicator(Sort::Publication), " ▲");
    }

    #[test]
    fn failure_wording_and_counts() {
        assert!(failure_detail(Some("x is in the eXoMedia torrent, which this install has not enabled")).contains("collection"));
        assert!(failure_detail(Some("Not enough disk space: ...")).contains("disk space"));
        assert!(failure_detail(Some("stream timed out")).contains("peers"));
        assert!(failure_detail(None).starts_with("Something"));
        assert_eq!(results_label(1, Kind::Magazine), "1 issue");
        assert_eq!(results_label(2195, Kind::All), "2,195 documents");
        assert!(jump_bar_is_wide(&["PC Zone".into()]));
        assert!(!jump_bar_is_wide(&["1991".into(), "A".into()]));
        assert_eq!(kind_label(&issue("a", "P", "T", None, None)), "Magazine");
    }
}
