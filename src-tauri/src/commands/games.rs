//! Catalogue, config and launch commands. The launch pipeline itself lives
//! in `crate::launchers`; `launch_game` here is the collection-agnostic spine.

use std::sync::Mutex;

use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, State};

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

#[tauri::command]
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
    };

    let total = queries::count_games_filtered(&conn, &f).map_err(|e| e.to_string())?;
    let games = queries::fetch_games_filtered(&conn, page, per_page, &f).map_err(|e| e.to_string())?;

    Ok(GameList { games, total })
}

#[tauri::command]
pub async fn get_genres(state: State<'_, DbState>, collection: Option<String>) -> Result<Vec<String>, String> {
    let conn = state.lock()?;
    let collection = collection.unwrap_or_default();
    queries::get_genres(&conn, &collection).map_err(|e| e.to_string())
}

#[tauri::command]
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
    };
    let result = queries::get_section_keys(&conn, &f).map_err(|e| e.to_string());
    log::debug!("get_section_keys: sort_by={:?} collection={:?} → {:?} keys", sort_by, collection, result.as_ref().map(|v| v.len()));
    result
}

#[tauri::command]
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

#[tauri::command]
pub async fn get_installed_games(state: State<'_, DbState>) -> Result<Vec<Game>, String> {
    let conn = state.lock()?;
    queries::fetch_installed_games(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn toggle_favorite(state: State<'_, DbState>, id: i64) -> Result<bool, String> {
    let conn = state.lock()?;
    queries::toggle_favorite(&conn, id).map_err(|e| e.to_string())
}

#[tauri::command]
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


#[tauri::command]
pub async fn get_config(state: State<'_, DbState>, key: String) -> Result<Option<String>, String> {
    let conn = state.lock()?;
    queries::get_config(&conn, &key).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_config(
    app: tauri::AppHandle,
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
    // The asset-protocol scope must cover the user-chosen data dir (covers,
    // screenshots, manuals); lib.rs grants the stored value at startup.
    if key == "data_dir" {
        crate::allow_asset_dir(&app, std::path::Path::new(&value));
    }
    Ok(())
}

/// Open a document (manual, magazine issue) in the system viewer. Only paths
/// under the data dir are allowed; the webview has no opener capability of
/// its own.
#[tauri::command]
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
#[tauri::command]
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
#[tauri::command]
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
#[tauri::command]
pub async fn stop_game(id: i64) -> Result<(), String> {
    launchers::stop_running_game(id)
}

/// Ids of the games whose emulator is running, for the Play/Stop buttons
/// after a reload.
#[tauri::command]
pub async fn running_game_ids() -> Result<Vec<i64>, String> {
    Ok(launchers::running_game_ids())
}

/// The two engine facts the UI asks for. Exorchy is Linux-only, where eXo's
/// DOSBox ECE build (Windows) can never run: both answers are always false and
/// the panel labels every DOS game "DOSBox Staging".
#[derive(Debug, serde::Serialize)]
pub struct GameEngineInfo {
    pub ece_available: bool,
    pub uses_ece: bool,
}

#[tauri::command]
pub async fn game_engine_info(_id: i64) -> Result<GameEngineInfo, String> {
    Ok(GameEngineInfo { ece_available: false, uses_ece: false })
}

/// The conf requests eXo's virtual printer, which DOSBox Staging does not
/// emulate (eXo's ECE build did, Windows only). A UI note, nothing more.
#[tauri::command]
pub async fn game_printing_unavailable(
    db_state: State<'_, DbState>,
    id: i64,
) -> Result<bool, String> {
    let (dosbox_conf, data_dir) = {
        let conn = db_state.lock()?;
        let game = queries::fetch_game_by_id(&conn, id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Game with id {} not found", id))?;
        let data_dir = queries::get_config(&conn, "data_dir").map_err(|e| e.to_string())?;
        (game.dosbox_conf, data_dir)
    };
    let (Some(conf), Some(data_dir)) = (dosbox_conf, data_dir) else {
        return Ok(false);
    };
    let Some((conf_path, _)) = launchers::dosbox::resolve_game_conf(&data_dir, &conf) else {
        return Ok(false);
    };
    let Ok(text) = std::fs::read_to_string(conf_path) else {
        return Ok(false);
    };
    Ok(launchers::dosbox::conf_requests_printer(&text))
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

/// Session-wide transfer rates: one read, and a peer serving two collections
/// is not counted twice.
#[tauri::command]
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
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct GameSettings {
    /// `staging` forces DOSBox Staging for an ECE game; None = eXo's choice.
    pub engine: Option<String>,
    pub glshader: Option<String>,
    pub fullscreen: Option<String>,
    pub cycles: Option<String>,
    pub custom_conf: Option<String>,
}

#[tauri::command]
pub async fn get_game_settings(state: State<'_, DbState>, id: i64) -> Result<GameSettings, String> {
    let conn = state.lock()?;
    let cfg = queries::get_all_game_config(&conn, id).map_err(|e| e.to_string())?;
    Ok(GameSettings {
        engine: cfg.get("engine").cloned(),
        glshader: cfg.get("glshader").cloned(),
        fullscreen: cfg.get("fullscreen").cloned(),
        cycles: cfg.get("cycles").cloned(),
        custom_conf: cfg.get("custom_conf").cloned(),
    })
}

#[tauri::command]
pub async fn set_game_settings(
    state: State<'_, DbState>,
    id: i64,
    engine: Option<String>,
    glshader: Option<String>,
    fullscreen: Option<String>,
    cycles: Option<String>,
    custom_conf: Option<String>,
) -> Result<(), String> {
    let conn = state.lock()?;
    // For each key: Some(value) = set, None = delete (inherit global)
    let pairs: &[(&str, &Option<String>)] = &[
        ("engine", &engine),
        ("glshader", &glshader),
        ("fullscreen", &fullscreen),
        ("cycles", &cycles),
        ("custom_conf", &custom_conf),
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

#[tauri::command]
pub async fn get_recently_played(state: State<'_, DbState>, limit: Option<usize>) -> Result<Vec<Game>, String> {
    let conn = state.lock()?;
    queries::fetch_recently_played(&conn, limit.unwrap_or(12)).map_err(|e| e.to_string())
}

/// Launch an installed game: the collection-agnostic spine. Op-lock, DB
/// read, guards and archive extraction here; the command line comes from the
/// collection's launcher (`crate::launchers::prepare`).
#[tauri::command]
pub async fn launch_game(app: AppHandle, db_state: State<'_, DbState>, id: i64) -> Result<String, String> {
    // Serialize against uninstall/download of the same game (see game_op_lock).
    let op_lock = game_op_lock(id);
    let _op_guard = op_lock.lock().await;
    // Read everything we need from the DB and drop the lock before the heavy
    // path resolution + process spawning below.
    let (game, data_dir, crt_auto, fullscreen, per_game) = {
        let conn = db_state.lock()?;
        let game = queries::fetch_game_by_id(&conn, id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Game with id {} not found", id))?;
        let data_dir = configured_data_dir(&conn)?;
        let global_glshader = queries::get_config(&conn, "global_glshader")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "crt-auto".to_string());
        let default_fullscreen = queries::get_config(&conn, "default_fullscreen")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "window".to_string());
        let per_game = queries::get_all_game_config(&conn, id).map_err(|e| e.to_string())?;
        // Record the launch timestamp for the "Recently played" shelf.
        if let Err(e) = queries::set_last_played(&conn, id) {
            log::warn!("Failed to update last_played for {}: {}", game.title, e);
        }
        (game, data_dir, global_glshader == "crt-auto", default_fullscreen == "fullscreen", per_game)
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
        crt_auto,
        per_game: &per_game,
    };
    match launchers::prepare(kind, &ctx).await? {
        LaunchOutcome::Prepared(p) => {
            spawn_emulator_and_track(&app, p.cmd, &p.binary, &running_game_key(&game), &game.title, id)
        }
        LaunchOutcome::Launched(msg) => Ok(msg),
    }
}
