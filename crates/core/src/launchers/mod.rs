//! The launcher seam: one `Launcher` kind per emulator pipeline. `launch_game`
//! (commands/games.rs) is the collection-agnostic spine: op-lock, DB read,
//! guards, archive extraction, spawn. `prepare` is the only step that differs
//! per kind. DOSBox Staging builds a command the spine spawns; the Win9x and
//! ScummVM launchers (commands/win9x.rs, commands/scummvm.rs, ported whole
//! from Exorchy) spawn through `spawn_emulator_and_track` themselves and
//! report `Launched`. A new kind is one module and one match arm.

pub(crate) mod dosbox;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use crate::host::AppHandle;

use crate::commands::collections::Launcher;
use crate::commands::shell_open::sanitize_appimage_env;
use crate::models::Game;

/// Everything a launcher needs to build its command line.
pub(crate) struct LaunchContext<'a> {
    pub app: &'a AppHandle,
    pub game: &'a Game,
    pub id: i64,
    /// The configured data dir (setup has run).
    pub data_dir: &'a str,
    /// `paths::game_root(data_dir)`: the single root every collection shares.
    pub root: PathBuf,
    /// Global preferences at the moment of launch.
    pub fullscreen: bool,
    pub crt_auto: bool,
    /// The game's `game_config` rows.
    pub per_game: &'a HashMap<String, String>,
}

/// A process ready to spawn, plus the binary it runs (for the log line).
pub(crate) struct PreparedLaunch {
    pub cmd: Command,
    pub binary: PathBuf,
}

/// What a launcher hands back to the spine.
pub(crate) enum LaunchOutcome {
    /// A process for the spine to spawn and track.
    Prepared(PreparedLaunch),
    /// The launcher spawned (and tracks) the emulator itself; the "Launched:"
    /// message for the frontend.
    Launched(String),
}

/// Dispatch to the launcher of a collection. Every arm is one module.
pub(crate) async fn prepare(kind: Launcher, ctx: &LaunchContext<'_>) -> Result<LaunchOutcome, String> {
    match kind {
        Launcher::DosBox => dosbox::prepare(ctx).await.map(LaunchOutcome::Prepared),
        Launcher::Win9x => crate::commands::win9x::launch_win9x_game(
            ctx.app, ctx.game.clone(), ctx.id, ctx.data_dir, ctx.fullscreen, ctx.per_game,
        )
        .await
        .map(LaunchOutcome::Launched),
        Launcher::ScummVm => crate::commands::scummvm::launch_scummvm_game(
            ctx.app, ctx.game.clone(), ctx.id, ctx.data_dir, ctx.fullscreen, ctx.per_game,
        )
        .await
        .map(LaunchOutcome::Launched),
    }
}

/// The emulator packs bundle their own SDL, whose PipeWire backend fails to
/// connect on a current Omarchy ("Could not open audio device: Pipewire:
/// Failed to connect stream!") and leaves the game silent. PipeWire's
/// PulseAudio server is always there on Omarchy, so point SDL at it when
/// the user has not chosen a backend themselves.
pub(crate) fn prefer_pulse_audio_backend(cmd: &mut Command) {
    if std::env::var_os("SDL_AUDIODRIVER").is_some() {
        return;
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let has_pulse = std::env::var_os("PULSE_SERVER").is_some()
        || runtime.is_some_and(|r| r.join("pulse").join("native").exists());
    if has_pulse {
        cmd.env("SDL_AUDIODRIVER", "pulseaudio");
    }
}

/// Currently-running games (keyed by shortcode:language, or id when no
/// shortcode exists). Inserted at spawn, removed by the reaper task when
/// the emulator exits. Lets uninstall refuse while the game's files are open.
pub(crate) fn running_games() -> &'static Mutex<std::collections::HashSet<String>> {
    static RUNNING: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    RUNNING.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// Emulator pid per launched game id, so "Stop game" can end it. Inserted at
/// spawn, removed by the reaper - a stale entry can therefore never outlive
/// the process by more than the reaper's turnaround.
pub(crate) fn running_pids() -> &'static Mutex<std::collections::HashMap<i64, u32>> {
    static PIDS: OnceLock<Mutex<std::collections::HashMap<i64, u32>>> = OnceLock::new();
    PIDS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Ids of the games whose emulator is running right now.
pub(crate) fn running_game_ids() -> Vec<i64> {
    running_pids().lock().map(|m| m.keys().copied().collect()).unwrap_or_default()
}

/// SIGTERM the emulator of a running game; Staging shuts down cleanly on it.
/// The reaper notices the exit and emits `game-exited` as for any other quit.
pub(crate) fn stop_running_game(id: i64) -> Result<(), String> {
    let pid = running_pids()
        .lock()
        .map_err(|e| e.to_string())?
        .get(&id)
        .copied()
        .ok_or("Game is not running")?;
    let rc = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if rc != 0 {
        return Err(format!("Could not stop the emulator (pid {})", pid));
    }
    log::info!("Stopped game {} (pid {})", id, pid);
    Ok(())
}

pub(crate) fn running_game_key(game: &Game) -> String {
    match game.shortcode.as_deref() {
        Some(sc) => format!("{}:{}", sc, game.language),
        None => format!("id:{}", game.id.unwrap_or(-1)),
    }
}

/// Per-game mutual exclusion for launch, uninstall and download: an uninstall
/// during launch-time extraction would rename the dir under the extractor.
pub(crate) fn game_op_lock(id: i64) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<
        Mutex<HashMap<i64, std::sync::Arc<tokio::sync::Mutex<()>>>>,
    > = OnceLock::new();
    let map = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = map.lock().expect("game-op lock map poisoned");
    std::sync::Arc::clone(
        map.entry(id)
            .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(()))),
    )
}

/// launch changed (the music player pauses for a running game).
#[derive(Clone, Serialize)]
pub(crate) struct GameExited {
    id: i64,
}

/// Stdio setup, spawn and reaping for every emulator process, whichever
/// launcher built the command.
pub(crate) fn spawn_emulator_and_track(
    app: &AppHandle,
    mut cmd: Command,
    emulator_bin: &Path,
    // What `running_games` tracks and what the "Launched:" line names.
    run_key: &str,
    label: &str,
    id: i64,
) -> Result<String, String> {
    // Per-game emulator log: stdout and stderr of the process.
    {
        cmd.stdin(std::process::Stdio::null());
        let mut stdio_set = false;
        if let Some(log_dir) = crate::commands::paths::LOG_DIR.get() {
            let _ = std::fs::create_dir_all(log_dir);
            let dosbox_log_path = log_dir.join(format!("dosbox-{}.log", id));
            match std::fs::File::create(&dosbox_log_path) {
                Ok(stdout_file) => match stdout_file.try_clone() {
                    Ok(stderr_file) => {
                        cmd.stdout(std::process::Stdio::from(stdout_file));
                        cmd.stderr(std::process::Stdio::from(stderr_file));
                        log::info!("Emulator output → {}", dosbox_log_path.display());
                        stdio_set = true;
                    }
                    Err(e) => log::warn!("Emulator log handle clone failed: {e}"),
                },
                Err(e) => log::warn!(
                    "Failed to open emulator log file {}: {e}",
                    dosbox_log_path.display()
                ),
            }
        }
        if !stdio_set {
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());
        }
    }

    sanitize_appimage_env(&mut cmd);
    prefer_pulse_audio_backend(&mut cmd);

    log::info!("Spawning emulator: {}", emulator_bin.display());
    let mut child = cmd.spawn().map_err(|e| {
        log::error!("Emulator spawn failed for {}: {} (raw_os_error={:?})",
            emulator_bin.display(), e, e.raw_os_error());
        format!(
            "Failed to launch emulator ({}): {}",
            emulator_bin.display(), e
        )
    })?;

    // Reap the child and track the running game, so uninstall can refuse
    // while the emulator holds files open (a live rename loses saves on
    // Windows).
    let run_key = run_key.to_string();
    running_games().lock().map(|mut s| s.insert(run_key.clone())).ok();
    running_pids().lock().map(|mut m| m.insert(id, child.id())).ok();
    let app = app.clone();
    crate::host::async_runtime::spawn_blocking(move || {
        match child.wait() {
            Ok(status) => log::info!("Emulator exited ({}) for {}", status, run_key),
            Err(e) => log::warn!("Emulator wait failed for {}: {}", run_key, e),
        }
        running_games().lock().map(|mut s| s.remove(&run_key)).ok();
        running_pids().lock().map(|mut m| m.remove(&id)).ok();
        let _ = app.emit("game-exited", GameExited { id });
    });

    Ok(format!("Launched: {}", label))
}

#[cfg(test)]
mod audio_backend_tests {
    use super::prefer_pulse_audio_backend;
    use std::process::Command;
    use std::sync::Mutex;

    /// The environment is process-wide and tests run in parallel: every test
    /// here that sets a variable holds this for its whole body.
    static ENV: Mutex<()> = Mutex::new(());

    fn env_of(cmd: &Command, key: &str) -> Option<String> {
        cmd.get_envs()
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    }

    /// A backend the user chose (SDL_AUDIODRIVER in the app's environment)
    /// is never overridden; the pulse socket decides otherwise.
    #[test]
    fn respects_an_explicit_sdl_audio_driver() {
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        // Environment is process-wide: set, check, restore.
        let saved = std::env::var_os("SDL_AUDIODRIVER");
        std::env::set_var("SDL_AUDIODRIVER", "alsa");
        let mut cmd = Command::new("true");
        prefer_pulse_audio_backend(&mut cmd);
        assert_eq!(env_of(&cmd, "SDL_AUDIODRIVER"), None, "the inherited value stands");
        match saved {
            Some(v) => std::env::set_var("SDL_AUDIODRIVER", v),
            None => std::env::remove_var("SDL_AUDIODRIVER"),
        }
    }

    #[test]
    fn points_sdl_at_pulse_when_the_socket_exists() {
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let saved_driver = std::env::var_os("SDL_AUDIODRIVER");
        let saved_runtime = std::env::var_os("XDG_RUNTIME_DIR");
        let saved_server = std::env::var_os("PULSE_SERVER");
        std::env::remove_var("SDL_AUDIODRIVER");
        std::env::remove_var("PULSE_SERVER");
        let dir = std::env::temp_dir().join(format!("exorchy_pulse_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pulse")).unwrap();
        std::env::set_var("XDG_RUNTIME_DIR", &dir);

        let mut none = Command::new("true");
        prefer_pulse_audio_backend(&mut none);
        assert_eq!(env_of(&none, "SDL_AUDIODRIVER"), None, "no socket, no override");

        std::fs::write(dir.join("pulse").join("native"), b"").unwrap();
        let mut pulse = Command::new("true");
        prefer_pulse_audio_backend(&mut pulse);
        assert_eq!(env_of(&pulse, "SDL_AUDIODRIVER").as_deref(), Some("pulseaudio"));

        let _ = std::fs::remove_dir_all(&dir);
        match saved_runtime { Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v), None => std::env::remove_var("XDG_RUNTIME_DIR") }
        match saved_driver { Some(v) => std::env::set_var("SDL_AUDIODRIVER", v), None => std::env::remove_var("SDL_AUDIODRIVER") }
        if let Some(v) = saved_server { std::env::set_var("PULSE_SERVER", v) }
    }
}
