//! Emulator commands: what binary a launch would use and fetching the
//! DOSBox Staging emulator pack (a content pack, see `crate::emulators`).

use tauri::{AppHandle, State};

use crate::db::queries;
use crate::emulators::{self, DosboxStatus, DOSBOX_STAGING_PACK_ID};

use super::{DbState, TorrentState};
use crate::support_files::{self, SupportStatus, DOS_SUPPORT};

/// Which DOSBox Staging a launch would use right now.
#[tauri::command]
pub async fn get_dosbox_status(db_state: State<'_, DbState>) -> Result<DosboxStatus, String> {
    let data_dir = {
        let conn = db_state.lock()?;
        queries::get_config(&conn, "data_dir").ok().flatten()
    };
    Ok(emulators::dosbox_status(data_dir.as_deref()))
}

/// Make sure a DOSBox Staging is available: when none resolves, start the
/// emulator-pack download (idempotent; progress via the content-pack job
/// keyed `eXoDOS:dosbox-staging`). Returns the status after the check.
#[tauri::command]
pub async fn ensure_dosbox_staging(
    app: AppHandle,
    db_state: State<'_, DbState>,
) -> Result<DosboxStatus, String> {
    let data_dir = {
        let conn = db_state.lock()?;
        queries::get_config(&conn, "data_dir").ok().flatten()
    };
    let status = emulators::dosbox_status(data_dir.as_deref());
    if status.path.is_some() || data_dir.is_none() {
        return Ok(status);
    }
    if super::setup::is_offline(&db_state.0) {
        return Ok(status);
    }
    match super::content_packs::start_pack_install(&app, "eXoDOS", DOSBOX_STAGING_PACK_ID).await {
        Ok(()) => log::info!("DOSBox Staging emulator pack download started"),
        Err(e) => log::info!("DOSBox Staging pack not queued: {e}"),
    }
    Ok(status)
}

/// MT-32 ROMs + SoundCanvas soundfont (the DOS support payload): ready,
/// downloading (with progress), missing or failed. The detail panel shows
/// it for games whose conf asks for MIDI.
#[tauri::command]
pub async fn get_dos_support_status(
    db_state: State<'_, DbState>,
    torrent_state: State<'_, TorrentState>,
) -> Result<SupportStatus, String> {
    let data_dir = {
        let conn = db_state.lock()?;
        queries::get_config(&conn, "data_dir").ok().flatten()
    };
    let Some(data_dir) = data_dir else {
        return Ok(SupportStatus::new("missing", 0.0, 0));
    };
    let root = super::paths::game_root(&data_dir);
    let ready = (DOS_SUPPORT.ready)(&root);
    let mgr = torrent_state.0.read().await.get(DOS_SUPPORT.collection).cloned();
    match mgr {
        Some(mgr) => Ok(support_files::status(&DOS_SUPPORT, &mgr, ready).await),
        None => Ok(SupportStatus::new(if ready { "ready" } else { "missing" }, 0.0, 0)),
    }
}
