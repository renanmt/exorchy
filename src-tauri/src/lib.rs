//! Exorchy: an Omarchy-native launcher for the eXoDOS collections.
//!
//! Module map (see docs/ARCHITECTURE.md):
//! - `commands/`   Tauri commands by responsibility (catalogue, setup, install, library, ...)
//! - `launchers/`  the emulator seam: one module per `Launcher` kind (DOSBox Staging today)
//! - `emulators`   where emulator binaries come from (emulator packs, system PATH)
//! - `omarchy`     the theme bridge (colors.toml watcher -> `theme-changed` events)
//! - `db/`         SQLite schema, catalogue refresh, queries
//! - `torrent/`    librqbit session, selective downloads, ranged zip reads
//! - `import/`     LaunchBox XML parser (build-time `generate_db` and import backfill)

mod commands;
pub mod db;
pub mod emulators;
pub mod import;
pub mod launchers;
pub mod media_sources;
pub mod models;
pub mod omarchy;
mod support_files;
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

use tauri::Manager;
use tokio::sync::RwLock;

use commands::{
    cancel_content_pack_install, cancel_download, create_playlist, data_dir_is_empty,
    delete_playlist, download_game, factory_reset, game_engine_info, game_printing_unavailable,
    get_available_collections, get_config, get_content_pack_progress, get_default_data_dir,
    get_download_progress, get_game, get_game_metadata, get_game_playlists, get_game_settings,
    get_game_variants, get_games, get_genres, get_installed_games, get_log_dir, get_playlists,
    get_poster_dir, get_preview_dir, get_recently_played, get_section_keys, get_setup_status,
    get_torrent_info, get_transfer_stats, init_download_manager, init_log_dir, init_resource_dir,
    install_content_pack, launch_game, list_content_packs, open_document, open_log_folder,
    rename_playlist, reset_game_data, scan_installed_games, set_config, set_game_settings,
    set_playlist_membership, set_rate_limits, set_seeding_enabled, setup_from_local, stop_game,
    toggle_favorite, uninstall_content_pack, uninstall_game, validate_exodos_dir,
    ContentPackState, DbState, TorrentState,
};

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

/// Grant the asset protocol the media subtrees of the data dir at runtime;
/// the static scope covers $RESOURCE/$APPDATA only, and the data dir itself
/// (often $HOME) would be far too wide.
pub fn allow_asset_dir(app: &tauri::AppHandle, data_dir: &Path) {
    // The game root and the content packs under <data>/content.
    let dirs = [
        commands::paths::game_root(&data_dir.to_string_lossy()),
        data_dir.join("content"),
    ];
    for dir in dirs {
        if let Err(e) = app.asset_protocol_scope().allow_directory(&dir, true) {
            log::warn!("Failed to extend asset scope to {}: {}", dir.display(), e);
        }
    }

    // Bundled previews: outside $RESOURCE in `tauri dev`.
    let preview_roots = [
        commands::paths::RESOURCE_DIR.get().map(|d| d.join("previews")),
        Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join("previews")),
    ];
    for previews in preview_roots.into_iter().flatten() {
        if !previews.is_dir() {
            continue;
        }
        if let Err(e) = app.asset_protocol_scope().allow_directory(&previews, true) {
            log::warn!("Failed to extend asset scope to {}: {}", previews.display(), e);
        }
    }
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

/// Which WebKitGTK render-path workaround this Linux session needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderPath {
    /// Drop accelerated compositing entirely (WebProcess rasterizes on its
    /// main thread). Costly, but the only thing that renders on X11/NVIDIA.
    pub disable_dmabuf: bool,
    /// Keep the DMA-BUF renderer but take NVIDIA's implicit-sync path.
    pub disable_explicit_sync: bool,
    /// Override the NVIDIA blocklist WebKitGTK < 2.52 applies to its own
    /// DMA-BUF renderer. Unread by 2.52+, which dropped the check.
    pub force_dmabuf: bool,
}

/// The WebKitGTK render path per GPU vendor and GDK backend (Exorchy §17 has
/// the measurements). NVIDIA needs a different workaround on Wayland
/// (explicit sync off) and X11 (DMA-BUF off); everyone else gets nothing set.
/// `accel_known_bad` forces the path that always renders after an
/// accelerated start died.
pub fn choose_render_path(nvidia: bool, wayland: bool, accel_known_bad: bool) -> RenderPath {
    if !nvidia {
        return RenderPath { disable_dmabuf: false, disable_explicit_sync: false, force_dmabuf: false };
    }
    if wayland && !accel_known_bad {
        return RenderPath { disable_dmabuf: false, disable_explicit_sync: true, force_dmabuf: true };
    }
    RenderPath { disable_dmabuf: true, disable_explicit_sync: false, force_dmabuf: false }
}

/// True when the proprietary NVIDIA driver is loaded. nouveau creates neither
/// of these, so Intel/AMD/nouveau fall through to the upstream default.
pub(crate) fn nvidia_proprietary_in_use() -> bool {
    Path::new("/sys/module/nvidia_drm").exists() || Path::new("/dev/nvidia0").exists()
}

/// The GDK backend, decided as GTK does (`GDK_BACKEND`, else `WAYLAND_DISPLAY`).
fn on_wayland_backend() -> bool {
    match std::env::var("GDK_BACKEND") {
        Ok(v) => v.split(',').next().is_some_and(|first| first.eq_ignore_ascii_case("wayland")),
        Err(_) => std::env::var_os("WAYLAND_DISPLAY").is_some(),
    }
}

/// Marker armed before an accelerated start, cleared after 6 s or a clean
/// exit. Found at startup, it means the last attempt died at first paint,
/// which is after `setup()` - so only elapsed time counts as survival.
fn accel_sentinel() -> std::path::PathBuf {
    commands::paths::app_state_dir().join("accel-attempt")
}

/// Apply the chosen render path, leaving any variable the user set themselves
/// alone - that is the escape hatch when this guess is wrong on their box.
fn apply_render_path() {
    let sentinel = accel_sentinel();
    let known_bad = sentinel.exists();
    let nvidia = nvidia_proprietary_in_use();
    let wayland = on_wayland_backend();
    let path = choose_render_path(nvidia, wayland, known_bad);

    let mut applied: Vec<&str> = Vec::new();
    if path.disable_dmabuf && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        applied.push("WEBKIT_DISABLE_DMABUF_RENDERER=1");
    }
    if path.force_dmabuf && std::env::var_os("WEBKIT_FORCE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_FORCE_DMABUF_RENDERER", "1");
        applied.push("WEBKIT_FORCE_DMABUF_RENDERER=1");
    }
    if path.disable_explicit_sync && std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_none() {
        std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
        applied.push("__NV_DISABLE_EXPLICIT_SYNC=1");
    }

    // Logging is not up yet, so this goes to stderr - it still lands in the
    // journal and in a terminal bug report.
    eprintln!(
        "render path: nvidia={nvidia} wayland={wayland} accel_known_bad={known_bad} -> [{}]",
        if applied.is_empty() { "upstream defaults".to_string() } else { applied.join(", ") }
    );

    if !path.disable_dmabuf && nvidia {
        // Arm the sentinel; first paint or a clean exit disarms it.
        if let Some(dir) = sentinel.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&sentinel, "1");
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(6));
            disarm_accel_sentinel();
        });
    } else {
        disarm_accel_sentinel();
    }
}

/// Pids of every descendant of this process whose command name is
/// `WebKitWebProces` (the kernel truncates `comm` to 15 bytes). WebKitGTK
/// runs it under bwrap, so the walk is recursive.
fn webkit_web_process_pids() -> Vec<i32> {
    let mut parent_of: std::collections::HashMap<i32, i32> = std::collections::HashMap::new();
    let mut comm_of: std::collections::HashMap<i32, String> = std::collections::HashMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return vec![] };
    for e in entries.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<i32>() else { continue };
        let Ok(stat) = std::fs::read_to_string(e.path().join("stat")) else { continue };
        // "pid (comm) state ppid ..." - comm may hold spaces, so split at ')'.
        let Some(close) = stat.rfind(')') else { continue };
        let Some(open) = stat.find('(') else { continue };
        let comm = stat[open + 1..close].to_string();
        let mut rest = stat[close + 2..].split_whitespace();
        let _state = rest.next();
        let Some(ppid) = rest.next().and_then(|p| p.parse::<i32>().ok()) else { continue };
        parent_of.insert(pid, ppid);
        comm_of.insert(pid, comm);
    }
    let me = std::process::id() as i32;
    let descends_from_me = |mut pid: i32| {
        for _ in 0..32 {
            match parent_of.get(&pid) {
                Some(&p) if p == me => return true,
                Some(&p) if p > 1 => pid = p,
                _ => return false,
            }
        }
        false
    };
    comm_of
        .iter()
        .filter(|(_, c)| c.as_str() == "WebKitWebProces")
        .map(|(p, _)| *p)
        .filter(|p| descends_from_me(*p))
        .collect()
}

/// WebKitGTK's web process dies in the NVIDIA EGL driver while running its
/// thread-local destructors at a normal exit (`SkiaGPUWorker`, seen on driver
/// 610.57 with WebKitGTK 2.52 on the accelerated path), and systemd-coredump
/// turns that into a "process crashed" notification every time the app is
/// closed. The app is quitting and the web process holds nothing worth a
/// graceful exit, so it is ended with SIGKILL before WebKit asks it to tear
/// down. NVIDIA only: elsewhere the normal exit is clean.
fn end_webkit_web_processes_hard() {
    if !nvidia_proprietary_in_use() {
        return;
    }
    for pid in webkit_web_process_pids() {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
        log::info!("Ended WebKit web process {pid} before exit (NVIDIA teardown crash workaround)");
    }
}

/// Record that this render path got the app running.
fn disarm_accel_sentinel() {
    let _ = std::fs::remove_file(accel_sentinel());
}

pub fn run() {
    raise_fd_limit();
    apply_render_path();

    tauri::Builder::default()
        // A second instance would contend on the SQLite DB and corrupt the
        // torrent session; focus the existing window instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Logger first, so the rest of setup is captured.
            let log_dir = commands::paths::app_log_dir();
            init_log_dir(log_dir.clone());
            if let Some(p) = init_logger(&log_dir) {
                log::info!("Log file: {}", p.display());
            }

            // Cache the resource_dir BEFORE any code tries to read bundled
            // metadata or torrents - the sync helpers in setup.rs rely on it.
            match app.path().resource_dir() {
                Ok(res_dir) => init_resource_dir(res_dir),
                Err(_) => log::warn!("resource_dir() unavailable; bundled assets may not be found"),
            }

            // A fatal error here must be VISIBLE: a panic shows nothing.
            // So: non-blocking dialog, in-memory DB, let the loop start.
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
                use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                let exit_handle = app.handle().clone();
                app.dialog()
                    .message(format!("{}\n\nSee the log folder for details.", msg))
                    .title("Exorchy failed to start")
                    .kind(MessageDialogKind::Error)
                    .show(move |_| exit_handle.exit(1));
            }

            // Catalog refresh when the bundled version is ahead; user state
            // and ids survive (`db::refresh_catalog`).
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
                // Asset protocol must reach game media in the user-chosen dir.
                allow_asset_dir(app.handle(), user_data_path);
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
            omarchy::start_theme_watcher(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_games,
            get_game,
            get_installed_games,
            get_game_variants,
            get_genres,
            launch_game,
            stop_game,
            commands::games::running_game_ids,
            game_printing_unavailable,
            game_engine_info,
            get_config,
            set_config,
            set_seeding_enabled,
            set_rate_limits,
            get_transfer_stats,
            get_torrent_info,
            get_setup_status,
            setup_from_local,
            commands::setup::setup_fresh,
            get_default_data_dir,
            get_available_collections,
            init_download_manager,
            factory_reset,
            download_game,
            cancel_download,
            uninstall_game,
            reset_game_data,
            commands::archives::archive_usage,
            commands::archives::remove_installed_archives,
            commands::archives::game_disk_usage,
            commands::storage::storage_overview,
            commands::storage::installed_games_storage,
            commands::storage::clear_media_caches,
            commands::storage::delete_save_backups,
            get_download_progress,
            toggle_favorite,
            get_section_keys,
            validate_exodos_dir,
            scan_installed_games,
            commands::install::list_active_downloads,
            list_content_packs,
            install_content_pack,
            uninstall_content_pack,
            get_content_pack_progress,
            cancel_content_pack_install,
            get_preview_dir,
            get_poster_dir,
            data_dir_is_empty,
            get_game_metadata,
            get_game_settings,
            set_game_settings,
            get_recently_played,
            get_log_dir,
            open_log_folder,
            open_document,
            get_playlists,
            create_playlist,
            rename_playlist,
            delete_playlist,
            set_playlist_membership,
            get_game_playlists,
            commands::emulator_cmds::get_dosbox_status,
            commands::emulator_cmds::ensure_dosbox_staging,
            commands::emulator_cmds::get_dos_support_status,
            commands::media::start_game_video,
            commands::media::get_video_status,
            commands::media::media_url,
            commands::media::video_playback_supported,
            commands::media::video_mirror_needed,
            commands::media::cancel_game_video,
            commands::media::start_game_music,
            commands::media::get_music_status,
            commands::media::cancel_game_music,
            commands::media::music_playback_supported,
            commands::media::music_shuffle_candidates,
            commands::media::music_cache_index,
            commands::reading::list_publications,
            commands::reading::list_issues,
            commands::reading::get_issue,
            commands::reading::game_articles,
            commands::reading::set_issue_favorited,
            commands::reading::set_issue_page,
            commands::reading::open_issue,
            commands::reading::get_issue_status,
            commands::reading::reading_cache_index,
            commands::reading::reading_storage_usage,
            commands::reading::remove_issue,
            commands::reading::cancel_issue_fetch,
            commands::reading::install_issue,
            commands::reading::launch_issue,
            commands::win9x::get_win9x_support_status,
            commands::win9x::win9x_engine_available,
            commands::win9x::win9x_network_status,
            commands::win9x::enable_win9x_network,
            commands::win9x::disable_win9x_network,
            commands::win9x::win9x_multiplayer_info,
            commands::win9x::dismiss_win9x_network_prompt,
            commands::scummvm::scummvm_engine_info,
            commands::scummvm::get_scummvm_support_status,
            commands::scummvm::scummvm_variants,
            commands::scummvm::set_scummvm_options,
            omarchy::get_theme,
            omarchy::get_system_font,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // The window closes before Exit fires; WebKit tears the web process
            // down in between, which is where the NVIDIA driver crashes - so the
            // web process is ended here, first.
            if let tauri::RunEvent::WindowEvent { event: tauri::WindowEvent::CloseRequested { .. }, .. } = &event {
                end_webkit_web_processes_hard();
            }
            if matches!(event, tauri::RunEvent::Exit) {
                // Reaching a clean exit proves the render path works.
                disarm_accel_sentinel();
                // Flush librqbit's session state deterministically.
                let handle = app.clone();
                tauri::async_runtime::block_on(async move {
                    commands::setup::shutdown_torrent_session(&handle).await;
                });
            }
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

#[cfg(test)]
mod render_path_tests {
    use super::choose_render_path;

    #[test]
    fn non_nvidia_gets_no_workaround() {
        for wayland in [true, false] {
            for bad in [true, false] {
                let p = choose_render_path(false, wayland, bad);
                assert!(!p.disable_dmabuf);
                assert!(!p.disable_explicit_sync);
                assert!(!p.force_dmabuf);
            }
        }
    }

    #[test]
    fn nvidia_wayland_keeps_acceleration() {
        let p = choose_render_path(true, true, false);
        assert!(!p.disable_dmabuf);
        assert!(p.disable_explicit_sync);
        assert!(p.force_dmabuf);
    }

    #[test]
    fn nvidia_x11_disables_dmabuf() {
        let p = choose_render_path(true, false, false);
        assert!(p.disable_dmabuf);
        assert!(!p.disable_explicit_sync);
        assert!(!p.force_dmabuf);
    }

    #[test]
    fn a_failed_accelerated_start_falls_back() {
        let p = choose_render_path(true, true, true);
        assert!(p.disable_dmabuf);
        assert!(!p.disable_explicit_sync);
        assert!(!p.force_dmabuf);
    }
}

#[cfg(test)]
mod webkit_exit_tests {
    /// The walk must only ever name our own webview's process. A test binary
    /// has no WebKit children, so the answer is empty - and it must not fall
    /// over on the /proc entries of other users' processes.
    #[test]
    fn finds_no_web_process_under_a_test_binary() {
        assert!(super::webkit_web_process_pids().is_empty());
    }
}
