//! Where the library lives, and moving it.
//!
//! The library is the configured `data_dir`: the game root (`root_folder`,
//! normally `eXoDOS`) and eXorchy's `content/` sit inside it, and every path
//! is derived from those two config keys. Nothing else stores the location
//! except librqbit's `session.json`, which records each torrent's output
//! folder; a move rewrites it (`init_download_manager` would otherwise evict
//! and re-check every torrent whose folder no longer matches).
//!
//! A move never runs beside the rest of the app: Settings records the target
//! (`library_move_target`) and restarts eXorchy, and the window runs the move
//! before the torrent session, the fetchers or a game can touch the library.
//! Same disk: one rename per folder. Another disk: a copy into a staging
//! folder next to the target, renamed into place when complete; the config
//! switches only after every folder is in place, and the originals are
//! deleted last (recorded in `library_move_cleanup` so a crash retries it).
//! An interrupted move therefore always leaves a working library behind.

use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use super::games::DbState;
use super::paths::{is_os_metadata, DEFAULT_ROOT_FOLDER, ROOT_IS_DATA_DIR};
use crate::db::queries;
use crate::host::{AppHandle, State};

/// Target of a move requested in Settings, run at the next start.
pub const MOVE_TARGET_KEY: &str = "library_move_target";
/// "1" once the user kept the library in `$HOME` when asked.
pub const MOVE_DECLINED_KEY: &str = "library_move_declined";
/// JSON list of folders a finished cross-disk move still has to delete.
pub const MOVE_CLEANUP_KEY: &str = "library_move_cleanup";

/// The folders eXorchy itself creates in the data dir, besides the root.
const OWN_FOLDERS: [&str; 2] = ["content", ".content-downloads"];

/// The library folder new installs are offered: `~/Games/eXorchy`.
pub fn default_library_dir() -> PathBuf {
    home_dir().unwrap_or_else(std::env::temp_dir).join("Games").join("eXorchy")
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

#[derive(Debug, Clone, Serialize)]
pub struct LibraryStatus {
    pub data_dir: String,
    pub root_folder: String,
    /// The game root exists. False: the folder was moved by hand or its
    /// drive is not mounted.
    pub found: bool,
    /// The library sits directly in `$HOME` (the old default) and the user
    /// has not declined the move yet.
    pub offer_move: bool,
    pub pending_target: Option<String>,
    /// Where `offer_move` proposes to put it.
    pub suggested: String,
}

/// The library's location and what the start-up gate should do about it.
/// `None` before setup has run.
pub async fn library_status(db: State<'_, DbState>) -> Result<Option<LibraryStatus>, String> {
    let conn = db.lock()?;
    let get = |k: &str| queries::get_config(&conn, k).ok().flatten().filter(|v| !v.trim().is_empty());
    let Some(data_dir) = get("data_dir") else { return Ok(None) };
    let root_folder = get("root_folder").unwrap_or_else(|| DEFAULT_ROOT_FOLDER.to_string());
    let pending_target = get(MOVE_TARGET_KEY);
    let declined = get(MOVE_DECLINED_KEY).as_deref() == Some("1");
    let found = root_path(Path::new(&data_dir), &root_folder).is_dir();
    let in_home = home_dir().is_some_and(|h| same_path(&h, Path::new(&data_dir)));
    Ok(Some(LibraryStatus {
        // `.` means the data dir IS an eXo tree the user imported: offering
        // to move $HOME itself would be absurd.
        offer_move: in_home && root_folder != ROOT_IS_DATA_DIR && found && !declined && pending_target.is_none(),
        data_dir,
        root_folder,
        found,
        pending_target,
        suggested: default_library_dir().to_string_lossy().into_owned(),
    }))
}

/// Keep the library in `$HOME`; the offer is not repeated.
pub async fn decline_library_move(db: State<'_, DbState>) -> Result<(), String> {
    let conn = db.lock()?;
    queries::set_config(&conn, MOVE_DECLINED_KEY, "1").map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct MovePlan {
    pub target: String,
    /// A rename: instant, no space needed.
    pub same_disk: bool,
    /// What a copy has to write (0 on the same disk).
    pub bytes: u64,
    pub free_bytes: Option<u64>,
}

/// Check a move to `target` without doing anything.
pub async fn plan_library_move(db: State<'_, DbState>, target: String) -> Result<MovePlan, String> {
    let (data_dir, root_folder) = location(&db)?;
    tokio::task::spawn_blocking(move || plan(Path::new(&data_dir), &root_folder, Path::new(&target)))
        .await
        .map_err(|e| e.to_string())?
}

/// Record a move for the next start. The caller restarts eXorchy.
pub async fn request_library_move(db: State<'_, DbState>, target: String) -> Result<MovePlan, String> {
    if crate::launchers::running_games().lock().map(|s| !s.is_empty()).unwrap_or(false) {
        return Err("Close the running game first: its files are part of the library.".into());
    }
    let plan = plan_library_move(db.clone(), target).await?;
    let conn = db.lock()?;
    queries::set_config(&conn, MOVE_TARGET_KEY, &plan.target).map_err(|e| e.to_string())?;
    log::info!("Library move requested: {} (same disk: {}, {} bytes)", plan.target, plan.same_disk, plan.bytes);
    Ok(plan)
}

/// Forget a requested move; the library stays where it is.
pub async fn cancel_library_move(db: State<'_, DbState>) -> Result<(), String> {
    let conn = db.lock()?;
    queries::delete_config(&conn, MOVE_TARGET_KEY).map_err(|e| e.to_string())
}

/// `library-move-progress` payload.
#[derive(Debug, Clone, Serialize)]
pub struct MoveProgress {
    pub done: u64,
    pub total: u64,
    /// The folder being moved, e.g. "eXoDOS".
    pub item: String,
}

/// Run the move recorded by `request_library_move`. Call before the torrent
/// session starts. Returns the new data dir.
pub async fn run_library_move(app: AppHandle, db: State<'_, DbState>) -> Result<String, String> {
    let (data_dir, root_folder) = location(&db)?;
    let target = {
        let conn = db.lock()?;
        queries::get_config(&conn, MOVE_TARGET_KEY).map_err(|e| e.to_string())?
    }
    .filter(|t| !t.trim().is_empty())
    .ok_or("No library move was requested")?;

    let (old, new, root) = (PathBuf::from(&data_dir), PathBuf::from(&target), root_folder.clone());
    let emitter = app.clone();
    let copied = tokio::task::spawn_blocking(move || {
        let items = move_items(&old, &root, &new);
        // Neither place holds the game root (a drive gone since the request):
        // switching would point the app at an empty folder.
        if !(root_path(&old, &root).is_dir() || root_path(&new, &root).is_dir()) {
            return Err(format!("The library was not found in {}.", old.display()));
        }
        let same_disk = same_device(&old, &new);
        execute(&items, same_disk, &|p| {
            let _ = emitter.emit("library-move-progress", p);
        })
        .map(|copied| (copied, items))
    })
    .await
    .map_err(|e| e.to_string())??;
    let (copied, items) = copied;

    // Point the torrents at the new root before the session reads them.
    let old_root = root_path(Path::new(&data_dir), &root_folder);
    let new_root = root_path(Path::new(&target), &root_folder);
    let session_json = crate::torrent::manager::fastresume_dir(&super::paths::app_data_dir()).join("session.json");
    match rewrite_session_output_folders(&session_json, &old_root, &new_root) {
        Ok(n) => log::info!("Library move: {n} torrent(s) now write to {}", new_root.display()),
        Err(e) => log::warn!("Library move: session.json not rewritten ({e}); torrents will be re-checked"),
    }

    // The switch. From here on the new location is the library.
    let leftovers: Vec<String> = if copied {
        items.iter().filter(|i| i.from.exists()).map(|i| i.from.to_string_lossy().into_owned()).collect()
    } else {
        Vec::new()
    };
    {
        let conn = db.lock()?;
        queries::set_config(&conn, "data_dir", &target).map_err(|e| e.to_string())?;
        queries::delete_config(&conn, MOVE_TARGET_KEY).map_err(|e| e.to_string())?;
        if !leftovers.is_empty() {
            let json = serde_json::to_string(&leftovers).map_err(|e| e.to_string())?;
            queries::set_config(&conn, MOVE_CLEANUP_KEY, &json).map_err(|e| e.to_string())?;
        }
    }
    log::info!("Library moved: {data_dir} -> {target}");

    finish_library_move_cleanup(db).await?;
    Ok(target)
}

/// Delete what a cross-disk move copied away from, if a previous run did not
/// get to it. Never deletes anything inside the current library.
pub async fn finish_library_move_cleanup(db: State<'_, DbState>) -> Result<(), String> {
    let (paths, data_dir) = {
        let conn = db.lock()?;
        let raw = queries::get_config(&conn, MOVE_CLEANUP_KEY).map_err(|e| e.to_string())?;
        let data_dir = queries::get_config(&conn, "data_dir").map_err(|e| e.to_string())?.unwrap_or_default();
        (raw, data_dir)
    };
    let Some(raw) = paths else { return Ok(()) };
    let paths: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
    let current = PathBuf::from(&data_dir);
    let failed = tokio::task::spawn_blocking(move || {
        let mut failed = Vec::new();
        for p in paths {
            let p = PathBuf::from(p);
            if p.starts_with(&current) || current.starts_with(&p) {
                log::warn!("Library move cleanup: {} overlaps the library, kept", p.display());
                continue;
            }
            if let Err(e) = std::fs::remove_dir_all(&p) {
                if p.exists() {
                    log::warn!("Library move cleanup: cannot delete {}: {e}", p.display());
                    failed.push(p.to_string_lossy().into_owned());
                }
            }
        }
        failed
    })
    .await
    .map_err(|e| e.to_string())?;
    let conn = db.lock()?;
    if failed.is_empty() {
        queries::delete_config(&conn, MOVE_CLEANUP_KEY).map_err(|e| e.to_string())
    } else {
        let json = serde_json::to_string(&failed).map_err(|e| e.to_string())?;
        queries::set_config(&conn, MOVE_CLEANUP_KEY, &json).map_err(|e| e.to_string())
    }
}

/// Point eXorchy at a library that is already somewhere else (moved by hand,
/// a drive mounted at a new path). The folder must hold the game root.
pub async fn locate_library(db: State<'_, DbState>, path: String) -> Result<(), String> {
    let (_, root_folder) = location(&db)?;
    let dir = PathBuf::from(&path);
    let root = root_path(&dir, &root_folder);
    let looks_right = if root_folder == ROOT_IS_DATA_DIR { dir.join("eXo").is_dir() } else { root.is_dir() };
    if !looks_right {
        return Err(if root_folder == ROOT_IS_DATA_DIR {
            format!("{path} has no eXo folder. Choose the folder that holds your games.")
        } else {
            format!("{path} has no {root_folder} folder. Choose the folder that contains it.")
        });
    }
    let conn = db.lock()?;
    queries::set_config(&conn, "data_dir", &path).map_err(|e| e.to_string())?;
    queries::delete_config(&conn, MOVE_TARGET_KEY).map_err(|e| e.to_string())?;
    log::info!("Library located at {path}");
    Ok(())
}

/// Start a new eXorchy once this one has exited. The caller quits right after.
pub async fn relaunch_after_exit() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let pid = std::process::id();
    // Quoted for sh; the path is ours, but it may contain spaces.
    let exe = exe.to_string_lossy().replace('\'', r"'\''");
    let script = format!(
        "for _ in $(seq 100); do kill -0 {pid} 2>/dev/null || break; sleep 0.2; done; \
         if command -v uwsm-app >/dev/null; then exec uwsm-app -- '{exe}'; else exec '{exe}'; fi"
    );
    std::process::Command::new("setsid")
        .args(["-f", "sh", "-c", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot restart eXorchy: {e}"))
}

// ── Internals ────────────────────────────────────────────────────────────

fn location(db: &State<'_, DbState>) -> Result<(String, String), String> {
    let conn = db.lock()?;
    let data_dir = super::games::configured_data_dir(&conn)?;
    let root = queries::get_config(&conn, "root_folder")
        .map_err(|e| e.to_string())?
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_ROOT_FOLDER.to_string());
    Ok((data_dir, root))
}

fn root_path(data_dir: &Path, root_folder: &str) -> PathBuf {
    if root_folder == ROOT_IS_DATA_DIR {
        data_dir.to_path_buf()
    } else {
        data_dir.join(root_folder)
    }
}

#[derive(Debug, Clone, PartialEq)]
struct MoveItem {
    from: PathBuf,
    to: PathBuf,
}

/// What moves: the whole data dir when it is the eXo tree itself, otherwise
/// the root and eXorchy's own folders - never anything else the data dir
/// holds (for the old default that is the rest of `$HOME`).
fn move_items(data_dir: &Path, root_folder: &str, target: &Path) -> Vec<MoveItem> {
    if root_folder == ROOT_IS_DATA_DIR {
        return vec![MoveItem { from: data_dir.to_path_buf(), to: target.to_path_buf() }];
    }
    std::iter::once(root_folder)
        .chain(OWN_FOLDERS)
        .map(|name| MoveItem { from: data_dir.join(name), to: target.join(name) })
        .filter(|i| i.from.exists() || i.to.exists())
        .collect()
}

fn plan(data_dir: &Path, root_folder: &str, target: &Path) -> Result<MovePlan, String> {
    if !target.is_absolute() {
        return Err("Choose a full folder path.".into());
    }
    let target = normalize(target);
    let data_dir = normalize(data_dir);
    if same_path(&target, &data_dir) {
        return Err("The library is already there.".into());
    }
    let items = move_items(&data_dir, root_folder, &target);
    if !items.first().is_some_and(|i| i.from.is_dir()) {
        return Err(format!("The library was not found in {}.", data_dir.display()));
    }
    for item in &items {
        if item.to.starts_with(&item.from) {
            return Err("The new folder is inside the library itself.".into());
        }
        if item.to.exists() && !is_empty_dir(&item.to) {
            return Err(format!("{} already exists. Choose another folder, or empty that one first.", item.to.display()));
        }
    }
    let anchor = existing_ancestor(&target).ok_or("That folder cannot be created.")?;
    if !writable(&anchor) {
        return Err(format!("{} is not writable.", anchor.display()));
    }
    let same_disk = same_device(&data_dir, &target);
    let bytes = if same_disk { 0 } else { items.iter().map(|i| tree_size(&i.from)).sum() };
    let free_bytes = free_space(&anchor);
    if let Some(free) = free_bytes {
        if bytes > free {
            return Err(format!(
                "Not enough space: the library needs {} and {} is free there.",
                human(bytes),
                human(free)
            ));
        }
    }
    Ok(MovePlan { target: target.to_string_lossy().into_owned(), same_disk, bytes, free_bytes })
}

/// Move every item. Returns whether anything was copied (the originals are
/// then still there, to be deleted after the switch). Re-running after an
/// interruption continues: an item already at its target is skipped.
fn execute(items: &[MoveItem], same_disk: bool, progress: &dyn Fn(MoveProgress)) -> Result<bool, String> {
    let pending: Vec<&MoveItem> = items.iter().filter(|i| !(i.to.exists() && !i.from.exists())).collect();
    let mut renamed: Vec<&MoveItem> = Vec::new();
    let mut copy: Vec<&MoveItem> = Vec::new();
    if same_disk {
        for item in &pending {
            match rename_into_place(item) {
                Ok(()) => renamed.push(item),
                // A bind mount looks like the same disk and still refuses.
                Err(e) if e.raw_os_error() == Some(libc::EXDEV) => copy.push(item),
                Err(e) => {
                    // Put back what already moved: the library stays whole.
                    for done in renamed.iter().rev() {
                        if let Err(back) = std::fs::rename(&done.to, &done.from) {
                            log::error!("Library move rollback: {} -> {}: {back}", done.to.display(), done.from.display());
                        }
                    }
                    return Err(format!("Cannot move {}: {e}", item.from.display()));
                }
            }
        }
    } else {
        copy = pending;
    }
    if copy.is_empty() {
        return Ok(false);
    }

    let total: u64 = copy.iter().map(|i| tree_size(&i.from)).sum();
    let mut done = 0u64;
    for item in copy {
        let name = item.to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let parent = item.to.parent().ok_or("target has no parent")?;
        std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
        let staging = parent.join(format!(".exorchy-move-{name}"));
        if staging.exists() {
            std::fs::remove_dir_all(&staging).map_err(|e| format!("Cannot clear {}: {e}", staging.display()))?;
        }
        copy_tree(&item.from, &staging, &mut |n| {
            done += n;
            progress(MoveProgress { done, total, item: name.clone() });
        })?;
        sync_dir(parent);
        if item.to.exists() {
            // Checked empty by the plan.
            std::fs::remove_dir(&item.to).map_err(|e| format!("Cannot replace {}: {e}", item.to.display()))?;
        }
        std::fs::rename(&staging, &item.to).map_err(|e| format!("Cannot finish {}: {e}", item.to.display()))?;
    }
    Ok(true)
}

fn rename_into_place(item: &MoveItem) -> std::io::Result<()> {
    if let Some(parent) = item.to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if item.to.exists() {
        std::fs::remove_dir(&item.to)?;
    }
    std::fs::rename(&item.from, &item.to)
}

/// Copy a tree, keeping symlinks as links, permissions and mtimes. Reports
/// bytes as they are written.
fn copy_tree(src: &Path, dst: &Path, on_bytes: &mut dyn FnMut(u64)) -> Result<(), String> {
    let err = |p: &Path, e: std::io::Error| format!("Copying {}: {e}", p.display());
    let mut buf = vec![0u8; 8 << 20];
    for entry in walkdir::WalkDir::new(src).follow_links(false) {
        let entry = entry.map_err(|e| format!("Reading {}: {e}", src.display()))?;
        let rel = entry.path().strip_prefix(src).map_err(|e| e.to_string())?;
        let out = dst.join(rel);
        let ft = entry.file_type();
        if ft.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| err(&out, e))?;
        } else if ft.is_symlink() {
            let link = std::fs::read_link(entry.path()).map_err(|e| err(entry.path(), e))?;
            std::os::unix::fs::symlink(&link, &out).map_err(|e| err(&out, e))?;
        } else if ft.is_file() {
            let meta = entry.metadata().map_err(|e| format!("Reading {}: {e}", entry.path().display()))?;
            let mut from = std::fs::File::open(entry.path()).map_err(|e| err(entry.path(), e))?;
            let mut to = std::fs::File::create(&out).map_err(|e| err(&out, e))?;
            loop {
                let n = from.read(&mut buf).map_err(|e| err(entry.path(), e))?;
                if n == 0 {
                    break;
                }
                to.write_all(&buf[..n]).map_err(|e| err(&out, e))?;
                on_bytes(n as u64);
            }
            to.sync_all().map_err(|e| err(&out, e))?;
            let _ = to.set_permissions(meta.permissions());
            if let Ok(mtime) = meta.modified() {
                let _ = to.set_modified(mtime);
            }
        }
    }
    // Directory permissions last: a read-only dir would refuse its children.
    for entry in walkdir::WalkDir::new(src).follow_links(false).into_iter().flatten() {
        if entry.file_type().is_dir() {
            if let (Ok(meta), Ok(rel)) = (entry.metadata(), entry.path().strip_prefix(src)) {
                let _ = std::fs::set_permissions(dst.join(rel), meta.permissions());
            }
        }
    }
    Ok(())
}

fn sync_dir(dir: &Path) {
    if let Ok(f) = std::fs::File::open(dir) {
        let _ = f.sync_all();
    }
}

/// Point every torrent that wrote under `old_root` at the same place under
/// `new_root`. Written atomically; a missing file is nothing to do.
fn rewrite_session_output_folders(path: &Path, old_root: &Path, new_root: &Path) -> Result<usize, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.to_string()),
    };
    let mut json: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut changed = 0;
    if let Some(torrents) = json.get_mut("torrents").and_then(|t| t.as_object_mut()) {
        for t in torrents.values_mut() {
            let Some(folder) = t.get("output_folder").and_then(|f| f.as_str()).map(PathBuf::from) else { continue };
            if let Ok(rest) = folder.strip_prefix(old_root) {
                let moved = if rest.as_os_str().is_empty() { new_root.to_path_buf() } else { new_root.join(rest) };
                t["output_folder"] = serde_json::Value::String(moved.to_string_lossy().into_owned());
                changed += 1;
            }
        }
    }
    if changed > 0 {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&json).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    }
    Ok(changed)
}

/// Lexical cleanup plus the real path of the part that exists, so
/// `~/Games/../Games/x` and a symlinked home compare equal.
fn normalize(p: &Path) -> PathBuf {
    let mut lexical = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                lexical.pop();
            }
            Component::CurDir => {}
            other => lexical.push(other.as_os_str()),
        }
    }
    let Some(anchor) = existing_ancestor(&lexical) else { return lexical };
    let rest = lexical.strip_prefix(&anchor).map(Path::to_path_buf).unwrap_or_default();
    match std::fs::canonicalize(&anchor) {
        Ok(real) if rest.as_os_str().is_empty() => real,
        Ok(real) => real.join(rest),
        Err(_) => lexical,
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    normalize(a) == normalize(b)
}

fn existing_ancestor(p: &Path) -> Option<PathBuf> {
    p.ancestors().find(|a| a.exists()).map(Path::to_path_buf)
}

fn same_device(a: &Path, b: &Path) -> bool {
    let dev = |p: &Path| existing_ancestor(p).and_then(|x| std::fs::metadata(x).ok()).map(|m| m.dev());
    matches!((dev(a), dev(b)), (Some(x), Some(y)) if x == y)
}

fn is_empty_dir(p: &Path) -> bool {
    std::fs::read_dir(p).map(|mut it| it.all(|e| e.map(|e| is_os_metadata(&e.path())).unwrap_or(false))).unwrap_or(false)
}

fn writable(dir: &Path) -> bool {
    let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()) else { return false };
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

fn free_space(dir: &Path) -> Option<u64> {
    let c = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0).then(|| st.f_bavail as u64 * st.f_frsize as u64)
}

fn tree_size(p: &Path) -> u64 {
    walkdir::WalkDir::new(p)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

fn human(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.0} MB", b / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(root: &Path) {
        std::fs::create_dir_all(root.join("eXoDOS/eXo/eXoDOS/!dos/KEEN")).unwrap();
        std::fs::write(root.join("eXoDOS/eXo/eXoDOS/!dos/KEEN/dosbox.conf"), b"[autoexec]\n").unwrap();
        std::os::unix::fs::symlink("dosbox.conf", root.join("eXoDOS/eXo/eXoDOS/!dos/KEEN/link.conf")).unwrap();
        std::fs::create_dir_all(root.join("content/emulators/x")).unwrap();
        std::fs::write(root.join("content/emulators/x/bin"), vec![7u8; 3000]).unwrap();
        std::fs::write(root.join("unrelated.txt"), b"not ours").unwrap();
    }

    fn check_moved(target: &Path) {
        assert_eq!(std::fs::read(target.join("eXoDOS/eXo/eXoDOS/!dos/KEEN/dosbox.conf")).unwrap(), b"[autoexec]\n");
        assert_eq!(std::fs::read_link(target.join("eXoDOS/eXo/eXoDOS/!dos/KEEN/link.conf")).unwrap(), Path::new("dosbox.conf"));
        assert_eq!(std::fs::read(target.join("content/emulators/x/bin")).unwrap().len(), 3000);
    }

    #[test]
    fn only_our_folders_move_out_of_home() {
        let items = move_items(Path::new("/home/u"), "eXoDOS", Path::new("/home/u/Games/eXorchy"));
        // Nothing exists on this fake path, so the filter drops every item;
        // what matters is that nothing but our names is ever considered.
        assert!(items.is_empty());
        let dir = tempfile::tempdir().unwrap();
        tree(dir.path());
        let items = move_items(dir.path(), "eXoDOS", &dir.path().join("Games/eXorchy"));
        let names: Vec<_> = items.iter().map(|i| i.from.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["eXoDOS", "content"]);
    }

    #[test]
    fn same_disk_move_renames_and_leaves_the_rest_of_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        tree(dir.path());
        let target = dir.path().join("Games/eXorchy");
        let p = plan(dir.path(), "eXoDOS", &target).unwrap();
        assert!(p.same_disk);
        let items = move_items(dir.path(), "eXoDOS", &target);
        let copied = execute(&items, true, &|_| {}).unwrap();
        assert!(!copied);
        check_moved(&target);
        assert!(!dir.path().join("eXoDOS").exists());
        assert!(dir.path().join("unrelated.txt").exists());
    }

    #[test]
    fn copy_path_keeps_the_originals_until_the_switch() {
        let dir = tempfile::tempdir().unwrap();
        tree(dir.path());
        let target = dir.path().join("elsewhere");
        let items = move_items(dir.path(), "eXoDOS", &target);
        let seen = std::cell::Cell::new(0u64);
        let copied = execute(&items, false, &|p| seen.set(p.done)).unwrap();
        assert!(copied);
        check_moved(&target);
        assert!(dir.path().join("eXoDOS/eXo").is_dir(), "the source is deleted only after the config switch");
        assert_eq!(seen.get(), 11 + 3000);
        assert!(!target.join(".exorchy-move-eXoDOS").exists());
    }

    #[test]
    fn an_interrupted_copy_resumes() {
        let dir = tempfile::tempdir().unwrap();
        tree(dir.path());
        let target = dir.path().join("elsewhere");
        // A crash mid-copy leaves a staging dir; content already made it.
        std::fs::create_dir_all(target.join(".exorchy-move-eXoDOS/partial")).unwrap();
        let items = move_items(dir.path(), "eXoDOS", &target);
        execute(&items[1..], false, &|_| {}).unwrap();
        std::fs::remove_dir_all(dir.path().join("content")).unwrap();
        let items = move_items(dir.path(), "eXoDOS", &target);
        execute(&items, false, &|_| {}).unwrap();
        check_moved(&target);
        assert!(!target.join("eXoDOS/partial").exists());
    }

    #[test]
    fn plan_refuses_unsafe_targets() {
        let dir = tempfile::tempdir().unwrap();
        tree(dir.path());
        let d = dir.path();
        assert!(plan(d, "eXoDOS", Path::new("relative/path")).is_err());
        assert!(plan(d, "eXoDOS", d).unwrap_err().contains("already there"));
        assert!(plan(d, "eXoDOS", &d.join("eXoDOS/inside")).unwrap_err().contains("inside the library"));
        std::fs::create_dir_all(d.join("busy/eXoDOS/stuff")).unwrap();
        assert!(plan(d, "eXoDOS", &d.join("busy")).unwrap_err().contains("already exists"));
        // An existing but empty target folder is fine.
        std::fs::create_dir_all(d.join("empty/eXoDOS")).unwrap();
        assert!(plan(d, "eXoDOS", &d.join("empty")).is_ok());
    }

    #[test]
    fn an_imported_tree_moves_whole() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("eXoDOS");
        std::fs::create_dir_all(lib.join("eXo/eXoDOS")).unwrap();
        let items = move_items(&lib, ROOT_IS_DATA_DIR, &dir.path().join("moved"));
        assert_eq!(items, vec![MoveItem { from: lib.clone(), to: dir.path().join("moved") }]);
        assert!(plan(&lib, ROOT_IS_DATA_DIR, &lib.join("sub")).is_err(), "a target inside the tree");
    }

    #[test]
    fn session_output_folders_follow_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(
            &path,
            r#"{"torrents":{"0":{"info_hash":"a","output_folder":"/home/u/eXoDOS","only_files":[1]},
                            "1":{"info_hash":"b","output_folder":"/mnt/other"}}}"#,
        )
        .unwrap();
        let n = rewrite_session_output_folders(&path, Path::new("/home/u/eXoDOS"), Path::new("/home/u/Games/eXorchy/eXoDOS")).unwrap();
        assert_eq!(n, 1);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["torrents"]["0"]["output_folder"], "/home/u/Games/eXorchy/eXoDOS");
        assert_eq!(v["torrents"]["0"]["only_files"][0], 1, "everything else untouched");
        assert_eq!(v["torrents"]["1"]["output_folder"], "/mnt/other");
        assert_eq!(rewrite_session_output_folders(&dir.path().join("none.json"), Path::new("/a"), Path::new("/b")).unwrap(), 0);
    }

    #[test]
    fn the_default_library_is_under_games() {
        assert!(default_library_dir().ends_with("Games/eXorchy"));
    }
}
