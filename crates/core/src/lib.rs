//! eXorchy core: the backend of the Omarchy-native launcher for the eXo
//! collections. No GUI code lives here; `crates/app` (GTK4) drives it
//! through [`host::AppHandle`] and the async functions in `commands`.
//!
//! Module map (see docs/ARCHITECTURE.md):
//! - `host`        what the UI hands the backend: managed state, events, the tokio runtime
//! - `commands/`   backend operations by responsibility (catalogue, setup, install, library, ...)
//! - `launchers/`  the emulator seam: one module per `Launcher` kind (DOSBox Staging today)
//! - `emulators`   where emulator binaries come from (emulator packs, system PATH)
//! - `omarchy`     the theme bridge (colors.toml watcher -> `theme-changed` events)
//! - `models/`     the serialisable records the UI renders
//! - `db/`         SQLite schema, catalogue refresh, queries
//! - `torrent/`    librqbit session, selective downloads, ranged zip reads
//! - `import/`     LaunchBox XML parser (build-time `generate_db` and import backfill)

pub mod commands;
pub mod db;
pub mod host;
pub mod emulators;
pub mod import;
pub mod launchers;
pub mod media_sources;
pub mod models;
pub mod omarchy;
pub mod support_files;
pub mod torrent;
pub mod vhd;

// Re-export utilities used by the generate_db example and integration tests
pub use commands::game_name_from_app_path;
pub use commands::torrent_search_names;
pub use commands::paths::game_root;
pub use commands::{collection_base_id, CollectionDef, COLLECTION_MAP};
#[doc(hidden)]
pub use commands::{bundled_metadata_dir, extract_bundled_configs, load_root_folder, scan_installed_games_with_db, set_root_folder};

use std::path::Path;
use std::sync::Mutex;

use tokio::sync::RwLock;

use commands::{init_log_dir, init_resource_dir, ContentPackState, DbState, TorrentState};

/// The SQLite file inside `paths::app_data_dir()`.
pub const DB_FILE_NAME: &str = "exorchy.db";
/// The bundled catalogue inside `metadata/` (gzipped).
pub const BUNDLED_DB_NAME: &str = "exorchy.db";

/// Raise the fd soft limit as far as allowed: librqbit keeps every torrent
/// file open (14,011 for eXoDOS), and the default 1024 fails the first add.
fn raise_fd_limit() {
    unsafe {
        let mut lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) != 0 {
            return;
        }
        let mut candidates: Vec<libc::rlim_t> = vec![lim.rlim_max.min(1 << 20), 65536, 10240];
        // Descending order so the early-exit below stays correct.
        candidates.sort_unstable_by(|a, b| b.cmp(a));
        candidates.dedup();

        // The eXoDOS torrent has 14 011 files and librqbit holds an fd for
        // each; anything below this leaves the main collection un-addable.
        const COMFORTABLE: libc::rlim_t = 15_000;

        for target in candidates {
            if target <= lim.rlim_cur {
                break; // already high enough
            }
            let new = libc::rlimit { rlim_cur: target, rlim_max: lim.rlim_max };
            if libc::setrlimit(libc::RLIMIT_NOFILE, &new) == 0 {
                log::info!("Raised open-file limit: {} -> {}", lim.rlim_cur, target);
                if target < COMFORTABLE {
                    log::warn!(
                        "Open-file limit {} is below the ~{} needed for the main eXoDOS \
                         torrent - downloads from it will fail with 'error opening ... in \
                         read/write mode'. Raise the hard limit (ulimit -n / limits.conf).",
                        target, COMFORTABLE
                    );
                }
                return;
            }
        }
        if lim.rlim_cur < COMFORTABLE {
            log::warn!(
                "Could not raise open-file limit above {} - large torrents may fail to add",
                lim.rlim_cur
            );
        }
    }
}

/// Copy the bundled pre-built DB to the target path.
pub fn install_bundled_db(target: &Path) -> Result<(), String> {
    let metadata_dir = bundled_metadata_dir()?;

    let bundled_db = metadata_dir.join(BUNDLED_DB_NAME);
    let bundled_db_gz = metadata_dir.join(format!("{BUNDLED_DB_NAME}.gz"));

    // Clean up any stale WAL/SHM files
    let _ = std::fs::remove_file(target.with_extension("db-wal"));
    let _ = std::fs::remove_file(target.with_extension("db-shm"));

    if bundled_db.exists() {
        std::fs::copy(&bundled_db, target).map_err(|e| format!("Failed to copy bundled DB: {}", e))?;
        log::info!("Installed bundled DB from {}", bundled_db.display());
    } else if bundled_db_gz.exists() {
        use flate2::read::GzDecoder;
        let file = std::fs::File::open(&bundled_db_gz).map_err(|e| e.to_string())?;
        let mut decoder = GzDecoder::new(file);
        let mut out = std::fs::File::create(target).map_err(|e| e.to_string())?;
        std::io::copy(&mut decoder, &mut out).map_err(|e| e.to_string())?;
        log::info!("Installed bundled DB from {}", bundled_db_gz.display());
    } else {
        return Err(format!("No bundled database found in {}", metadata_dir.display()));
    }
    Ok(())
}

/// Empty in-memory DB used when the real one can't be opened, so the app
/// can still reach the event loop and show the startup-error dialog without
/// commands panicking on missing state.
fn fallback_in_memory_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().expect("in-memory SQLite cannot fail to open");
    let _ = db::init(&conn);
    conn
}

/// Open the installed DB, (re)installing the bundled one when the file is
/// missing, unreadable, or empty (post factory-reset). Every failure comes
/// back as a message for the startup error dialog instead of a panic.
fn open_or_reinstall_db(db_path: &Path) -> Result<rusqlite::Connection, String> {
    if !db_path.exists() {
        install_bundled_db(db_path)?;
    }

    match db::open(db_path).and_then(|c| {
        db::init(&c)?;
        Ok(c)
    }) {
        Ok(c) => {
            let count: i64 = c.query_row("SELECT COUNT(*) FROM games", [], |r| r.get(0)).unwrap_or(0);
            if count == 0 {
                drop(c);
                install_bundled_db(db_path)?;
                let c = db::open(db_path).map_err(|e| format!("failed to open freshly installed DB: {}", e))?;
                db::init(&c).map_err(|e| format!("failed to run migrations: {}", e))?;
                Ok(c)
            } else {
                Ok(c)
            }
        }
        Err(e) => {
            log::warn!("Database unreadable ({}), reinstalling", e);
            let _ = std::fs::remove_file(db_path);
            install_bundled_db(db_path)?;
            let c = db::open(db_path).map_err(|e| format!("failed to open freshly installed DB: {}", e))?;
            db::init(&c).map_err(|e| format!("failed to initialize schema: {}", e))?;
            Ok(c)
        }
    }
}

/// Copy the bundled catalog DB into the app data dir for ATTACH: in place it
/// would write WAL sidecars into the resources dir. Caller deletes.
fn stage_bundled_catalog(data_dir: &Path) -> Result<std::path::PathBuf, String> {
    let metadata_dir = bundled_metadata_dir()?;
    let tmp = data_dir.join("catalog-refresh.db");

    let bundled_db = metadata_dir.join(BUNDLED_DB_NAME);
    if bundled_db.exists() {
        std::fs::copy(&bundled_db, &tmp).map_err(|e| e.to_string())?;
        return Ok(tmp);
    }
    let bundled_db_gz = metadata_dir.join(format!("{BUNDLED_DB_NAME}.gz"));
    if bundled_db_gz.exists() {
        let file = std::fs::File::open(&bundled_db_gz).map_err(|e| e.to_string())?;
        let mut decoder = flate2::read::GzDecoder::new(file);
        let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        std::io::copy(&mut decoder, &mut out).map_err(|e| e.to_string())?;
        return Ok(tmp);
    }
    Err(format!("No bundled database found in {}", metadata_dir.display()))
}

/// Make-writer that locks a shared file handle on every write.
#[derive(Clone)]
struct SharedFileMakeWriter(std::sync::Arc<std::sync::Mutex<std::fs::File>>);

impl std::io::Write for SharedFileMakeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.0.lock() {
            Ok(mut f) => f.write_all(buf).map(|_| buf.len()),
            Err(_) => Ok(buf.len()),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self.0.lock() {
            Ok(mut f) => f.flush(),
            Err(_) => Ok(()),
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedFileMakeWriter {
    type Writer = SharedFileMakeWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The tracing subscriber: stderr plus `<log_dir>/exorchy.log`, with `log!`
/// bridged in. Returns the log path.
fn init_logger(log_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::io::Write;
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let _ = std::fs::create_dir_all(log_dir);
    let log_path = log_dir.join("exorchy.log");

    // Rotate once the log grows past ~10 MB: keep exactly one predecessor.
    if let Ok(meta) = std::fs::metadata(&log_path) {
        if meta.len() > 10 * 1024 * 1024 {
            let _ = std::fs::rename(&log_path, log_dir.join("exorchy.log.1"));
        }
    }

    let file_result = std::fs::OpenOptions::new().create(true).append(true).open(&log_path);

    // info by default; `RUST_LOG=librqbit=debug,exorchy_lib=debug` to dig.
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let file_writer: Option<SharedFileMakeWriter> = match file_result {
        Ok(mut file) => {
            let epoch_secs = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = writeln!(
                file,
                "\n=== exorchy session start (epoch {}, log_dir {}) ===",
                epoch_secs,
                log_dir.display()
            );
            Some(SharedFileMakeWriter(std::sync::Arc::new(std::sync::Mutex::new(file))))
        }
        Err(_) => None,
    };

    let stderr_layer = fmt::layer().with_writer(std::io::stderr).with_target(true).with_ansi(false);
    let registry = tracing_subscriber::registry().with(env_filter).with(stderr_layer);

    let result = if let Some(writer) = file_writer.clone() {
        let file_layer = fmt::layer().with_writer(writer).with_target(true).with_ansi(false);
        registry.with(file_layer).try_init()
    } else {
        registry.try_init()
    };

    if result.is_err() {
        // A subscriber was already installed (e.g. tests) - not fatal.
        return None;
    }

    // Bridge `log!` → tracing so log-only crates land in the same sink.
    let _ = tracing_log::LogTracer::init();

    file_writer.map(|_| log_path)
}


/// Everything the backend needs before the first command: XDG dirs, the
/// logger, the database (installed, refreshed, opened), the game root, the
/// enabled collections, the managed state and the Omarchy theme watcher.
///
/// Returns the handle the UI drives the backend with. A startup failure is
/// returned as a message for the UI to show; the handle still carries a
/// usable (empty, in-memory) database so the app can reach its event loop.
pub struct Bootstrap {
    pub app: crate::host::AppHandle,
    pub startup_error: Option<String>,
    pub log_path: Option<std::path::PathBuf>,
}

/// Where the bundled resources are: `EXORCHY_RESOURCE_DIR`, then the
/// package install (`/usr/lib/exorchy`), the user install
/// (`~/.local/lib/exorchy`), else the source checkout.
pub fn resource_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("EXORCHY_RESOURCE_DIR").filter(|d| !d.is_empty()) {
        return std::path::PathBuf::from(d);
    }
    let mut candidates = vec![std::path::PathBuf::from("/usr/lib/exorchy")];
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(Path::new(&home).join(".local/lib/exorchy"));
    }
    candidates
        .into_iter()
        .find(|p| p.join("metadata").is_dir())
        .unwrap_or_else(commands::paths::dev_project_root)
}

pub fn bootstrap() -> Bootstrap {
    raise_fd_limit();
    // The runtime exists before any backend task is spawned.
    let _ = crate::host::async_runtime::runtime();

    // Logger first, so the rest of startup is captured.
    let log_dir = commands::paths::app_log_dir();
    init_log_dir(log_dir.clone());
    let log_path = init_logger(&log_dir);
    if let Some(p) = &log_path {
        log::info!("Log file: {}", p.display());
    }

    // Cache the resource dir BEFORE any code tries to read bundled metadata
    // or torrents - the sync helpers in setup.rs rely on it.
    let res_dir = resource_dir();
    log::info!("Resource dir: {}", res_dir.display());
    init_resource_dir(res_dir);

    let app = crate::host::AppHandle::new();
    let mut startup_error: Option<String> = None;

    let data_dir = commands::paths::app_data_dir();
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        startup_error = Some(format!(
            "Could not create the application data directory {}: {}",
            data_dir.display(),
            e
        ));
    }
    let db_path = data_dir.join(DB_FILE_NAME);
    log::info!("Database path: {}", db_path.display());

    let mut conn = if startup_error.is_none() {
        match open_or_reinstall_db(&db_path) {
            Ok(c) => c,
            Err(msg) => {
                startup_error = Some(format!("Could not open the game database: {}", msg));
                fallback_in_memory_db()
            }
        }
    } else {
        fallback_in_memory_db()
    };

    if let Some(msg) = &startup_error {
        log::error!("Fatal startup error: {}", msg);
    }

    // Catalog refresh when the bundled version is ahead; user state and ids
    // survive (`db::refresh_catalog`).
    let installed_ver = db::catalog_version(&conn);
    if startup_error.is_none() && installed_ver < db::CATALOG_VERSION {
        match stage_bundled_catalog(&data_dir) {
            Ok(cat_path) => {
                match db::refresh_catalog(&mut conn, &cat_path) {
                    Ok((updated, inserted)) => {
                        log::info!(
                            "Catalog refreshed v{} -> v{}: {} rows updated, {} inserted",
                            installed_ver, db::CATALOG_VERSION, updated, inserted
                        );
                    }
                    Err(e) => log::error!("Catalog refresh failed: {}", e),
                }
                let _ = std::fs::remove_file(&cat_path);
            }
            Err(e) => log::error!("Catalog refresh: bundled DB unavailable: {}", e),
        }
    }

    // Establishes the game root. Must run before anything below reads it.
    commands::paths::load_root_folder(&conn);
    // Which packs the library shows (Settings → Collections).
    db::queries::load_enabled_collections(&conn);

    // Clean up stale content-pack download artifacts from interrupted installs.
    if let Ok(Some(user_data_dir)) = db::queries::get_config(&conn, "data_dir") {
        let user_data_path = std::path::Path::new(&user_data_dir);
        commands::content_packs::cleanup_stale_downloads(user_data_path);
        commands::content_packs::cleanup_stale_content_packs(&conn, user_data_path);
    }

    app.manage(DbState(Mutex::new(conn)));
    app.manage(TorrentState(RwLock::new(std::collections::HashMap::new())));
    app.manage(ContentPackState::new());
    app.manage(commands::media::VideoState::new());
    app.manage(commands::media::MusicState::new());
    app.manage(commands::media::MediaServerState::new());
    app.manage(media_sources::MediaTorrentState::new());
    app.manage(commands::reading::ReadingState::new());

    // Omarchy theme: read once now, then follow `omarchy theme set`.
    omarchy::start_theme_watcher(app.clone());

    Bootstrap { app, startup_error, log_path }
}

/// Flush librqbit's session state deterministically. Call once, from the
/// UI's exit path, before the process ends.
pub fn shutdown(app: &crate::host::AppHandle) {
    let handle = app.clone();
    crate::host::async_runtime::block_on(async move {
        commands::setup::shutdown_torrent_session(&handle).await;
    });
}

#[cfg(test)]
mod fd_limit_tests {
    #[test]
    fn raise_fd_limit_reaches_torrent_scale() {
        super::raise_fd_limit();
        unsafe {
            let mut lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
            assert_eq!(libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim), 0);
            assert!(lim.rlim_cur >= 10240.min(lim.rlim_max));
        }
    }
}

