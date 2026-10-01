//! Emulator binaries: where they come from and how they are found.
//!
//! DOSBox Staging is AUR-only on Arch, so Exorchy fetches the official Linux
//! release tarball as an "emulator pack" (a content pack with a `platforms`
//! map, see `metadata/manifest.json`) into
//! `<data_dir>/content/emulators/dosbox-staging/`. The tarball is
//! self-contained: `dosbox` runs in place with its `lib/`, `plugins/` and
//! `resources/{shaders,shader-presets}` beside it. A system `dosbox-staging`
//! (AUR) is used instead when the user asks for it in Settings, or when no
//! pack is installed. The same mechanism carries DOSBox-X / 86Box for a
//! future eXoWin9x launcher.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Manifest pack id and install dir leaf for DOSBox Staging.
pub const DOSBOX_STAGING_PACK_ID: &str = "dosbox-staging";
/// Config key: "1" prefers a `dosbox-staging` found on PATH over the pack.
pub const USE_SYSTEM_DOSBOX_KEY: &str = "use_system_dosbox";
/// Config key: absolute path of a DOSBox Staging binary the user picked.
pub const DOSBOX_BINARY_KEY: &str = "dosbox_binary";

pub const DOSBOX_MISSING_MESSAGE: &str = "DOSBox Staging is not installed yet. \
    Open Settings → Emulators to download it, or install the dosbox-staging package.";

/// The pack's install dir under the data dir.
pub fn dosbox_pack_dir(data_dir: &str) -> PathBuf {
    Path::new(data_dir)
        .join("content")
        .join("emulators")
        .join(DOSBOX_STAGING_PACK_ID)
}

/// The `dosbox` binary inside an installed pack, if the pack is there.
pub fn pack_dosbox_binary(data_dir: &str) -> Option<PathBuf> {
    let bin = dosbox_pack_dir(data_dir).join("dosbox");
    bin.is_file().then_some(bin)
}

/// A `dosbox-staging` on PATH (the AUR package's binary name).
pub fn system_dosbox_binary() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("dosbox-staging"))
        .find(|candidate| candidate.is_file())
}

/// Manifest pack id and install dir leaf for DOSBox-X (owned by eXoWin9x in
/// the manifest; DOS and Windows 3.x games use the same pack).
pub const DOSBOX_X_PACK_ID: &str = "dosbox-x";
/// DOSBox-X's Flatpak id (no official Linux binaries exist for it).
pub const DOSBOX_X_FLATPAK: &str = "com.dosbox_x.DOSBox-X";

/// How a resolved emulator is invoked: a binary on disk, or a Flatpak.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EngineCmd {
    Direct(PathBuf),
    Flatpak(&'static str),
}

impl EngineCmd {
    /// Build a Command; `grant` is a directory the Flatpak sandbox must see.
    pub(crate) fn command(&self, grant: &Path) -> (std::process::Command, PathBuf) {
        self.command_granting(&[grant])
    }

    /// Same, for every directory the emulator reads or writes.
    pub(crate) fn command_granting(&self, grants: &[&Path]) -> (std::process::Command, PathBuf) {
        match self {
            EngineCmd::Direct(bin) => (std::process::Command::new(bin), bin.clone()),
            EngineCmd::Flatpak(id) => {
                let mut cmd = std::process::Command::new("flatpak");
                cmd.arg("run");
                for g in grants {
                    cmd.arg(format!("--filesystem={}", g.display()));
                }
                cmd.arg(id);
                (cmd, PathBuf::from("flatpak"))
            }
        }
    }
}

/// DOSBox-X: the pack's AppImage, then `dosbox-x` on PATH, then the Flatpak.
/// The Win9x launcher puts its own preferences in front of this.
pub(crate) fn resolve_dosbox_x(data_dir: &str) -> Option<EngineCmd> {
    if !data_dir.is_empty() {
        let pack = Path::new(data_dir).join("content/emulators").join(DOSBOX_X_PACK_ID).join("DOSBox-X.AppImage");
        if pack.is_file() {
            return Some(EngineCmd::Direct(pack));
        }
    }
    if let Some(bin) = on_path("dosbox-x") {
        return Some(EngineCmd::Direct(bin));
    }
    flatpak_installed(DOSBOX_X_FLATPAK).then_some(EngineCmd::Flatpak(DOSBOX_X_FLATPAK))
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|c| c.is_file())
}

fn flatpak_installed(id: &str) -> bool {
    std::process::Command::new("flatpak")
        .args(["info", id])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Which binary a launch would use, and why.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DosboxStatus {
    /// "pack" | "system" | "custom" | "missing"
    pub source: String,
    pub path: Option<String>,
    pub pack_installed: bool,
    pub system_available: bool,
    pub use_system: bool,
}

fn config_flag(conn: &rusqlite::Connection, key: &str) -> Option<String> {
    crate::db::queries::get_config(conn, key).ok().flatten().filter(|v| !v.trim().is_empty())
}

/// Resolution order: the user's explicit binary, then PATH when
/// `use_system_dosbox` is on, then the pack, then PATH as a last resort.
pub fn resolve_dosbox_staging(data_dir: &str) -> Option<PathBuf> {
    resolve_with(data_dir, read_prefs())
}

struct Prefs {
    custom: Option<PathBuf>,
    use_system: bool,
}

fn read_prefs() -> Prefs {
    // A side connection: launch holds no DB lock across the spawn, and this
    // is two reads on a WAL database.
    let db_path = crate::commands::paths::app_data_dir().join(crate::DB_FILE_NAME);
    let conn = rusqlite::Connection::open(&db_path).ok();
    let custom = conn
        .as_ref()
        .and_then(|c| config_flag(c, DOSBOX_BINARY_KEY))
        .map(PathBuf::from);
    let use_system = conn
        .as_ref()
        .and_then(|c| config_flag(c, USE_SYSTEM_DOSBOX_KEY))
        .as_deref()
        == Some("1");
    Prefs { custom, use_system }
}

fn resolve_with(data_dir: &str, prefs: Prefs) -> Option<PathBuf> {
    if let Some(custom) = prefs.custom {
        if custom.is_file() {
            return Some(custom);
        }
        log::warn!("Configured DOSBox binary {} is missing; falling back", custom.display());
    }
    if prefs.use_system {
        if let Some(sys) = system_dosbox_binary() {
            return Some(sys);
        }
    }
    pack_dosbox_binary(data_dir).or_else(system_dosbox_binary)
}

/// The status the Settings → Emulators page shows.
pub fn dosbox_status(data_dir: Option<&str>) -> DosboxStatus {
    let prefs = read_prefs();
    let use_system = prefs.use_system;
    let pack_installed = data_dir.and_then(pack_dosbox_binary).is_some();
    let system_available = system_dosbox_binary().is_some();
    let resolved = data_dir.and_then(|d| resolve_with(d, prefs));
    let source = match &resolved {
        None => "missing",
        Some(p) if data_dir.is_some_and(|d| p.starts_with(dosbox_pack_dir(d))) => "pack",
        Some(p) if system_dosbox_binary().as_deref() == Some(p.as_path()) => "system",
        Some(_) => "custom",
    };
    DosboxStatus {
        source: source.to_string(),
        path: resolved.map(|p| p.to_string_lossy().into_owned()),
        pack_installed,
        system_available,
        use_system,
    }
}

/// Staging aborts without its shader set ("Error setting fallback shaders").
/// The pack carries them beside the binary and the AUR package under
/// `/usr/share/dosbox-staging`; anything else is worth a warning in the log.
pub fn warn_if_shaders_missing(binary: &Path) {
    let sentinel = "shaders/interpolation/bilinear.glsl";
    let beside = binary.parent().map(|d| d.join("resources").join(sentinel));
    let mut candidates: Vec<PathBuf> = beside.into_iter().collect();
    candidates.push(PathBuf::from("/usr/share/dosbox-staging").join(sentinel));
    candidates.push(PathBuf::from("/usr/local/share/dosbox-staging").join(sentinel));
    if let Some(cfg) = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    {
        candidates.push(cfg.join("dosbox").join(sentinel));
    }
    if !candidates.iter().any(|p| p.is_file()) {
        log::warn!(
            "No DOSBox Staging shader set found near {} - CRT shaders will not work and Staging may refuse to start",
            binary.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_wins_unless_the_user_prefers_the_system_binary() {
        let dir = std::env::temp_dir().join(format!("exorchy_emu_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let data = dir.to_string_lossy().into_owned();
        let pack_bin = dosbox_pack_dir(&data).join("dosbox");
        std::fs::create_dir_all(pack_bin.parent().unwrap()).unwrap();
        std::fs::write(&pack_bin, b"#!/bin/sh\n").unwrap();

        let got = resolve_with(&data, Prefs { custom: None, use_system: false }).unwrap();
        assert_eq!(got, pack_bin);

        // A custom binary that exists beats everything; a missing one is ignored.
        let custom = dir.join("my-dosbox");
        std::fs::write(&custom, b"#!/bin/sh\n").unwrap();
        assert_eq!(resolve_with(&data, Prefs { custom: Some(custom.clone()), use_system: false }).unwrap(), custom);
        assert_eq!(
            resolve_with(&data, Prefs { custom: Some(dir.join("gone")), use_system: false }).unwrap(),
            pack_bin
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
