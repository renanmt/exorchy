//! Catalogue, config and launch commands. The launch pipeline itself lives
//! in `crate::launchers`; `launch_game` here is the collection-agnostic spine.

use std::sync::Mutex;

use rusqlite::Connection;
use serde::Serialize;
use crate::host::{AppHandle, State};

use crate::db::queries;
use crate::launchers::{self, game_op_lock, running_game_key, running_games, spawn_emulator_and_track, LaunchContext, LaunchOutcome};
use crate::models::Game;

use super::collections::{collection_def, Launcher};
use super::install::extract_before_launch;
use super::shell_open::open_with_default_app;
use super::TorrentState;


pub struct DbState(pub Mutex<Connection>);

impl DbState {
    /// The connection guard; a poisoned mutex surfaces as a command error.
    pub fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.0.lock().map_err(|e| e.to_string())
    }
}

/// The configured data dir; setup has not run when it is missing.
pub(crate) fn configured_data_dir(conn: &Connection) -> Result<String, String> {
    queries::get_config(conn, "data_dir")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Data directory not configured. Run setup first.".to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct GameList {
    pub games: Vec<Game>,
    pub total: usize,
}

#[allow(clippy::too_many_arguments)]
pub async fn get_games(
    state: State<'_, DbState>,
    page: Option<usize>,
    per_page: Option<usize>,
    query: Option<String>,
    genre: Option<String>,
    sort_by: Option<String>,
    collection: Option<String>,
    favorites_only: Option<bool>,
    playlist_id: Option<i64>,
    with_music: Option<bool>,
) -> Result<GameList, String> {
    let conn = state.lock()?;
    let page = page.unwrap_or(1);
    let per_page = per_page.unwrap_or(50).min(10000);
    let query = query.unwrap_or_default();
    let genre = genre.unwrap_or_default();
    let sort_by = sort_by.unwrap_or_default();
    let collection = collection.unwrap_or_default();

    let f = queries::GameFilter {
        query: &query,
        genre: &genre,
        sort_by: &sort_by,
        collection: &collection,
        favorites_only: favorites_only.unwrap_or(false),
        playlist_id,
        with_music: with_music.unwrap_or(false),
        browse: Default::default(),
    };

    let total = queries::count_games_filtered(&conn, &f).map_err(|e| e.to_string())?;
    let games = queries::fetch_games_filtered(&conn, page, per_page, &f).map_err(|e| e.to_string())?;

    Ok(GameList { games, total })
}

/// Everything the library's grid asks with: the classic filters plus the
/// sidebar's browse-by value and the filter bar's year / region.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct GameQuery {
    pub query: String,
    pub genre: String,
    pub sort_by: String,
    pub collection: String,
    pub favorites_only: bool,
    pub playlist_id: Option<i64>,
    pub with_music: bool,
    pub browse: queries::BrowseFilter,
}

impl GameQuery {
    fn filter(&self) -> queries::GameFilter<'_> {
        queries::GameFilter {
            query: &self.query,
            genre: &self.genre,
            sort_by: &self.sort_by,
            collection: &self.collection,
            favorites_only: self.favorites_only,
            playlist_id: self.playlist_id,
            with_music: self.with_music,
            browse: self.browse.clone(),
        }
    }
}

/// `get_games` with the full `GameQuery`.
pub async fn get_games_browse(state: State<'_, DbState>, page: usize, per_page: usize, q: GameQuery) -> Result<GameList, String> {
    let conn = state.lock()?;
    let f = q.filter();
    let total = queries::count_games_filtered(&conn, &f).map_err(|e| e.to_string())?;
    let games = queries::fetch_games_filtered(&conn, page.max(1), per_page.min(10000), &f).map_err(|e| e.to_string())?;
    Ok(GameList { games, total })
}

/// `get_section_keys` with the full `GameQuery`.
pub async fn get_section_keys_browse(state: State<'_, DbState>, q: GameQuery) -> Result<Vec<String>, String> {
    let conn = state.lock()?;
    queries::get_section_keys(&conn, &q.filter()).map_err(|e| e.to_string())
}

/// A browse category's values with counts, for the sidebar.
pub async fn get_facet_values(state: State<'_, DbState>, facet: String) -> Result<Vec<queries::FacetValue>, String> {
    let conn = state.lock()?;
    queries::facet_values(&conn, &facet).map_err(|e| e.to_string())
}

pub async fn get_genres(state: State<'_, DbState>, collection: Option<String>) -> Result<Vec<String>, String> {
    let conn = state.lock()?;
    let collection = collection.unwrap_or_default();
    queries::get_genres(&conn, &collection).map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
pub async fn get_section_keys(
    state: State<'_, DbState>,
    sort_by: Option<String>,
    query: Option<String>,
    genre: Option<String>,
    collection: Option<String>,
    favorites_only: Option<bool>,
    playlist_id: Option<i64>,
    with_music: Option<bool>,
) -> Result<Vec<String>, String> {
    let conn = state.lock()?;
    let sort_by = sort_by.unwrap_or_default();
    let query = query.unwrap_or_default();
    let genre = genre.unwrap_or_default();
    let collection = collection.unwrap_or_default();
    let f = queries::GameFilter {
        query: &query,
        genre: &genre,
        sort_by: &sort_by,
        collection: &collection,
        favorites_only: favorites_only.unwrap_or(false),
        playlist_id,
        with_music: with_music.unwrap_or(false),
        browse: Default::default(),
    };
    let result = queries::get_section_keys(&conn, &f).map_err(|e| e.to_string());
    log::debug!("get_section_keys: sort_by={:?} collection={:?} → {:?} keys", sort_by, collection, result.as_ref().map(|v| v.len()));
    result
}

pub async fn get_game_variants(
    state: State<'_, DbState>,
    torrent_state: State<'_, TorrentState>,
    shortcode: String,
    collection: String,
) -> Result<Vec<Game>, String> {
    let mut games = {
        let conn = state.lock()?;
        variants_with_overlay_prices(&conn, &shortcode, &collection)?
    };
    // Bytes the session already holds are not fetched again (§6): a complete
    // archive, or the GameData a sibling variant brought. Only rows with a
    // full-length file on disk ask the ledger - `stats()` copies the whole
    // file-progress table, and most rows have a placeholder.
    let managers: std::collections::HashMap<String, std::sync::Arc<crate::torrent::manager::DownloadManager>> = {
        let guard = torrent_state.0.read().await;
        guard.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    };
    // (row, file index) pairs worth asking about, grouped per manager.
    let mut asks: std::collections::HashMap<String, Vec<(usize, usize)>> = std::collections::HashMap::new();
    for (row, game) in games.iter().enumerate() {
        if game.installed || game.download_size.is_none() {
            continue;
        }
        let Some(source) = game.torrent_source.as_deref() else { continue };
        let Some(mgr) = managers.get(source) else { continue };
        for idx in [game.game_torrent_index, game.gamedata_torrent_index].into_iter().flatten() {
            let idx = idx as usize;
            let full = mgr.index().files.get(idx).map(|f| f.size).unwrap_or(0);
            let present = mgr
                .file_output_path(idx)
                .and_then(|p| std::fs::metadata(p).ok())
                .is_some_and(|m| full > 0 && m.len() == full);
            if present {
                asks.entry(source.to_string()).or_default().push((row, idx));
            }
        }
    }
    for (source, pairs) in asks {
        let Some(mgr) = managers.get(&source) else { continue };
        let indices: Vec<usize> = pairs.iter().map(|(_, i)| *i).collect();
        let complete = mgr.files_complete(&indices).await;
        for ((row, idx), done) in pairs.into_iter().zip(complete) {
            if !done {
                continue;
            }
            let full = mgr.index().files.get(idx).map(|f| f.size).unwrap_or(0) as i64;
            if let Some(game) = games.get_mut(row) {
                game.download_size = game.download_size.map(|s| (s - full).max(0));
            }
        }
    }
    Ok(games)
}

fn variants_with_overlay_prices(
    conn: &rusqlite::Connection,
    shortcode: &str,
    collection: &str,
) -> Result<Vec<Game>, String> {
    let mut games =
        queries::fetch_game_variants(conn, shortcode, collection).map_err(|e| e.to_string())?;
    // An overlay variant is installed as English-plus-patch, so its price is
    // both archives. The stored size is the patch plus the shared GameData.
    let mut dependents_by_base: Vec<(i64, String)> = Vec::new();
    for game in &games {
        let names: Vec<String> = crate::commands::lp_overlay::dependents_of(conn, game, true)
            .into_iter()
            .map(|g| g.title)
            .collect();
        if let (Some(id), false) = (game.id, names.is_empty()) {
            dependents_by_base.push((id, names.join(", ")));
        }
    }
    // Where the English trees live, so an already-installed base is not
    // charged again.
    let root = queries::get_config(conn, "data_dir")
        .ok()
        .flatten()
        .map(|d| crate::commands::paths::game_root(&d));
    for game in &mut games {
        if let Some(base) = crate::commands::lp_overlay::base_for(conn, game) {
            let on_disk = root
                .as_ref()
                .and_then(|r| crate::commands::lp_overlay::base_game_dir(r, &base))
                .is_some();
            // Only what this click would actually fetch: the patch alone once
            // the English game is there, both together while it is not.
            if !on_disk {
                game.requires_base = true;
                if let Some(size) = crate::commands::lp_overlay::archive_size_of(&base) {
                    game.download_size = Some(game.download_size.unwrap_or(0) + size as i64);
                }
            }
        }
        game.installed_with = dependents_by_base
            .iter()
            .find(|(id, _)| Some(*id) == game.id)
            .map(|(_, names)| names.clone());
    }
    Ok(games)
}

pub async fn get_installed_games(state: State<'_, DbState>) -> Result<Vec<Game>, String> {
    let conn = state.lock()?;
    queries::fetch_installed_games(&conn).map_err(|e| e.to_string())
}

pub async fn toggle_favorite(state: State<'_, DbState>, id: i64) -> Result<bool, String> {
    let conn = state.lock()?;
    queries::toggle_favorite(&conn, id).map_err(|e| e.to_string())
}

/// Hide a title from Browse and the shelves (an installed one stays
/// findable by name and playable).
pub async fn hide_game(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let conn = state.lock()?;
    queries::hide_game(&conn, id).map_err(|e| e.to_string())
}

pub async fn unhide_game(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let conn = state.lock()?;
    queries::unhide_game(&conn, id).map_err(|e| e.to_string())
}

pub async fn get_hidden_games(state: State<'_, DbState>) -> Result<Vec<Game>, String> {
    let conn = state.lock()?;
    queries::fetch_hidden_games(&conn).map_err(|e| e.to_string())
}

pub async fn get_hidden_ids(state: State<'_, DbState>) -> Result<Vec<i64>, String> {
    let conn = state.lock()?;
    queries::hidden_ids(&conn).map_err(|e| e.to_string())
}

/// My Library's search: installed games only (hidden ones included).
pub async fn search_library(state: State<'_, DbState>, query: String) -> Result<Vec<Game>, String> {
    let conn = state.lock()?;
    queries::search_library(&conn, query.trim(), 200).map_err(|e| e.to_string())
}

/// Installed games no shelf lists (hidden, or adult while those are off).
pub async fn count_hidden_installed(state: State<'_, DbState>) -> Result<usize, String> {
    let conn = state.lock()?;
    queries::count_hidden_installed(&conn).map_err(|e| e.to_string())
}

/// Take a game out of Recently played (the play history, not the game).
pub async fn remove_from_recently_played(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let conn = state.lock()?;
    queries::clear_last_played(&conn, id).map_err(|e| e.to_string())
}

pub async fn get_show_adult() -> Result<bool, String> {
    Ok(queries::show_adult())
}

/// Switch adult titles on or off everywhere (stored, then mirrored for the
/// catalogue queries).
pub async fn set_show_adult(state: State<'_, DbState>, on: bool) -> Result<(), String> {
    let conn = state.lock()?;
    queries::set_config(&conn, "show_adult", if on { "1" } else { "0" }).map_err(|e| e.to_string())?;
    queries::set_show_adult(on);
    Ok(())
}

pub async fn get_game(state: State<'_, DbState>, id: i64) -> Result<Option<Game>, String> {
    let conn = state.lock()?;
    let game = queries::fetch_game_by_id(&conn, id).map_err(|e| e.to_string())?;
    // The panel re-reads its row through here after every library change, and
    // the language chips live on `available_languages`. `fetch_game_by_id` is
    // also the hot path of the 1 Hz download poll, so the group query is
    // attached HERE rather than inside it.
    let Some(game) = game else { return Ok(None) };
    let mut rows = vec![game];
    queries::attach_language_maps(&conn, &mut rows).map_err(|e| e.to_string())?;
    Ok(rows.pop())
}


pub async fn get_config(state: State<'_, DbState>, key: String) -> Result<Option<String>, String> {
    let conn = state.lock()?;
    queries::get_config(&conn, &key).map_err(|e| e.to_string())
}

pub async fn set_config(
    _app: crate::host::AppHandle,
    state: State<'_, DbState>,
    key: String,
    value: String,
) -> Result<(), String> {
    {
        let conn = state.lock()?;
        queries::set_config(&conn, &key, &value).map_err(|e| e.to_string())?;
        if key == "collections" {
            queries::load_enabled_collections(&conn);
        }
    }
    Ok(())
}

/// Open a document (manual, magazine issue) in the system viewer. Only paths
/// under the data dir are allowed; the webview has no opener capability of
/// its own.
pub async fn open_document(
    app: AppHandle,
    state: State<'_, DbState>,
    path: String,
) -> Result<(), String> {
    let data_dir = {
        let conn = state.lock()?;
        configured_data_dir(&conn)?
    };
    let canonical = std::fs::canonicalize(&path)
        .map_err(|e| format!("Cannot open '{}': {}", path, e))?;
    let base = std::fs::canonicalize(&data_dir).map_err(|e| e.to_string())?;
    if !canonical.starts_with(&base) {
        return Err(format!("Refusing to open path outside the data directory: {}", path));
    }
    open_with_default_app(&app, &canonical).await
}

/// Toggle seeding (uploading to the swarm). Persists the choice and applies
/// it live to the shared torrent session.
pub async fn set_seeding_enabled(
    db_state: State<'_, DbState>,
    torrent_state: State<'_, TorrentState>,
    enabled: bool,
) -> Result<(), String> {
    {
        let conn = db_state.lock()?;
        queries::set_config(&conn, "seeding_enabled", if enabled { "1" } else { "0" })
            .map_err(|e| e.to_string())?;
    }
    apply_stored_limits(&db_state, &torrent_state).await;
    Ok(())
}

/// Set the user's transfer caps in KB/s; `None` (or 0 from the UI) means
/// unlimited. Persisted and applied live.
pub async fn set_rate_limits(
    db_state: State<'_, DbState>,
    torrent_state: State<'_, TorrentState>,
    up_kbps: Option<u32>,
    down_kbps: Option<u32>,
) -> Result<(), String> {
    {
        let conn = db_state.lock()?;
        // Store "" for unlimited rather than deleting the row: the reader
        // treats unparseable and absent alike, and a present key documents
        // that the user has been here.
        let write = |key: &str, v: Option<u32>| {
            queries::set_config(&conn, key, &v.map_or(String::new(), |k| k.to_string()))
        };
        write("rate_limit_up_kbps", up_kbps).map_err(|e| e.to_string())?;
        write("rate_limit_down_kbps", down_kbps).map_err(|e| e.to_string())?;
    }
    apply_stored_limits(&db_state, &torrent_state).await;
    Ok(())
}

/// Seeding and rate caps share one librqbit knob, so both are re-applied
/// together (`DownloadManager::apply_limits`). No manager: nothing to do,
/// a new session reads the preferences itself.
async fn apply_stored_limits(db_state: &State<'_, DbState>, torrent_state: &State<'_, TorrentState>) {
    let seeding = crate::commands::setup::seeding_enabled(&db_state.0);
    let (up, down) = crate::commands::setup::rate_limits(&db_state.0);
    // All managers share one session - applying via any of them is enough.
    let mgr = { torrent_state.0.read().await.values().next().cloned() };
    if let Some(mgr) = mgr {
        mgr.apply_limits(seeding, up, down);
    }
}

/// Live transfer figures shown in the network badge.
/// End a running game's emulator, Steam-style.
pub async fn stop_game(id: i64) -> Result<(), String> {
    launchers::stop_running_game(id)
}

/// Ids of the games whose emulator is running, for the Play/Stop buttons
/// after a reload.
pub async fn running_game_ids() -> Result<Vec<i64>, String> {
    Ok(launchers::running_game_ids())
}

/// How a game's emulator is chosen, for Game Settings and the panel. Only
/// DOS and Windows 3.x games choose (`engine` is None for the rest).
#[derive(Debug, serde::Serialize)]
pub struct GameEngineInfo {
    /// eXo's DOSBox ECE build is Windows-only and never runs here.
    pub ece_available: bool,
    pub uses_ece: bool,
    /// "staging" | "dosbox-x": what the next launch uses.
    pub engine: Option<String>,
    /// eXo's pick for this game: what "eXo's choice" means.
    pub exo_engine: Option<String>,
    /// The conf uses eXo's virtual printer.
    pub prints: bool,
    /// `engine` resolves on this system right now.
    pub engine_available: bool,
    /// The emulator whose graphics filters apply ("staging" | "dosbox-x"):
    /// `engine` for DOS games, DOSBox-X for the Win9x games it runs.
    pub filter_engine: Option<String>,
}

pub async fn game_engine_info(db_state: State<'_, DbState>, id: i64) -> Result<GameEngineInfo, String> {
    let (game, data_dir, setting) = {
        let conn = db_state.lock()?;
        let game = queries::fetch_game_by_id(&conn, id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Game with id {} not found", id))?;
        let data_dir = queries::get_config(&conn, "data_dir").map_err(|e| e.to_string())?.unwrap_or_default();
        let setting = queries::get_all_game_config(&conn, id).map_err(|e| e.to_string())?.remove("engine");
        (game, data_dir, setting)
    };
    let mut info = GameEngineInfo {
        ece_available: false,
        uses_ece: false,
        engine: None,
        exo_engine: None,
        prints: false,
        engine_available: false,
        filter_engine: None,
    };
    let source = game.torrent_source.as_deref().unwrap_or("eXoDOS");
    match collection_def(source).map(|c| c.launcher) {
        Some(Launcher::DosBox) => {}
        Some(Launcher::Win9x) => {
            let variant = game.dosbox_variant.as_deref().unwrap_or("x98");
            if !variant.starts_with("86box") && variant != "pcbox" {
                info.filter_engine = Some(launchers::dosbox::DosEngine::DosboxX.key().into());
            }
            return Ok(info);
        }
        _ => return Ok(info),
    }
    tokio::task::spawn_blocking(move || {
        let prints = game
            .dosbox_conf
            .as_deref()
            .and_then(|conf| launchers::dosbox::resolve_game_conf(&data_dir, conf))
            .and_then(|(path, _)| std::fs::read_to_string(path).ok())
            .is_some_and(|text| launchers::dosbox::conf_requests_printer(&text));
        let variant = game.dosbox_variant.as_deref();
        let exo = launchers::dosbox::exo_engine(variant, prints);
        let engine = launchers::dosbox::chosen_engine(setting.as_deref(), variant, prints);
        info.prints = prints;
        info.exo_engine = Some(exo.key().into());
        info.engine = Some(engine.key().into());
        info.filter_engine = Some(engine.key().into());
        info.engine_available = match engine {
            launchers::dosbox::DosEngine::Staging => crate::emulators::resolve_dosbox_staging(&data_dir).is_some(),
            launchers::dosbox::DosEngine::DosboxX => crate::emulators::resolve_dosbox_x(&data_dir).is_some(),
        };
        info
    })
    .await
    .map_err(|e| e.to_string())
}

/// The game prints but will run under DOSBox Staging, which has no printer
/// (only when the user picked Staging over eXo's DOSBox-X). A UI note.
pub async fn game_printing_unavailable(
    db_state: State<'_, DbState>,
    id: i64,
) -> Result<bool, String> {
    let info = game_engine_info(db_state, id).await?;
    Ok(info.prints && info.engine.as_deref() == Some("staging"))
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct TransferStats {
    pub download_bps: u64,
    pub upload_bps: u64,
    /// Uploaded since the session started - librqbit keeps no lifetime total.
    pub uploaded_bytes: u64,
    /// Peers currently connected. The readout that answers "is anything
    /// happening" while the rates sit at zero: connections are a standing
    /// state, transfer is event-driven.
    pub peers: u32,
    /// False when no torrent is live anywhere - the difference between "idle"
    /// and "nothing running", which the badge shows differently.
    pub active: bool,
}

/// One row of the Transfers page: a session torrent, the collection whose
/// manager owns it (if one does), and a name to show. Every eXoDOS torrent,
/// the Media Pack included, calls itself "eXoDOS" inside, so the name comes
/// from the bundled `.torrent` it was added from.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TorrentRow {
    pub source: Option<String>,
    pub label: String,
    #[serde(flatten)]
    pub transfer: crate::torrent::manager::TorrentTransfer,
}

/// Info hash → display name for every bundled torrent: collections by their
/// display name, media sources by file name. Read once; the files ship with
/// the app.
fn bundled_torrent_labels() -> &'static Vec<(String, String)> {
    static LABELS: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    LABELS.get_or_init(|| {
        let collections = crate::COLLECTION_MAP.iter().map(|c| (c.torrent_file, c.display_name.to_string()));
        let media = crate::media_sources::MEDIA_SOURCES.iter().map(|m| (m.torrent_file, m.torrent_file.trim_end_matches(".torrent").to_string()));
        collections
            .chain(media)
            .filter_map(|(file, label)| {
                let path = crate::commands::paths::bundled_torrent_path(file).ok()?;
                let hash = crate::torrent::TorrentIndex::infohash(&path).ok()?;
                Some((hash, label))
            })
            .collect()
    })
}

/// Every torrent in the shared session with its own counters, labelled. All
/// managers share one session, so one manager lists them all.
pub async fn get_session_torrents(torrent_state: State<'_, TorrentState>) -> Result<Vec<TorrentRow>, String> {
    let managers: Vec<(String, _)> = { torrent_state.0.read().await.iter().map(|(k, m)| (k.clone(), m.clone())).collect() };
    let Some((_, first)) = managers.first() else {
        return Ok(Vec::new());
    };
    let first = first.clone();
    // `stats()` takes librqbit's locks, and the labels read files the first
    // time: both off the async workers.
    let (transfers, labels) = tokio::task::spawn_blocking(move || (first.session_torrents(), bundled_torrent_labels()))
        .await
        .map_err(|e| e.to_string())?;
    let mut rows: Vec<TorrentRow> = transfers
        .into_iter()
        .map(|t| {
            let source = managers
                .iter()
                .find(|(_, m)| m.info_hash_hex().is_some_and(|h| h.eq_ignore_ascii_case(&t.info_hash)))
                .map(|(k, _)| k.clone());
            let label = labels
                .iter()
                .find(|(h, _)| h.eq_ignore_ascii_case(&t.info_hash))
                .map(|(_, l)| l.clone())
                .or_else(|| source.clone())
                .unwrap_or_else(|| if t.name.is_empty() { "Torrent".into() } else { t.name.clone() });
            TorrentRow { source, label, transfer: t }
        })
        .collect();
    rows.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(rows)
}

/// Session-wide transfer rates: one read, and a peer serving two collections
/// is not counted twice.
pub async fn get_transfer_stats(torrent_state: State<'_, TorrentState>) -> Result<TransferStats, String> {
    let managers: Vec<_> = { torrent_state.0.read().await.values().cloned().collect() };
    let Some(first) = managers.first() else {
        return Ok(TransferStats::default());
    };
    let t = first.session_transfer();
    // Liveness is per torrent, so it still needs every manager: the session
    // exists in offline-to-online transitions before any torrent is running.
    let mut active = false;
    for mgr in &managers {
        if mgr.status().await.live {
            active = true;
            break;
        }
    }
    Ok(TransferStats {
        download_bps: t.download_bps,
        upload_bps: t.upload_bps,
        uploaded_bytes: t.uploaded_bytes,
        peers: t.peers,
        active,
    })
}

/// Per-game settings the dialog edits; each key overrides the global one.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct GameSettings {
    /// `staging` forces DOSBox Staging for an ECE game; None = eXo's choice.
    pub engine: Option<String>,
    pub glshader: Option<String>,
    /// DOSBox-X's filter (`dosbox::DOSBOX_X_FILTERS` id, or `dosbox::FILTER_NONE`).
    pub dosx_filter: Option<String>,
    pub fullscreen: Option<String>,
    pub cycles: Option<String>,
    pub custom_conf: Option<String>,
}

pub async fn get_game_settings(state: State<'_, DbState>, id: i64) -> Result<GameSettings, String> {
    let conn = state.lock()?;
    let cfg = queries::get_all_game_config(&conn, id).map_err(|e| e.to_string())?;
    Ok(GameSettings {
        engine: cfg.get("engine").cloned(),
        glshader: cfg.get("glshader").cloned(),
        dosx_filter: cfg.get("dosx_filter").cloned(),
        fullscreen: cfg.get("fullscreen").cloned(),
        cycles: cfg.get("cycles").cloned(),
        custom_conf: cfg.get("custom_conf").cloned(),
    })
}

/// Store the dialog's settings; a None (or empty) field deletes the key, so
/// the game inherits the global value again.
pub async fn set_game_settings(state: State<'_, DbState>, id: i64, settings: GameSettings) -> Result<(), String> {
    let conn = state.lock()?;
    let pairs: &[(&str, &Option<String>)] = &[
        ("engine", &settings.engine),
        ("glshader", &settings.glshader),
        ("dosx_filter", &settings.dosx_filter),
        ("fullscreen", &settings.fullscreen),
        ("cycles", &settings.cycles),
        ("custom_conf", &settings.custom_conf),
    ];
    for (key, val) in pairs {
        match val {
            Some(v) if !v.is_empty() => {
                queries::set_game_config(&conn, id, key, v).map_err(|e| e.to_string())?;
            }
            _ => {
                queries::delete_game_config(&conn, id, key).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

pub async fn get_recently_played(state: State<'_, DbState>, limit: Option<usize>) -> Result<Vec<Game>, String> {
    let conn = state.lock()?;
    queries::fetch_recently_played(&conn, limit.unwrap_or(12)).map_err(|e| e.to_string())
}

/// Launch an installed game: the collection-agnostic spine. Op-lock, DB
/// read, guards and archive extraction here; the command line comes from the
/// collection's launcher (`crate::launchers::prepare`).
pub async fn launch_game(app: AppHandle, db_state: State<'_, DbState>, id: i64) -> Result<String, String> {
    // Serialize against uninstall/download of the same game (see game_op_lock).
    let op_lock = game_op_lock(id);
    let _op_guard = op_lock.lock().await;
    // Read everything we need from the DB and drop the lock before the heavy
    // path resolution + process spawning below.
    let (game, data_dir, staging_shader, fullscreen, dosx_filter, svm_filter, per_game) = {
        let conn = db_state.lock()?;
        let game = queries::fetch_game_by_id(&conn, id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Game with id {} not found", id))?;
        let data_dir = configured_data_dir(&conn)?;
        let global_glshader = queries::get_config(&conn, "global_glshader").map_err(|e| e.to_string())?;
        let dosx_filter = queries::get_config(&conn, "dosx_filter").map_err(|e| e.to_string())?;
        let default_fullscreen = queries::get_config(&conn, "default_fullscreen")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "window".to_string());
        let svm_filter = queries::get_config(&conn, "svm_filter").map_err(|e| e.to_string())?;
        let per_game = queries::get_all_game_config(&conn, id).map_err(|e| e.to_string())?;
        // Record the launch timestamp for the "Recently played" shelf.
        if let Err(e) = queries::set_last_played(&conn, id) {
            log::warn!("Failed to update last_played for {}: {}", game.title, e);
        }
        (
            game,
            data_dir,
            launchers::dosbox::staging_shader(global_glshader.as_deref()),
            default_fullscreen == "fullscreen",
            dosx_filter,
            svm_filter,
            per_game,
        )
    };

    if !game.installed {
        return Err(format!("{} is not installed. Download it first.", game.title));
    }

    // One instance per game.
    if running_games()
        .lock()
        .map(|s| s.contains(&running_game_key(&game)))
        .unwrap_or(false)
    {
        return Err(format!("'{}' is already running.", game.title));
    }

    let source = game.torrent_source.as_deref().unwrap_or("eXoDOS");
    let kind = collection_def(source).map(|c| c.launcher).unwrap_or(Launcher::DosBox);
    let root = crate::commands::paths::game_root(&data_dir);

    // The archive becomes a game dir here for every launcher.
    if game.shortcode.as_deref().is_some_and(|s| !s.is_empty()) {
        extract_before_launch(&app, &game, id, source, &root).await?;
    }

    let ctx = LaunchContext {
        app: &app,
        game: &game,
        id,
        data_dir: &data_dir,
        root,
        fullscreen,
        staging_shader,
        dosx_filter,
        svm_filter,
        per_game: &per_game,
    };
    match launchers::prepare(kind, &ctx).await? {
        LaunchOutcome::Prepared(p) => {
            spawn_emulator_and_track(&app, p.cmd, &p.binary, &running_game_key(&game), &game.title, id)
        }
        LaunchOutcome::Launched(msg) => Ok(msg),
    }
}
