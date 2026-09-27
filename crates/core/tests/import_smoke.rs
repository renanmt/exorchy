//! CI smoke test: the bundled metadata must import fully. Guards against a
//! corrupted .xml.gz, a parser regression, or a catalog accidentally shipped
//! half-empty - all of which would otherwise only surface on a user's first
//! run.

use std::io::BufReader;
use std::path::Path;

use exorchy_core::import::xml::parse_games_xml;

fn metadata_path(file: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("metadata")
        .join(file)
}

fn parse_bundled(file: &str, shortcode_segment: &str) -> Vec<exorchy_core::models::Game> {
    let path = metadata_path(file);
    let f = std::fs::File::open(&path)
        .unwrap_or_else(|e| panic!("bundled metadata missing: {}: {}", path.display(), e));
    parse_games_xml(
        BufReader::new(flate2::read::GzDecoder::new(f)),
        shortcode_segment,
    )
    .unwrap_or_else(|e| panic!("failed to parse {}: {}", path.display(), e))
}

#[test]
fn bundled_en_catalog_imports_fully() {
    let games = parse_bundled("MS-DOS.xml.gz", "!dos");
    assert!(
        games.len() >= 7600,
        "expected >= 7600 EN games, got {}",
        games.len()
    );
    let with_path = games
        .iter()
        .filter(|g| g.application_path.as_deref().is_some_and(|p| !p.is_empty()))
        .count();
    assert!(
        with_path * 100 / games.len() >= 99,
        "only {}/{} games have an application_path",
        with_path,
        games.len()
    );
}

#[test]
fn bundled_lp_catalogs_import() {
    for (file, min_games) in [
        ("GLP.xml.gz", 600),
        ("SLP.xml.gz", 600),
        ("PLP.xml.gz", 200),
    ] {
        let games = parse_bundled(file, "!dos");
        assert!(
            games.len() >= min_games,
            "{}: expected >= {} games, got {}",
            file,
            min_games,
            games.len()
        );
    }
}
