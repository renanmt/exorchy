//! DOSBox Staging launcher: eXoDOS, its language packs and (later) eXoWin3x.
//!
//! Turns eXo's authored `dosbox.conf` into a Staging command line: host-path
//! rewriting (§10a), the LP overlay mount, ECE->Staging config translation,
//! then the global and per-game override fragments. Everything DOS-specific
//! lives here; `launchers::launch` is the collection-agnostic spine.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::commands::collections::{collection_game_prefix, collection_lang_dir};
use crate::commands::paths::launch_conf_dir;

use super::{LaunchContext, PreparedLaunch};

/// Build the DOSBox Staging process for an installed, extracted DOS game.
/// The caller has taken the game op-lock, checked `installed`, extracted the
/// archive and verified the game is not already running.
pub(crate) async fn prepare(ctx: &LaunchContext<'_>) -> Result<PreparedLaunch, String> {
    let game = ctx.game;
    let id = ctx.id;
    let dosbox_conf = game.dosbox_conf.as_deref().ok_or_else(|| {
        let msg = format!(
            "Game '{}' (id={}, lang={}, shortcode={:?}) has no DOSBox config path",
            game.title, id, game.language, game.shortcode
        );
        log::error!("launch_game: {}", msg);
        msg
    })?;

    // Every collection lives in ONE root (eXo's own merged layout), so the
    // main tree and this game's tree are the same directory.
    let source = game.torrent_source.as_deref().unwrap_or("eXoDOS");
    let src_game_prefix = collection_game_prefix(source);
    let torrent_root = ctx.root.clone();
    // working_dir is the first path component of game_prefix (e.g. "eXo")
    let working_dir_name = src_game_prefix.split('/').next().unwrap_or("eXo");
    let options_conf = torrent_root.join("eXo/emulators/dosbox/options.conf");

    let Some((game_conf, conf_root)) = resolve_game_conf(ctx.data_dir, dosbox_conf) else {
        let msg = format!(
            "Game config not found: {}\nMake sure the game is fully downloaded and extracted.",
            torrent_root.join(dosbox_conf.replace('\\', "/")).display()
        );
        log::error!("launch_game({}): {}", game.title, msg);
        return Err(msg);
    };
    let working_dir = conf_root.join(working_dir_name);
    if !working_dir.exists() {
        return Err(format!("Working directory not found: {}", working_dir.display()));
    }

    // For LP games, determine the language dir and game path for config patching.
    // The game_folder is the second component of game_prefix (e.g. "eXoDOS" from "eXo/eXoDOS").
    let shortcode = game.shortcode.as_deref().unwrap_or("");
    let game_folder = src_game_prefix.split('/').nth(1).unwrap_or("eXoDOS");
    let lp_info = collection_lang_dir(source).map(|ld| {
        let dir = torrent_root.join(format!("{}/{}/{}", src_game_prefix, ld, shortcode));
        (shortcode, ld, game_folder, dir)
    });
    let game_dir = (!shortcode.is_empty()).then(|| match &lp_info {
        Some((_, _, _, dir)) => dir.clone(),
        None => torrent_root.join(src_game_prefix).join(shortcode),
    });
    if let Some(game_dir) = &game_dir {
        rewrite_bat_host_paths(game_dir, &working_dir);
    }

    let conf_text = std::fs::read_to_string(&game_conf).unwrap_or_default();
    let prints = conf_requests_printer(&conf_text);
    let engine = chosen_engine(ctx.per_game.get("engine").map(String::as_str), game.dosbox_variant.as_deref(), prints);
    if engine == DosEngine::Staging && game.dosbox_variant.as_deref().is_some_and(|v| v.starts_with("ece")) {
        log::info!(
            "Game '{}' is tuned for DOSBox ECE (Windows-only build). Running under DOSBox Staging - experience may vary.",
            game.title
        );
    }

    let patched_conf = patch_dosbox_conf(
        &game_conf,
        &working_dir,
        lp_info.as_ref().map(|(sc, ld, gf, dir)| (*sc, *ld, *gf, dir.as_path())),
        // DOSBox-X reads eXo's ECE/X keys ([midi] fluid.*, mt32.*, [ide]) natively.
        engine == DosEngine::Staging,
    )?;

    log::info!(
        "Launching: {} with config {} (patched: {}, engine: {})",
        game.title,
        game_conf.display(),
        patched_conf.display(),
        engine.label(),
    );

    let launch_dir = launch_conf_dir()?;
    let (mut cmd, dosbox_bin) = match engine {
        DosEngine::Staging => {
            let bin = crate::emulators::resolve_dosbox_staging(ctx.data_dir)
                .ok_or_else(|| crate::emulators::DOSBOX_MISSING_MESSAGE.to_string())?;
            crate::emulators::warn_if_shaders_missing(&bin);
            (Command::new(&bin), bin)
        }
        DosEngine::DosboxX => crate::emulators::resolve_dosbox_x(ctx.data_dir)
            .ok_or_else(|| DOSBOX_X_MISSING_MESSAGE.to_string())?
            .command_granting(&[&torrent_root, &launch_dir]),
    };
    cmd.current_dir(&working_dir).arg("-conf").arg(&patched_conf);
    if options_conf.exists() {
        cmd.arg("-conf").arg(&options_conf);
    }
    // Hyprland: DOSBox-X opens floating at its final size (it cannot rescale).
    let float_window = (engine == DosEngine::DosboxX).then(|| crate::emulators::float_dosbox_x_on_hyprland(&mut cmd)).flatten();
    if engine == DosEngine::DosboxX {
        // eXo's own launch line; the confs set showmenu=false as well.
        cmd.arg("-nomenu");
        if prints {
            if let Some(game_dir) = &game_dir {
                cmd.arg("-conf").arg(printer_fragment(&launch_dir, id, game_dir)?);
            }
        }
    }

    // User preferences, applied LAST and written for both states: Staging
    // defaults glshader to crt-auto, so "off" has to be an explicit value.
    // The shaders are Staging's; DOSBox-X's filter goes in the per-game
    // fragment below, which is written after this one.
    {
        let glshader_val = &ctx.staging_shader;
        let fullscreen_val = if ctx.fullscreen { "true" } else { "false" };
        let frag = match engine {
            DosEngine::Staging => format!("[sdl]\nfullscreen = {fullscreen_val}\n[render]\nglshader = {glshader_val}\n"),
            // Its fullscreen is broken where it floats (see emulators.rs).
            DosEngine::DosboxX => match &float_window {
                Some(size) => format!("[sdl]\nfullscreen = false\nwindowresolution = {size}\n"),
                None => format!("[sdl]\nfullscreen = {fullscreen_val}\n"),
            },
        };
        let conf_path = launch_conf_dir()?.join(format!("global_overrides_{}.conf", id));
        std::fs::write(&conf_path, &frag)
            .map_err(|e| format!("Failed to write global override conf: {e}"))?;
        cmd.arg("-conf").arg(&conf_path);
    }

    // Per-game overrides (last-wins over global). Only written if the user has
    // configured game-specific settings via the Game Settings dialog.
    {
        let game_conf_path = launch_conf_dir()?.join(format!("game_{}.conf", id));
        let mut frag = String::new();
        if let Some(fs) = ctx.per_game.get("fullscreen").filter(|_| float_window.is_none()) {
            frag.push_str(&format!("[sdl]\nfullscreen = {}\n", fs));
        }
        if let Some(gs) = ctx.per_game.get("glshader").filter(|_| engine == DosEngine::Staging) {
            if gs != "default" {
                frag.push_str(&format!("[render]\nglshader = {}\n", gs));
            }
        }
        if engine == DosEngine::DosboxX {
            let per_game = ctx.per_game.get("dosx_filter").map(String::as_str);
            if let Some(filter) = dosbox_x_filter_conf(per_game, ctx.dosx_filter.as_deref()) {
                frag.push_str(&filter);
            }
        }
        if let Some(cy) = ctx.per_game.get("cycles") {
            frag.push_str(&format!("[cpu]\ncycles = {}\n", cy));
        }
        if let Some(custom) = ctx.per_game.get("custom_conf") {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                frag.push('\n');
                frag.push_str(trimmed);
                frag.push('\n');
            }
        }
        if frag.is_empty() {
            // Drop a stale fragment.
            let _ = std::fs::remove_file(&game_conf_path);
        } else {
            std::fs::write(&game_conf_path, &frag)
                .map_err(|e| format!("Failed to write per-game conf: {e}"))?;
            cmd.arg("-conf").arg(&game_conf_path);
        }
    }

    Ok(PreparedLaunch { cmd, binary: dosbox_bin })
}

/// DOSBox Staging's shaders as eXorchy offers them (`glshader`): the
/// adaptive CRTs built into Staging, then the shader files its releases ship
/// under `resources/shaders`. `crt-auto` is what a launch uses when nothing
/// is chosen, as Staging itself does.
pub const STAGING_SHADERS: &[(&str, &str)] = &[
    ("crt-auto", "CRT (automatic)"),
    ("crt-auto-machine", "CRT (per machine)"),
    ("crt-auto-arcade", "CRT: arcade"),
    ("crt-auto-arcade-sharp", "CRT: arcade, sharp"),
    ("crt/crt-hyllian", "CRT: Hyllian"),
    ("crt/vga-1080p", "CRT: VGA"),
    ("crt/vga-1080p-fake-double-scan", "CRT: VGA, double scan"),
    ("sharp", "Off (sharp pixels)"),
    ("interpolation/nearest", "Nearest"),
    ("interpolation/bilinear", "Bilinear"),
    ("interpolation/catmull-rom", "Catmull-Rom"),
    ("scaler/xbr-lv3", "xBR"),
    ("scaler/xbr-lv2-noblend", "xBR, no blend"),
    ("scaler/xbr-lv2-3d", "xBR 3D"),
    ("scaler/advmame2x", "AdvMAME2x"),
    ("scaler/advmame3x", "AdvMAME3x"),
    ("scaler/advinterp2x", "AdvInterp2x"),
    ("scaler/advinterp3x", "AdvInterp3x"),
];

/// The global `global_glshader` as a Staging shader. Unset is Staging's own
/// default; "default" is what the old On/Off switch stored for Off.
pub fn staging_shader(global: Option<&str>) -> String {
    match global {
        None | Some("") => "crt-auto",
        Some("default") => "sharp",
        Some(v) => v,
    }
    .to_string()
}

/// DOSBox-X's filters (`dosx_filter`): id, label. A bare id is a GLSL shader
/// (built in, or one of the files the AppImage ships, both found by name);
/// `scaler/<name>` is one of its software scalers.
pub const DOSBOX_X_FILTERS: &[(&str, &str)] = &[
    ("crt-lottes", "CRT: Lottes"),
    ("crt-lottes-fast", "CRT: Lottes fast"),
    ("crt-geom", "CRT: Geom"),
    ("crt-easymode", "CRT: Easymode"),
    ("crt-hyllian", "CRT: Hyllian"),
    ("crt-aperture", "CRT: Aperture"),
    ("crt-caligari", "CRT: Caligari"),
    ("crt-pi", "CRT: Pi"),
    ("zfast_crt", "CRT: zfast"),
    ("scan2x", "Scanlines 2x"),
    ("scan3x", "Scanlines 3x"),
    ("tv2x", "TV2x"),
    ("tv3x", "TV3x"),
    ("rgb2x", "RGB2x"),
    ("rgb3x", "RGB3x"),
    ("xbr-lv3", "xBR"),
    ("xbr-lv2-noblend", "xBR, no blend"),
    ("advmame2x", "AdvMAME2x"),
    ("advmame3x", "AdvMAME3x"),
    ("advinterp2x", "AdvInterp2x"),
    ("advinterp3x", "AdvInterp3x"),
    ("scaler/hq2x", "HQ2x"),
    ("scaler/hq3x", "HQ3x"),
    ("scaler/xbrz", "xBRZ"),
    ("scaler/2xsai", "2xSaI"),
    ("scaler/super2xsai", "Super2xSaI"),
    ("scaler/supereagle", "SuperEagle"),
];

/// The `dosx_filter` value that turns the global DOSBox-X filter off for one game.
pub const FILTER_NONE: &str = "none";

/// The conf fragment for the DOSBox-X filter in effect: the game's choice,
/// else the global one; None leaves eXo's conf alone. Shaders need OpenGL
/// output and the picture unscaled (eXo's confs ask for `normal2x`);
/// software scalers are `forced`, or DOSBox-X skips them on larger modes.
pub fn dosbox_x_filter_conf(per_game: Option<&str>, global: Option<&str>) -> Option<String> {
    let id = per_game.filter(|v| !v.is_empty()).or(global)?;
    DOSBOX_X_FILTERS.iter().find(|f| f.0 == id)?;
    Some(match id.strip_prefix("scaler/") {
        Some(scaler) => format!("[sdl]\noutput = opengl\n[render]\nglshader = none\nscaler = {scaler} forced\n"),
        None => format!("[sdl]\noutput = opengl\n[render]\nscaler = none\nglshader = {id}\n"),
    })
}

pub const DOSBOX_X_MISSING_MESSAGE: &str = "This game runs under DOSBox-X, which is not installed yet. \
    Download it from the game's panel, or install dosbox-x (AUR) or its Flatpak (com.dosbox_x.DOSBox-X).";

/// Which emulator runs a DOS / Windows 3.x game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DosEngine {
    Staging,
    DosboxX,
}

impl DosEngine {
    /// The `engine` value stored in `game_config` and sent to the UI.
    pub fn key(self) -> &'static str {
        match self {
            DosEngine::Staging => "staging",
            DosEngine::DosboxX => "dosbox-x",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DosEngine::Staging => "DOSBox Staging",
            DosEngine::DosboxX => "DOSBox-X",
        }
    }
}

/// eXo's own pick: DOSBox-X where eXo pins its `x` build (`dosbox.txt` /
/// `dosbox3x.txt`: `x`, `x2`), and for every conf that prints - Staging has no
/// printer, and eXo's ECE build that printed is Windows-only. Staging for the
/// rest, which is what eXo's `staging*`, `dosbox` and `ece*` pins get on Linux.
pub fn exo_engine(variant: Option<&str>, prints: bool) -> DosEngine {
    if prints || matches!(variant, Some("x" | "x2")) {
        DosEngine::DosboxX
    } else {
        DosEngine::Staging
    }
}

/// The per-game `engine` setting wins; empty or unknown means eXo's pick.
pub fn chosen_engine(setting: Option<&str>, variant: Option<&str>, prints: bool) -> DosEngine {
    match setting {
        Some("staging") => DosEngine::Staging,
        Some("dosbox-x") => DosEngine::DosboxX,
        _ => exo_engine(variant, prints),
    }
}

/// Where DOSBox-X writes a printing game's pages: PNG files in the game's
/// own `!prints` folder, beside its saves. eXo's confs say
/// `printoutput=printer`, which only means something on Windows.
pub fn printouts_dir(game_dir: &Path) -> PathBuf {
    game_dir.join("!prints")
}

fn printer_fragment(launch_dir: &Path, id: i64, game_dir: &Path) -> Result<PathBuf, String> {
    let out = printouts_dir(game_dir);
    std::fs::create_dir_all(&out).map_err(|e| format!("Cannot create {}: {e}", out.display()))?;
    let frag = format!("[printer]\nprintoutput = png\nmultipage = false\ndocpath = {}\n", out.display());
    let path = launch_dir.join(format!("printer_{id}.conf"));
    std::fs::write(&path, frag).map_err(|e| format!("Failed to write printer conf: {e}"))?;
    Ok(path)
}

/// The conf enables eXo's virtual printer (`printer=true` or
/// `parallel1=printer`). Comment lines are skipped: one eXoWin3x conf quotes
/// the whole option documentation while disabling the port.
pub(crate) fn conf_requests_printer(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .any(|l| {
            let lower = l.to_ascii_lowercase();
            match lower.split_once('=') {
                Some((k, v)) => {
                    let (k, v) = (k.trim(), v.trim());
                    (k == "printer" && v == "true")
                        || (k.starts_with("parallel") && v.starts_with("printer"))
                }
                None => false,
            }
        })
}

/// Locate a game's dosbox.conf under the game root: the catalogue path
/// first, then the lang-scoped variants (LP rows inherit the EN path).
/// Shared by `prepare` and `game_printing_unavailable`; the returned root
/// is the launch working-dir base.
pub(crate) fn resolve_game_conf(data_dir: &str, dosbox_conf: &str) -> Option<(PathBuf, PathBuf)> {
    let rel = dosbox_conf.replace('\\', "/");
    let root = crate::commands::paths::game_root(data_dir);

    let direct = root.join(&rel);
    if direct.exists() {
        return Some((direct, root));
    }
    // eXo's catalogue and eXo's folders disagree in case (`eXo\eXoWin3X\...`
    // in the XML, `eXo/eXoWin3x/` in the torrent); DOS never cared, Linux
    // does. Walk the path one component at a time, matching case-insensitively.
    if let Some(found) = resolve_rel_ignoring_case(&root, &rel) {
        return Some((found, root));
    }

    let prefix = collection_game_prefix("eXoDOS");
    let segment = crate::commands::collections::collection_def("eXoDOS")
        .map(|c| c.shortcode_segment)
        .unwrap_or("!dos");
    let shortcode = rel
        .strip_suffix("/dosbox.conf")
        .and_then(|p| p.rsplit('/').next())
        .filter(|s| !s.is_empty())?;
    for lang_dir in crate::commands::collections::COLLECTION_MAP.iter().filter_map(|c| c.lang_dir) {
        let alt = root.join(format!("{prefix}/{segment}/{lang_dir}/{shortcode}/dosbox.conf"));
        if alt.exists() {
            return Some((alt, root));
        }
    }
    None
}


/// need no admin rights or developer mode).
fn link_dir(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

/// Per-launch overlay root for an LP game (§10a): `<shortcode>` links to the
/// LP game dir, other root entries the autoexec names link to the real tree.
/// Rebuilt every launch; holds links only.
fn build_lp_overlay(
    working_dir: &std::path::Path,
    game_folder: &str,
    shortcode: &str,
    lang_dir: &str,
    lp_game_dir: &std::path::Path,
    en_conf: &str,
) -> Result<PathBuf, String> {
    let staging = working_dir
        .join(".exorchy_lp")
        .join(format!("{}_{}", lang_dir.trim_start_matches('!'), shortcode));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .map_err(|e| format!("clearing {}: {}", staging.display(), e))?;
    }
    std::fs::create_dir_all(&staging).map_err(|e| format!("creating {}: {}", staging.display(), e))?;
    link_dir(lp_game_dir, &staging.join(shortcode))
        .map_err(|e| format!("linking {}: {}", shortcode, e))?;

    // Pass-through links for other referenced root entries.
    let real_root = working_dir.join(game_folder);
    let needle = format!("{}\\", game_folder);
    let autoexec = en_conf.split("[autoexec]").nth(1).unwrap_or("");
    for (idx, _) in autoexec.match_indices(&needle) {
        let rest = &autoexec[idx + needle.len()..];
        let entry: String = rest
            .chars()
            .take_while(|c| !"\\/\" \t\r\n".contains(*c))
            .collect();
        if entry.is_empty() || entry.eq_ignore_ascii_case(shortcode) {
            continue;
        }
        let dst = staging.join(&entry);
        let src = real_root.join(&entry);
        if !dst.exists() && src.exists() {
            if let Err(e) = link_dir(&src, &dst) {
                log::warn!("LP overlay: pass-through link {} failed: {}", entry, e);
            }
        }
    }
    Ok(staging)
}

/// Can the EN autoexec run against the LP dir through the overlay? Simulates
/// the cd chain and requires the launch command's program to exist there.
/// A path as the emulator would see it: exact match first, then the one
/// entry of the parent whose name differs only in case.
/// `root/rel` with every component of `rel` matched case-insensitively
/// against what is on disk; None when any component is missing.
pub(crate) fn resolve_rel_ignoring_case(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = root.to_path_buf();
    for comp in rel.split('/').filter(|c| !c.is_empty() && *c != ".") {
        let exact = cur.join(comp);
        if exact.exists() {
            cur = exact;
            continue;
        }
        let wanted = comp.to_ascii_lowercase();
        let hit = std::fs::read_dir(&cur)
            .ok()?
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().to_ascii_lowercase() == wanted)?;
        cur = hit.path();
    }
    Some(cur)
}

fn resolve_ignoring_case(path: &std::path::Path) -> Option<PathBuf> {
    if path.exists() {
        return Some(path.to_path_buf());
    }
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    let parent = path.parent()?;
    std::fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().to_ascii_lowercase() == name)
        .map(|e| e.path())
}

fn lp_autoexec_compatible(
    en_conf: &str,
    shortcode: &str,
    lp_game_dir: &std::path::Path,
    real_root: &std::path::Path,
) -> bool {
    let Some(autoexec) = en_conf.split("[autoexec]").nth(1) else {
        return false;
    };
    // cwd: None = the C: mount root. `mount c <target>` sets it - eXo's
    // confs mount the game directory itself, so a following `cd sub` is
    // relative to that, not to the staging dir the paths are rewritten to.
    let mut cwd: Option<PathBuf> = None;
    let mut mount_root: Option<PathBuf> = None;
    for line in autoexec.lines() {
        let t = line.trim();
        let t = t.strip_prefix('@').unwrap_or(t).trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let lower = t.to_ascii_lowercase();

        if lower == "cd" || lower == "cd." || lower == "cd.." {
            continue;
        }
        let cd_target = if let Some(r) = lower.strip_prefix("cd ") {
            Some(r)
        } else if let Some(r) = lower.strip_prefix("cd\\") {
            // "cd\FOO" is an absolute path from the mount root.
            cwd = None;
            Some(r)
        } else {
            None
        };
        if let Some(target) = cd_target {
            let target = target.trim().trim_matches('"');
            if target.is_empty() || target == "\\" || target == "/" || target == ".." {
                cwd = None;
                continue;
            }
            let next = match (&cwd, &mount_root) {
                (None, Some(root)) => root.join(target),
                (None, None) => {
                    if target.eq_ignore_ascii_case(shortcode) {
                        lp_game_dir.to_path_buf()
                    } else {
                        // Root-level cd into a non-game entry resolves
                        // through a pass-through link to the real tree.
                        real_root.join(target)
                    }
                }
                (Some(dir), _) => dir.join(target),
            };
            // DOS is case-insensitive and so is the emulator's view of the
            // mounted host directory; the conf says `cd odyssey` where the
            // folder is `ODYSSEY`. Only Linux cares, and there this check
            // rejected perfectly good confs.
            let Some(next) = resolve_ignoring_case(&next) else {
                log::info!(
                    "LP launch: EN autoexec cd target '{}' missing under LP layout",
                    target
                );
                return false;
            };
            cwd = Some(next);
            continue;
        }

        // `mount c <host path>` decides what C:\ is. Only the form that
        // mounts the game's OWN directory is read here; every other target
        // (eXo's confs often mount the collection root) leaves the staging
        // dir as the root, which is what `cwd = None` already means.
        if let Some(rest) = lower.strip_prefix("mount c ") {
            let target = rest.trim().trim_matches('"').trim_end_matches(['\\', '/']);
            let leaf = target.rsplit(['\\', '/']).next().unwrap_or(target);
            if leaf.eq_ignore_ascii_case(shortcode) {
                mount_root = Some(lp_game_dir.to_path_buf());
            }
            cwd = None;
            continue;
        }

        // Housekeeping lines that never launch anything.
        let is_drive_switch = lower.len() == 2
            && lower.as_bytes()[1] == b':'
            && lower.as_bytes()[0].is_ascii_alphabetic();
        if is_drive_switch
            || ["mount ", "imgmount ", "echo ", "rem ", "set ", "config "]
                .iter()
                .any(|p| lower.starts_with(p))
            || ["cls", "exit", "pause", "echo", "echo."].contains(&lower.as_str())
        {
            continue;
        }

        // First real command is the launch line. Forms this cannot check
        // (boot images, drive-letter paths) are trusted.
        if lower == "boot" || lower.starts_with("boot ") {
            return true;
        }
        let base = t
            .strip_prefix("call ")
            .or_else(|| t.strip_prefix("CALL "))
            .or_else(|| t.strip_prefix("loadfix "))
            .unwrap_or(t);
        // Skip option tokens ("loadfix -32 game.exe") before picking the program.
        let base = base
            .split_whitespace()
            .find(|tok| !tok.starts_with('-'))
            .unwrap_or(base);
        if base.contains(':') || base.contains('\\') || base.contains('/') {
            return true;
        }
        let dir = match (&cwd, &mount_root) {
            (Some(d), _) => d.clone(),
            (None, Some(root)) => root.clone(),
            (None, None) => return true, // command at mount root - rare, trust it
        };
        let base_lower = base.to_ascii_lowercase();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                let stem = name.rsplitn(2, '.').last().unwrap_or(&name);
                if stem == base_lower || name == base_lower {
                    return true;
                }
            }
        }
        log::info!(
            "LP launch: EN launch command '{}' not present in {} - falling back",
            base,
            dir.display()
        );
        return false;
    }
    // No launch command at all (fully commented autoexec): the overlay still
    // works - the caller appends a find_lp_launch command.
    true
}

/// Rewrite eXo's `.\`-relative HOST paths and nothing else. `resolve` gets the
/// token after `.\` and returns the absolute replacement; quoted tokens run to
/// the closing quote. Everything else is GUEST text (`path=C:\;z:\`), which a
/// blanket backslash swap breaks (§15).
pub(crate) fn rewrite_host_paths(text: &str, resolve: &dyn Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    let mut rest = text;
    // Byte offset of the current match in `text`, for the mount-line check.
    let mut consumed = 0usize;
    while let Some(idx) = rest.find(".\\") {
        out.push_str(&rest[..idx]);
        let quoted = out.ends_with('"');
        let tail = &rest[idx + 2..];
        let end = tail
            .find(|c: char| if quoted { c == '"' } else { c.is_whitespace() })
            .unwrap_or(tail.len());
        let replacement = resolve(&tail[..end]);
        // A data dir with spaces needs quotes on mount ARGUMENTS only; a
        // config property takes its value literally, quotes included.
        if !quoted && replacement.contains(' ') && on_mount_line(text, consumed + idx) {
            out.push('"');
            out.push_str(&replacement);
            out.push('"');
        } else {
            out.push_str(&replacement);
        }
        consumed += idx + 2 + end;
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Is the token at `pos` an argument of a `mount`/`imgmount` command?
/// Those are the lines whose host path is parsed as a whitespace-split
/// argument; everything else in a DOSBox conf reads its value verbatim.
fn on_mount_line(text: &str, pos: usize) -> bool {
    let line_start = text[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let head = text[line_start..pos].trim_start().trim_start_matches('@');
    let cmd = head.split_whitespace().next().unwrap_or("");
    cmd.eq_ignore_ascii_case("mount") || cmd.eq_ignore_ascii_case("imgmount")
}

/// Drop trailing separators from a substituted host path: on Windows DOSBox
/// strips a trailing `\` before `stat()`ing a mount target but not `/`, so
/// `mount c .\eXoDOS\` failed silently for 1,570 configs (§10a).
fn trim_trailing_sep(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    // Never shorten a root ("/" or "G:/") into something else.
    if trimmed.is_empty() || trimmed.ends_with(':') {
        path.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Rewrite `.\`-relative host paths in the game's own bats to `./`-relative
/// ones. eXo's multi-disc `run.bat`s `imgmount` images by a host path
/// relative to DOSBox's cwd, and a backslash is not a separator on POSIX:
/// the mount fails silently and the game reports no CD drive (§10a).
/// Windows DOSBox reads both forms, so the caller skips it there.
pub(crate) fn rewrite_bat_host_paths(game_dir: &Path, working_dir: &Path) {
    for bat in bat_files(game_dir, 2) {
        let Ok(bytes) = std::fs::read(&bat) else { continue };
        // Byte-per-char so a CP437 menu survives the round trip unchanged.
        let content: String = bytes.iter().map(|&b| b as char).collect();
        if !content.contains(".\\") {
            continue;
        }
        let rewritten = rewrite_host_paths(&content, &|body| {
            if body.is_empty() {
                return ".\\".to_string();
            }
            let fwd = body.replace('\\', "/");
            if working_dir.join(&fwd).exists() {
                return trim_trailing_sep(&format!("./{}", fwd));
            }
            // Case mismatch between the bat and the unpacked tree (see
            // `patch_dosbox_conf`): use the on-disk spelling.
            match resolve_rel_ignoring_case(working_dir, &fwd).and_then(|p| p.strip_prefix(working_dir).ok().map(|r| r.to_string_lossy().replace('\\', "/"))) {
                Some(rel) => trim_trailing_sep(&format!("./{}", rel)),
                None => format!(".\\{}", body),
            }
        });
        if rewritten == content {
            continue;
        }
        let out: Vec<u8> = rewritten.chars().map(|c| c as u8).collect();
        match std::fs::write(&bat, out) {
            Ok(()) => log::info!("Rewrote host paths in {}", bat.display()),
            Err(e) => log::warn!("Cannot rewrite host paths in {}: {}", bat.display(), e),
        }
    }
}

fn bat_files(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 1 {
                out.extend(bat_files(&path, depth - 1));
            }
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("bat"))
        {
            out.push(path);
        }
    }
    out
}

/// Rewrite a conf's `.\`-relative host paths to absolute ones. For LP games
/// (`lp_info`) the EN conf runs verbatim against the overlay mount; only an
/// incompatible LP layout gets a generated autoexec (§10a).
pub(crate) fn patch_dosbox_conf(
    conf_path: &std::path::Path,
    working_dir: &std::path::Path,
    lp_info: Option<(&str, &str, &str, &std::path::Path)>, // (shortcode, lang_dir, game_folder, lp_game_dir)
    // false when launching under DOSBox ECE, which understands the original
    // ECE [midi] keys natively - translating them would break its MIDI.
    translate_for_staging: bool,
) -> Result<PathBuf, String> {
    let content = std::fs::read_to_string(conf_path)
        .map_err(|e| format!("Failed to read {}: {}", conf_path.display(), e))?;

    // Forward slashes even on Windows: DOSBox accepts them on every platform,
    // and it keeps the substituted host path free of backslashes that a later
    // reader could mistake for guest-side DOS text.
    let abs_prefix = format!("{}/", working_dir.to_string_lossy()).replace('\\', "/");
    // A `.\` token is a host path only when the target exists: eXo also
    // writes `.\` GUEST paths after a drive switch (11th Hour's
    // `imgmount d ".\cd\11HDISK1.cue"` means C:\cd on the mounted drive).
    let to_working_dir = |body: &str| {
        // Bare `.\` is guest text for "current directory" (OxydGold passes it
        // as a program argument) - it would resolve to the working dir, which
        // always exists, so the existence gate alone can't catch it.
        if body.is_empty() {
            return ".\\".to_string();
        }
        let fwd = body.replace('\\', "/");
        let resolved = format!("{}{}", abs_prefix, fwd);
        if std::path::Path::new(&resolved).exists() {
            return trim_trailing_sep(&resolved);
        }
        // eXo's confs spell folders as Windows saw them (`Adark3`); the
        // archive unpacked `adark3`. On ext4 that is a different path, so
        // the mount target is matched ignoring case and written as on disk.
        match resolve_rel_ignoring_case(working_dir, &fwd) {
            Some(found) => trim_trailing_sep(&found.to_string_lossy().replace('\\', "/")),
            None => format!(".\\{}", body),
        }
    };

    let patched = if let Some((shortcode, lang_dir, game_folder, game_dir)) = lp_info {
        // Strategy 1: overlay mount - eXo's autoexec runs as written, only
        // WHERE the files live changes. The link shadows an installed EN copy.
        let real_root = working_dir.join(game_folder);
        let overlay = if game_dir.exists()
            && lp_autoexec_compatible(&content, shortcode, game_dir, &real_root)
        {
            build_lp_overlay(working_dir, game_folder, shortcode, lang_dir, game_dir, &content)
                .map_err(|e| log::warn!("LP overlay build failed for {}: {}", shortcode, e))
                .ok()
        } else {
            None
        };

        if let Some(staging) = overlay {
            log::info!(
                "LP launch: overlay mount for {} ({} -> {})",
                shortcode,
                staging.display(),
                game_dir.display()
            );
            let staging_fwd = staging.to_string_lossy().replace('\\', "/");
            // Route eXoDOS-root references through the overlay, everything else
            // to the real working dir. Both only touch `.\`-relative host paths.
            let mut result = rewrite_host_paths(&content, &|body| {
                if let Some(tail) = body.replace('\\', "/").strip_prefix(game_folder) {
                    let staged = format!("{}{}", staging_fwd, tail);
                    if std::path::Path::new(&staged).exists() {
                        return trim_trailing_sep(&staged);
                    }
                }
                to_working_dir(body)
            });

            // If autoexec has no actual launch command (e.g., all commented out with #),
            // append one found by inspecting the LP game directory.
            if !autoexec_has_launch_cmd(&result) {
                log::info!("LP launch: autoexec has no launch cmd, appending find_lp_launch for {}", shortcode);
                if let Some((subdir, cmd)) = find_lp_launch(game_dir, Some(&content)) {
                    // Strip any trailing `exit` so our appended commands aren't skipped.
                    let trimmed = result.trim_end();
                    if trimmed.to_ascii_lowercase().ends_with("exit") {
                        result.truncate(trimmed.len() - "exit".len());
                        result.push('\n');
                    }
                    // The generated command runs from the mount root; enter the
                    // game dir (via the overlay link) first.
                    result.push_str(&format!("cd {}\n", shortcode));
                    if !subdir.is_empty() {
                        result.push_str(&format!("cd {}\n", subdir));
                    }
                    result.push_str("cls\n");
                    result.push_str(&format!("{}\n", cmd));
                    result.push_str("exit\n");
                }
            }
            result
        } else {
            // Strategy 2: Different directory structure - generate custom autoexec
            log::info!("LP launch: generating custom autoexec for {} (redirected path not found)", shortcode);
            let settings = content
                .split("[autoexec]")
                .next()
                .unwrap_or(&content);

            let mut patched = rewrite_host_paths(settings, &to_working_dir);

            let game_dir_abs = game_dir.to_string_lossy();
            patched.push_str("[autoexec]\n");
            patched.push_str(&format!("@mount c \"{}\"\n", game_dir_abs));
            patched.push_str("c:\n");

            // Find the game subdirectory and launch command
            if let Some((subdir, cmd)) = find_lp_launch(game_dir, Some(&content)) {
                if !subdir.is_empty() {
                    patched.push_str(&format!("cd {}\n", subdir));
                }
                patched.push_str("cls\n");
                patched.push_str(&format!("{}\n", cmd));
            }
            patched.push_str("exit\n");
            patched
        }
    } else {
        // EN game: rewrite host paths only - guest-side DOS text stays as authored.
        rewrite_host_paths(&content, &to_working_dir)
    };

    let patched = if translate_for_staging {
        translate_ide_for_staging(&translate_midi_for_staging(&patched))
    } else {
        patched
    };

    // Name the fragment after the game's conf dir: working_dir is SHARED
    // across every game in a collection, and a fixed name let two
    // concurrent launches read each other's patched conf (wrong game boots).
    let tag = conf_path
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "conf".to_string());
    // CD images whose cue names a track in another case mount empty.
    super::cue::alias_cue_tracks(&patched, working_dir);
    let patched_path = working_dir.join(format!(".exorchy_launch_{}.conf", tag));
    std::fs::write(&patched_path, &patched)
        .map_err(|e| format!("Failed to write patched config: {}", e))?;

    log::debug!("Patched config written to {}", patched_path.display());

    Ok(patched_path)
}

/// ECE's dotted `[midi]` keys (`mt32.romdir`, `fluid.soundfont`) become
/// Staging's `[mt32]` / `[fluidsynth]` sections; `mididevice = default`
/// becomes `auto`. Confs that already carry the sections pass through.
/// Runs after path rewriting, so captured paths are absolute.
fn translate_midi_for_staging(conf: &str) -> String {
    let lower = conf.to_ascii_lowercase();
    let has_ece_keys = lower.contains("mt32.") || lower.contains("fluid.");
    let has_default_device = lower.contains("mididevice");
    if !has_ece_keys && !has_default_device {
        return conf.to_string();
    }
    let has_mt32_section = lower.lines().any(|l| l.trim() == "[mt32]");
    let has_fluid_section = lower.lines().any(|l| l.trim() == "[fluidsynth]");

    let mut romdir: Option<String> = None;
    let mut soundfont: Option<String> = None;
    let mut out: Vec<String> = Vec::with_capacity(conf.lines().count() + 6);
    let mut section = String::new();

    for line in conf.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            section = trimmed.to_ascii_lowercase();
            out.push(line.to_string());
            continue;
        }
        if section == "[midi]" && !trimmed.starts_with('#') {
            if let Some((key, value)) = trimmed.split_once('=') {
                let key = key.trim().to_ascii_lowercase();
                let value = value.trim();
                if key == "mt32.romdir" {
                    romdir = Some(value.to_string());
                    continue; // drop the ECE key
                }
                if key == "fluid.soundfont" {
                    soundfont = Some(value.to_string());
                    continue;
                }
                if key.starts_with("mt32.") || key.starts_with("fluid.") {
                    continue; // ECE tuning keys with no Staging equivalent
                }
                if key == "mididevice" && value.eq_ignore_ascii_case("default") {
                    out.push("mididevice = auto".to_string());
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }

    if !has_mt32_section {
        if let Some(dir) = romdir {
            out.push(String::new());
            out.push("[mt32]".to_string());
            out.push(format!("romdir = {}", dir));
            if !std::path::Path::new(&dir).exists() {
                log::warn!(
                    "MT-32 ROM dir {} not on disk yet - music will be missing until \
                     the DOSBox support files finish downloading",
                    dir
                );
            }
        }
    }
    if !has_fluid_section {
        if let Some(sf) = soundfont {
            out.push(String::new());
            out.push("[fluidsynth]".to_string());
            out.push(format!("soundfont = {}", sf));
            if !std::path::Path::new(&sf).exists() {
                log::warn!(
                    "Soundfont {} not on disk yet - General MIDI music will be missing \
                     until the DOSBox support files finish downloading",
                    sf
                );
            }
        }
    }

    let mut result = out.join("\n");
    result.push('\n');
    result
}

/// DOSBox-X's `[ide]` section becomes Staging's `-ide` flag on CD imgmounts
/// (slot form `-ide 2m` normalized): a guest booted from an HDD image reaches
/// the CD only through its own ATAPI driver (§15).
fn translate_ide_for_staging(conf: &str) -> String {
    // Comment lines are skipped, same as conf_requests_printer: one eXoWin3x
    // conf carries the whole option documentation as `#` comments.
    let has_ide_section = conf
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .any(|l| l.to_ascii_lowercase().starts_with("[ide"));
    if !has_ide_section {
        return conf.to_string();
    }
    let mut out: Vec<String> = Vec::with_capacity(conf.lines().count());
    for line in conf.lines() {
        let lower = line.to_ascii_lowercase();
        let cmd = lower.trim_start().trim_start_matches('@');
        // Token-wise `-t cdrom|iso` detection - a doubled space between the
        // flag and its value must not hide a CD mount from the translation.
        let toks: Vec<&str> = cmd.split_whitespace().collect();
        let is_cd_imgmount = cmd.starts_with("imgmount")
            && toks
                .windows(2)
                .any(|w| w[0] == "-t" && (w[1] == "cdrom" || w[1] == "iso"));
        if !is_cd_imgmount {
            out.push(line.to_string());
            continue;
        }
        // Standalone flag only: a data dir like `/mnt/games-ide/` must not
        // read as "already present". Lowercasing keeps byte offsets.
        if let Some(pos) = find_ide_flag(&lower) {
            let end = pos + "-ide".len();
            let rest = &line[end..];
            let after_ws = rest.trim_start();
            let token: String = after_ws.chars().take_while(|c| !c.is_whitespace()).collect();
            // DOSBox-X slot argument: "2m", "1s", "2" - short, digit-first.
            let is_slot = !token.is_empty()
                && token.len() <= 2
                && token.chars().next().is_some_and(|c| c.is_ascii_digit());
            if is_slot {
                out.push(format!("{}{}", &line[..end], &after_ws[token.len()..]));
            } else {
                out.push(line.to_string());
            }
        } else {
            out.push(format!("{} -ide", line.trim_end()));
        }
    }
    let mut result = out.join("\n");
    result.push('\n');
    result
}

/// Byte offset of a standalone `-ide` flag token: preceded by whitespace and
/// followed by whitespace or end-of-line. Substring hits inside path segments
/// or filenames (`/mnt/games-ide/`, `T-IDE.iso`) don't count.
fn find_ide_flag(lower: &str) -> Option<usize> {
    let bytes = lower.as_bytes();
    let mut start = 0;
    while let Some(p) = lower[start..].find("-ide") {
        let pos = start + p;
        let end = pos + "-ide".len();
        let before_ok = pos > 0 && bytes[pos - 1].is_ascii_whitespace();
        let after_ok = end >= lower.len() || bytes[end].is_ascii_whitespace();
        if before_ok && after_ok {
            return Some(pos);
        }
        start = end;
    }
    None
}

/// Launch command for an LP game whose layout differs from EN: the EN
/// autoexec's command if it exists here, else run.bat's target, else an
/// executable. Returns (subdir, command).
fn find_lp_launch(game_dir: &std::path::Path, en_conf: Option<&str>) -> Option<(String, String)> {
    // Strategy 0: the EN autoexec names the launcher - the only signal that
    // works for a bare root-level EXE without a .bat (Cobra Mission ES).
    if let Some(autoexec) = en_conf.and_then(|c| c.split("[autoexec]").nth(1)) {
        for line in autoexec.lines() {
            let t = line.trim();
            let t = t.strip_prefix('@').unwrap_or(t).trim();
            let t = t
                .strip_prefix("call ")
                .or_else(|| t.strip_prefix("CALL "))
                .unwrap_or(t)
                .trim();
            if t.is_empty() {
                continue;
            }
            let lower = t.to_ascii_lowercase();
            let is_drive_switch = lower.len() == 2
                && lower.as_bytes()[1] == b':'
                && lower.as_bytes()[0].is_ascii_alphabetic();
            let is_housekeeping = is_drive_switch
                || lower.starts_with('#')
                || ["mount ", "imgmount ", "echo ", "rem ", "cd ", "cd\\", "set "]
                    .iter()
                    .any(|p| lower.starts_with(p))
                || ["cls", "cd", "exit", "pause", "echo", "echo."]
                    .contains(&lower.as_str());
            if is_housekeeping {
                continue;
            }
            let base = t.split_whitespace().next().unwrap_or(t);
            let base_lower = base.to_ascii_lowercase();
            if let Ok(entries) = std::fs::read_dir(game_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let name_lower = entry.file_name().to_string_lossy().to_ascii_lowercase();
                    let runnable = name_lower.ends_with(".exe")
                        || name_lower.ends_with(".com")
                        || name_lower.ends_with(".bat");
                    let stem = name_lower.rsplitn(2, '.').last().unwrap_or(&name_lower);
                    if runnable && (stem == base_lower || name_lower == base_lower) {
                        log::info!(
                            "LP launch: using EN autoexec command '{}' (found {})",
                            t,
                            entry.file_name().to_string_lossy()
                        );
                        return Some((String::new(), t.to_string()));
                    }
                }
            }
            // Only the FIRST real command is the launch line; later lines
            // (cleanup, exit chains) must not be mistaken for it.
            break;
        }
    }

    let mut search_dirs: Vec<(String, std::path::PathBuf)> =
        vec![("".to_string(), game_dir.to_path_buf())];

    if let Ok(entries) = std::fs::read_dir(game_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.path().is_dir() {
                search_dirs.push((
                    entry.file_name().to_string_lossy().to_string(),
                    entry.path(),
                ));
            }
        }
    }

    // Strategy 1: Parse run.bat to find the real executable
    for (subdir, dir) in &search_dirs {
        let run_bat = dir.join("run.bat");
        if let Ok(content) = std::fs::read_to_string(&run_bat) {
            // Look for "@call <program>" or just "<program>" lines that reference
            // a .com/.exe/.bat that exists in the directory
            for line in content.lines() {
                let trimmed = line.trim();
                let cmd = trimmed
                    .strip_prefix("@call ")
                    .or_else(|| trimmed.strip_prefix("@CALL "))
                    .or_else(|| trimmed.strip_prefix("@"))
                    .unwrap_or(trimmed);
                let cmd = cmd.trim();
                let cmd_lower = cmd.to_ascii_lowercase();

                // Skip control flow, echo, copy, config, choice, labels, etc.
                let skip_prefixes = [
                    ":", "echo", "cls", "copy", "config", "choice",
                    "if ", "goto", "exit", "rem ", "set ", "pause",
                ];
                if cmd.is_empty() || skip_prefixes.iter().any(|p| cmd_lower.starts_with(p)) {
                    continue;
                }

                // Check if this command corresponds to an actual file in the game dir
                let base = cmd.split_whitespace().next().unwrap_or(cmd);
                // Search directory for a case-insensitive match
                if let Ok(entries) = std::fs::read_dir(dir) {
                    let base_lower = base.to_ascii_lowercase();
                    for entry in entries.filter_map(|e| e.ok()) {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let name_lower = name.to_ascii_lowercase();
                        let stem = name_lower.rsplitn(2, '.').last().unwrap_or(&name_lower);
                        if stem == base_lower || name_lower == base_lower {
                            log::info!("LP launch: found '{}' via run.bat in {}", base, subdir);
                            return Some((subdir.clone(), base.to_string()));
                        }
                    }
                }
            }
        }
    }

    // Strategy 2: Look for any .bat file that calls an exe/com (skip known utility names).
    // Returns the .bat itself as the command so all its steps run in sequence.
    const SKIP_BAT_STEMS: &[&str] = &[
        "anleit", "readme", "install", "setup", "help", "manual",
        "problem", "config", "uninstal", "uninst",
    ];
    for (subdir, dir) in &search_dirs {
        let dir_stem = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let mut candidates: Vec<String> = if let Ok(entries) = std::fs::read_dir(dir) {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_lowercase();
                    name.ends_with(".bat")
                        && name != "run.bat"
                        && !SKIP_BAT_STEMS.iter().any(|s| name.starts_with(s))
                })
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        } else {
            vec![]
        };

        // Prefer .bat whose stem matches the directory name
        candidates.sort_by_key(|b| {
            let stem = b.rsplitn(2, '.').last().unwrap_or(b).to_lowercase();
            usize::from(stem != dir_stem)
        });

        for bat in &candidates {
            let bat_path = dir.join(bat);
            if let Ok(content) = std::fs::read_to_string(&bat_path) {
                let has_exe_call = content.lines().any(|line| {
                    let l = line.trim().to_ascii_lowercase();
                    !l.is_empty()
                        && !l.starts_with(':')
                        && !l.starts_with("rem ")
                        && (l.contains(".exe") || l.contains(".com"))
                });
                if has_exe_call {
                    log::info!("LP launch: found .bat launcher '{}' in '{}'", bat, subdir);
                    return Some((subdir.clone(), bat.clone()));
                }
            }
        }
    }

    // Strategy 3: Look for a .com file (more likely to be a DOS game than .exe)
    for (subdir, dir) in &search_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.ends_with(".com") && !name.contains("mouse") {
                    return Some((
                        subdir.clone(),
                        entry.file_name().to_string_lossy().to_string(),
                    ));
                }
            }
        }
    }

    // Strategy 4: an .exe in a subdirectory, then at the root (skipping
    // utilities and installers).
    const SKIP_EXE_STEMS: &[&str] = &[
        "install", "setup", "uninst", "config", "cdtest", "showtext",
        // DOS/4GW and protected-mode extenders - not the game itself
        "rtm", "dos4gw", "dpmi", "cwsdpmi",
    ];
    let subdirs_then_root = search_dirs
        .iter()
        .filter(|(s, _)| !s.is_empty())
        .chain(search_dirs.iter().filter(|(s, _)| s.is_empty()));
    for (subdir, dir) in subdirs_then_root {
        let dir_stem = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let mut exes: Vec<String> = if let Ok(entries) = std::fs::read_dir(dir) {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_lowercase();
                    name.ends_with(".exe")
                        && !SKIP_EXE_STEMS.iter().any(|s| name.starts_with(s))
                })
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        } else {
            vec![]
        };

        // Prefer exe whose stem matches the directory name
        exes.sort_by_key(|e| {
            let stem = e.rsplitn(2, '.').last().unwrap_or(e).to_lowercase();
            usize::from(stem != dir_stem)
        });

        if let Some(exe) = exes.first() {
            log::info!("LP launch: found .exe '{}' in '{}'", exe, subdir);
            return Some((subdir.clone(), exe.clone()));
        }
    }

    None
}

/// Returns true if the [autoexec] section of a dosbox conf contains at least one
/// line that looks like an actual game launch command (not just mounts, drive switches,
/// comments, or housekeeping).
fn autoexec_has_launch_cmd(conf: &str) -> bool {
    let autoexec = match conf.split("[autoexec]").nth(1) {
        Some(s) => s,
        None => return false,
    };
    autoexec.lines().any(|line| {
        let l = line.trim().to_ascii_lowercase();
        if l.is_empty() || l.starts_with('#') || l.starts_with("rem ") {
            return false;
        }
        // Drive-switch: single letter followed by colon (a: through z:)
        let is_drive_switch = l.len() >= 2
            && l.as_bytes()[1] == b':'
            && l.as_bytes()[0].is_ascii_alphabetic();
        if is_drive_switch {
            return false;
        }
        const NON_LAUNCH: &[&str] = &[
            "@echo", "@exit", "echo ", "mount ", "imgmount", "exit", "cls",
        ];
        !NON_LAUNCH.iter().any(|p| l.starts_with(p))
    })
}

#[cfg(test)]
mod tests {

    /// Unset is Staging's own default, and the old switch's "default" was Off.
    #[test]
    fn staging_shader_reads_old_and_new_values() {
        use super::staging_shader;
        assert_eq!(staging_shader(None), "crt-auto");
        assert_eq!(staging_shader(Some("")), "crt-auto");
        assert_eq!(staging_shader(Some("default")), "sharp");
        assert_eq!(staging_shader(Some("crt-auto")), "crt-auto");
        assert_eq!(staging_shader(Some("scaler/xbr-lv3")), "scaler/xbr-lv3");
    }

    /// Shaders need OpenGL and no scaler; scalers are forced; the game's
    /// choice wins and "none" turns the global one off.
    #[test]
    fn dosbox_x_filter_fragment() {
        use super::{dosbox_x_filter_conf, FILTER_NONE};
        assert_eq!(dosbox_x_filter_conf(None, None), None);
        let crt = dosbox_x_filter_conf(None, Some("crt-lottes")).unwrap();
        assert!(crt.contains("output = opengl") && crt.contains("scaler = none") && crt.contains("glshader = crt-lottes"));
        let hq = dosbox_x_filter_conf(Some("scaler/hq2x"), Some("crt-lottes")).unwrap();
        assert!(hq.contains("scaler = hq2x forced") && hq.contains("glshader = none"));
        assert_eq!(dosbox_x_filter_conf(Some(FILTER_NONE), Some("crt-lottes")), None);
        assert_eq!(dosbox_x_filter_conf(None, Some("not-a-filter")), None);
    }

    /// The emulator's view of a mounted host directory is case-insensitive,
    /// so the probe's must be too. Only Linux can fail this - which is where
    /// CI caught it - but the assertion holds on every platform.
    #[test]
    fn a_cd_target_resolves_regardless_of_case() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("ODYSSEY")).unwrap();
        let found = resolve_ignoring_case(&tmp.path().join("odyssey"))
            .expect("a differently-cased directory must still resolve");
        assert!(found.is_dir(), "{}", found.display());
        assert!(resolve_ignoring_case(&tmp.path().join("nothing-here")).is_none());
    }

    /// eXo mounts the game directory as C: and then cds into a subdirectory
    /// of it. Reading the mount is what makes that `cd` resolvable; without
    /// it every such conf was rejected and the launch fell back to the
    /// generated autoexec (Alien Odyssey DE).
    #[test]
    fn lp_probe_follows_the_mount_target_into_a_subdirectory() {
        let tmp = tempfile::tempdir().unwrap();
        let lp = tmp.path().join("!german/AlienOdy");
        std::fs::create_dir_all(lp.join("ODYSSEY")).unwrap();
        std::fs::write(lp.join("ODYSSEY/run.bat"), b"").unwrap();
        let conf = "[autoexec]\ncd ..\ncd ..\nmount c .\\eXoDOS\\AlienOdy\nc:\n@cd odyssey\n@call run\nexit\n";
        assert!(lp_autoexec_compatible(conf, "AlienOdy", &lp, tmp.path()));
    }

    /// The launch command still has to exist: a variant whose files were
    /// restructured must keep falling through to `find_lp_launch`.
    #[test]
    fn lp_probe_still_rejects_a_launch_command_the_variant_lacks() {
        let tmp = tempfile::tempdir().unwrap();
        let lp = tmp.path().join("!german/AlienOdy");
        std::fs::create_dir_all(lp.join("ODYSSEY")).unwrap();
        let conf = "[autoexec]\nmount c .\\eXoDOS\\AlienOdy\nc:\n@cd odyssey\n@call run\nexit\n";
        assert!(!lp_autoexec_compatible(conf, "AlienOdy", &lp, tmp.path()));
    }


    use super::*;
    use std::fs;

    // ── translate_midi_for_staging ───────────────────────────────────────────

    #[test]
    fn midi_translate_converts_ece_keys_to_staging_sections() {
        // Shape of ~1,500 real eXoDOS configs after path rewriting.
        let conf = "[sdl]\nfullscreen = true\n\
                    [midi]\nmididevice = mt32\nmpu401 = intelligent\n\
                    mt32.romdir = /data/eXo/mt32\n\
                    fluid.soundfont = /data/eXo/mt32/SoundCanvas.sf2\n\
                    fluid.gain = 0.4\n\
                    [autoexec]\nmount c /data/eXo/eXoDOS/SQ5\n";
        let out = translate_midi_for_staging(conf);

        // ECE dotted keys removed from [midi], Staging keys kept.
        assert!(!out.contains("mt32.romdir"));
        assert!(!out.contains("fluid.soundfont"));
        assert!(!out.contains("fluid.gain"));
        assert!(out.contains("mididevice = mt32"));
        assert!(out.contains("mpu401 = intelligent"));

        // Staging sections appended with the captured values.
        assert!(out.contains("[mt32]\nromdir = /data/eXo/mt32"));
        assert!(out.contains("[fluidsynth]\nsoundfont = /data/eXo/mt32/SoundCanvas.sf2"));

        // Autoexec untouched.
        assert!(out.contains("mount c /data/eXo/eXoDOS/SQ5"));
    }

    #[test]
    fn midi_translate_maps_default_device_to_auto() {
        let conf = "[midi]\nmididevice = default\nmt32.romdir = /x/mt32\n";
        let out = translate_midi_for_staging(conf);
        assert!(out.contains("mididevice = auto"));
        assert!(!out.contains("default"));
    }

    #[test]
    fn midi_translate_leaves_staging_native_configs_alone() {
        // Shape of the ~750 Staging-authored eXoDOS configs.
        let conf = "[midi]\nmididevice = auto\n\
                    [mt32]\nromdir = /data/eXo/mt32\n\
                    [fluidsynth]\nsoundfont = /data/eXo/mt32/SoundCanvas.sf2\n";
        let out = translate_midi_for_staging(conf);
        assert_eq!(out.matches("[mt32]").count(), 1);
        assert_eq!(out.matches("[fluidsynth]").count(), 1);
        assert!(out.contains("romdir = /data/eXo/mt32"));
    }

    #[test]
    fn midi_translate_no_midi_config_is_passthrough() {
        let conf = "[sdl]\nfullscreen = true\n[autoexec]\nrunme.exe\n";
        assert_eq!(translate_midi_for_staging(conf), conf);
    }

    // ── extract_game_zip / launch_zip_candidates ─────────────────────────────


    // ── patch_dosbox_conf ────────────────────────────────────────────────────

    fn write_conf(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    /// The autoexec's `path=C:\;z:\;c:\windows\` is guest text; rewriting
    /// its backslashes broke 1,122 eXoWin3x games at `runexit`.
    #[test]
    fn patch_dosbox_conf_keeps_guest_dos_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path();
        let game_dir = working_dir.join("eXoWin3x/20k3x/cd");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("cd.cue"), b"").unwrap();

        let conf_content = "[autoexec]\nmount c .\\eXoWin3x\\20k3x\n\
             imgmount d .\\eXoWin3x\\20k3x\\cd\\cd.cue -t cdrom\nc:\n\
             path=C:\\;z:\\;c:\\windows\\\n@cd 20000\n@win runexit 20000\nexit\n";
        let conf_path = write_conf(working_dir, "dosbox.conf", conf_content);

        let patched_path = patch_dosbox_conf(&conf_path, working_dir, None, true).unwrap();
        let patched = fs::read_to_string(&patched_path).unwrap();

        // Guest-side DOS text is untouched.
        assert!(
            patched.contains("path=C:\\;z:\\;c:\\windows\\"),
            "DOS PATH must keep its backslashes: {}", patched
        );
        assert!(patched.contains("@win runexit 20000"), "launch line intact: {}", patched);
        // Host paths still become absolute and forward-slashed.
        let abs = format!("{}/", working_dir.to_string_lossy()).replace('\\', "/");
        assert!(
            patched.contains(&format!("mount c {}eXoWin3x/20k3x", abs)),
            "mount must be absolute: {}", patched
        );
        assert!(
            patched.contains(&format!("{}eXoWin3x/20k3x/cd/cd.cue -t cdrom", abs)),
            "imgmount must be absolute: {}", patched
        );
    }

    /// 11th Hour (DE): `imgmount d ".\cd\11HDISK1.cue"` after `c:` is a
    /// GUEST path - no eXo/cd exists on the host.
    #[test]
    fn patch_dosbox_conf_keeps_guest_imgmount_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path();
        let cd_dir = working_dir.join("eXoDOS/11thHour/cd");
        fs::create_dir_all(&cd_dir).unwrap();
        fs::write(cd_dir.join("11HDISK1.cue"), b"").unwrap();

        let conf_content = "[autoexec]\necho off\nmount c .\\eXoDOS\\11thHour\nc:\n\
             imgmount d \".\\cd\\11HDISK1.cue\" -t iso\ngame.exe /9 .\\ .\\\n@call run\nexit\n";
        let conf_path = write_conf(working_dir, "dosbox.conf", conf_content);

        let patched_path = patch_dosbox_conf(&conf_path, working_dir, None, true).unwrap();
        let patched = fs::read_to_string(&patched_path).unwrap();

        let abs = format!("{}/", working_dir.to_string_lossy()).replace('\\', "/");
        assert!(
            patched.contains(&format!("mount c {}eXoDOS/11thHour", abs)),
            "existing mount target still becomes absolute: {}", patched
        );
        assert!(
            patched.contains("imgmount d \".\\cd\\11HDISK1.cue\" -t iso"),
            "guest-relative imgmount must stay as authored: {}", patched
        );
        // Bare `.\` is a guest argument (OxydGold) - the working dir itself
        // always exists, so it must be excluded from the existence gate.
        assert!(
            patched.contains("game.exe /9 .\\ .\\"),
            "bare .\\ arguments must stay as authored: {}", patched
        );
    }

    /// eXoWin3x IDE games: the DOSBox-X `[ide]` section becomes Staging's
    /// `-ide` imgmount flag - without it a guest booted from an HDD image
    /// never sees the CD (its ATAPI driver finds no controller).
    #[test]
    fn ide_translate_adds_flag_to_cd_imgmounts() {
        let conf = "[dosbox]\nmemsize=32\n[ide, primary] \nenable=true \n\
             [ide, secondary] \nenable=true \n[autoexec]\n@echo off\n\
             imgmount c game/i100_203.img\nimgmount d game/cd/cd.cue -t cdrom \n\
             boot -l c\nexit\n";
        let out = translate_ide_for_staging(conf);
        assert!(
            out.contains("imgmount d game/cd/cd.cue -t cdrom -ide"),
            "CD imgmount must gain -ide: {}", out
        );
        // The HDD imgmount is not a CD mount - Staging's -ide only applies to CD drives.
        assert!(out.contains("imgmount c game/i100_203.img\n"), "hdd imgmount untouched: {}", out);
    }

    /// 6 eXoWin3x configs already carry the flag in DOSBox-X's argument form
    /// (`-ide 2m` = secondary master); Staging's flag takes no argument.
    #[test]
    fn ide_translate_normalizes_dosbox_x_slot_argument() {
        let conf = "[ide, secondary]\nenable=true\n[autoexec]\n\
             imgmount d \"game/cd/A Title (Pub).ISO\" -t iso -fs iso -ide 2m\nboot -l c\n";
        let out = translate_ide_for_staging(conf);
        assert!(
            out.contains("imgmount d \"game/cd/A Title (Pub).ISO\" -t iso -fs iso -ide\n"),
            "slot argument must be dropped: {}", out
        );
        assert!(!out.contains("-ide 2m"), "DOSBox-X form must not survive: {}", out);
    }

    /// Configs without an [ide] section keep their imgmounts as authored -
    /// 483 non-IDE Win3x games mount .cue sheets that work fine without a
    /// controller, and forcing one on them changes tested behavior.
    #[test]
    fn ide_translate_leaves_non_ide_configs_alone() {
        let conf = "[dosbox]\nmemsize=32\n[autoexec]\n\
             imgmount d game/cd/cd.cue -t cdrom\nwin runexit GAME\n";
        assert_eq!(translate_ide_for_staging(conf), conf);
        // Commented-out [ide] documentation (the TheCHAOS pattern) is not a
        // request for a controller either.
        let commented = "[dosbox]\n# [ide, primary] docs only\n[autoexec]\n\
             imgmount d game/cd/cd.cue -t cdrom\n";
        assert_eq!(translate_ide_for_staging(commented), commented);
    }

    /// The line holds an absolute REWRITTEN host path when this runs - `-ide`
    /// inside a path segment must not read as "flag already present", or a
    /// data dir like /mnt/games-ide/ silently disables the translation.
    #[test]
    fn ide_translate_ignores_ide_inside_paths() {
        let conf = "[ide, primary]\nenable=true\n[autoexec]\n\
             imgmount d /mnt/games-ide/eXoWin3x/T-IDE.iso -t iso\nboot -l c\n";
        let out = translate_ide_for_staging(conf);
        assert!(
            out.contains("imgmount d /mnt/games-ide/eXoWin3x/T-IDE.iso -t iso -ide\n"),
            "flag must still be appended: {}", out
        );
        // Doubled space between -t and its value is still a CD mount.
        let spaced = "[ide, primary]\nenable=true\n[autoexec]\n\
             imgmount d game/cd.cue -t  cdrom\n";
        assert!(
            translate_ide_for_staging(spaced).contains("-t  cdrom -ide\n"),
            "double-spaced -t value must still translate"
        );
    }

    /// Every collection resolves inside the single root; LP rows fall back
    /// to the lang-scoped conf when the EN path is absent.
    #[test]
    fn resolve_game_conf_probe_order() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().to_string_lossy().into_owned();
        let rel = "eXo/eXoWin3x/!win3x/GeoGeo/dosbox.conf";
        let root = tmp.path().join(crate::commands::paths::DEFAULT_ROOT_FOLDER);

        // Nothing on disk: no result.
        assert!(resolve_game_conf(&data_dir, rel).is_none());

        // A Win3x conf is found in the one root, not in a tree of its own.
        let conf_path = root.join(rel);
        fs::create_dir_all(conf_path.parent().unwrap()).unwrap();
        fs::write(&conf_path, "[autoexec]\n").unwrap();
        let (conf, found_root) = resolve_game_conf(&data_dir, rel).unwrap();
        assert_eq!(conf, conf_path);
        assert_eq!(found_root, root);

        // Lang-scoped alternate (LP rows): conf only under a language subdir.
        let lang_conf = root.join("eXo/eXoDOS/!dos/!german/DasAmt/dosbox.conf");
        fs::create_dir_all(lang_conf.parent().unwrap()).unwrap();
        fs::write(&lang_conf, "[autoexec]\n").unwrap();
        let (conf, found_root) =
            resolve_game_conf(&data_dir, "eXo/eXoDOS/!dos/DasAmt/dosbox.conf")
                .unwrap();
        assert_eq!(conf, lang_conf);
        assert_eq!(found_root, root);
    }

    // ── conf_requests_printer ───────────────────────────────────────────────

    #[test]
    fn exo_picks_dosbox_x_for_its_x_builds_and_for_printing() {
        use super::{chosen_engine, exo_engine, DosEngine::*};
        assert_eq!(exo_engine(Some("x"), false), DosboxX);
        assert_eq!(exo_engine(Some("x2"), false), DosboxX);
        // Laffer Utilities: pinned to ECE, but it prints.
        assert_eq!(exo_engine(Some("ece4230"), true), DosboxX);
        for v in [None, Some("dosbox"), Some("staging0.81.1"), Some("ece4230"), Some("svn"), Some("x98")] {
            assert_eq!(exo_engine(v, false), Staging, "{v:?}");
        }
        // The per-game setting wins both ways; anything else is eXo's pick.
        assert_eq!(chosen_engine(Some("staging"), Some("x"), true), Staging);
        assert_eq!(chosen_engine(Some("dosbox-x"), Some("dosbox"), false), DosboxX);
        assert_eq!(chosen_engine(Some(""), Some("x"), false), DosboxX);
        assert_eq!(chosen_engine(Some("ece"), Some("dosbox"), false), Staging);
        assert_eq!(chosen_engine(None, None, false), Staging);
    }

    #[test]
    fn printouts_go_to_the_games_own_folder_as_png() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("eXo/eXoDOS/NewPS");
        let frag = super::printer_fragment(dir.path(), 7, &game).unwrap();
        let text = fs::read_to_string(frag).unwrap();
        assert!(text.starts_with("[printer]\n"));
        assert!(text.contains("printoutput = png"));
        assert!(text.contains(&format!("docpath = {}", game.join("!prints").display())));
        assert!(game.join("!prints").is_dir(), "created before launch");
    }

    #[test]
    fn printer_detection_matches_enabled_not_documentation() {
        // The 13 eXoDOS printer titles set both keys.
        assert!(conf_requests_printer("[parallel]\nparallel1=printer\n[printer]\nprinter=true\nprintoutput=printer\n"));
        // TheCHAOS (eXoWin3x) has the whole option documentation as comments
        // but disables the port - must NOT match.
        assert!(!conf_requests_printer(
            "[parallel]\nparallel1=disabled\nparallel2=disabled\n\
             # parallel1: parallel1-3 -- set type of device connected to lpt port.\n\
             #               printer (virtual dot-matrix printer, see [printer] section)\n"
        ));
    }

    #[test]
    fn patch_dosbox_conf_converts_windows_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path();
        fs::create_dir_all(working_dir.join("eXoDOS/SQ5")).unwrap();

        let conf_content = "[sdl]\nfullscreen=false\n[autoexec]\n@mount c .\\eXoDOS\\SQ5\nc:\nSQ5.bat\nexit\n";
        let conf_path = write_conf(working_dir, "dosbox.conf", conf_content);

        let patched_path = patch_dosbox_conf(&conf_path, working_dir, None, true).unwrap();
        let patched = fs::read_to_string(&patched_path).unwrap();

        // Backslash replaced with forward slash
        assert!(!patched.contains('\\'), "no backslashes should remain: {}", patched);
        // Relative .\ prefix replaced with absolute working dir. On Windows
        // the working dir itself contains backslashes, which the patcher
        // normalizes to forward slashes - normalize the expectation too.
        let abs_prefix = format!("{}/", working_dir.to_string_lossy()).replace('\\', "/");
        assert!(patched.contains(&abs_prefix), "absolute path prefix expected: {}", patched);
    }

    #[test]
    fn mount_targets_are_matched_ignoring_case() {
        // After Dark 3.2: the conf mounts `.\eXoWin3x\Adark3` and images
        // `.\eXoWin3x\Adark3\cd\ADW320_C.ISO`; the archive unpacked `adark3`.
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path();
        fs::create_dir_all(working_dir.join("eXoWin3x/adark3/cd")).unwrap();
        fs::write(working_dir.join("eXoWin3x/adark3/cd/ADW320_C.ISO"), b"").unwrap();

        let conf_content = "[autoexec]\nmount c .\\eXoWin3x\\Adark3\nimgmount d .\\eXoWin3x\\Adark3\\cd\\ADW320_C.ISO -t cdrom\nc:\nexit\n";
        let conf_path = write_conf(working_dir, "dosbox.conf", conf_content);
        let patched = fs::read_to_string(patch_dosbox_conf(&conf_path, working_dir, None, true).unwrap()).unwrap();

        let dir = working_dir.join("eXoWin3x/adark3").to_string_lossy().replace('\\', "/");
        assert!(patched.contains(&format!("mount c {dir}\n")), "on-disk spelling expected: {patched}");
        assert!(patched.contains(&format!("imgmount d {dir}/cd/ADW320_C.ISO -t cdrom")), "{patched}");
        assert!(!patched.contains(".\\"), "no Windows-relative path may survive: {patched}");
    }

    #[test]
    fn bat_host_paths_are_matched_ignoring_case() {
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path();
        let game = working_dir.join("eXoDOS/game1");
        fs::create_dir_all(game.join("cd")).unwrap();
        fs::write(game.join("cd/DISC1.CUE"), b"").unwrap();
        fs::write(game.join("run.bat"), b"imgmount d .\\eXoDOS\\GAME1\\CD\\DISC1.CUE -t cdrom\r\n").unwrap();

        rewrite_bat_host_paths(&game, working_dir);
        let out = fs::read_to_string(game.join("run.bat")).unwrap();
        assert!(out.contains("imgmount d ./eXoDOS/game1/cd/DISC1.CUE -t cdrom"), "{out}");
    }

    #[test]
    fn mount_targets_keep_no_trailing_separator() {
        // `mount c .\eXoDOS\` (1,570 confs): the trailing separator must go.
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path();
        fs::create_dir_all(working_dir.join("eXoDOS/DOOMII")).unwrap();

        let conf_content = "[autoexec]\nmount c .\\eXoDOS\\\nc:\n@cd DOOMII\n@call run\nexit\n";
        let conf_path = write_conf(working_dir, "dosbox.conf", conf_content);

        let patched_path = patch_dosbox_conf(&conf_path, working_dir, None, true).unwrap();
        let patched = fs::read_to_string(&patched_path).unwrap();

        let mount_line = patched
            .lines()
            .find(|l| l.contains("mount c "))
            .expect("mount line survives");
        assert!(
            !mount_line.trim_end().ends_with('/'),
            "mount target must not end in a separator: {}",
            mount_line
        );
        let expected = format!("{}/eXoDOS", working_dir.to_string_lossy()).replace('\\', "/");
        assert!(
            mount_line.contains(&expected),
            "mount should point at the eXoDOS root: {}",
            mount_line
        );
    }

    #[test]
    fn a_data_dir_with_spaces_gets_the_mount_argument_quoted() {
        // eXo writes the mount target unquoted because `.\eXoDOS\SQ5` has no
        // spaces. The substituted host path can - DOSBox would then mount
        // everything up to the first one.
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path().join("My Games/eXo");
        fs::create_dir_all(working_dir.join("eXoDOS/SQ5")).unwrap();

        let conf_content =
            "[midi]\nfluid.soundfont=.\\mt32\\SoundCanvas.sf2\n[autoexec]\n@mount c .\\eXoDOS\\SQ5\nc:\nSQ5.bat\nexit\n";
        fs::create_dir_all(working_dir.join("mt32")).unwrap();
        fs::write(working_dir.join("mt32/SoundCanvas.sf2"), b"").unwrap();
        let conf_path = write_conf(&working_dir, "dosbox.conf", conf_content);

        let patched_path = patch_dosbox_conf(&conf_path, &working_dir, None, false).unwrap();
        let patched = fs::read_to_string(&patched_path).unwrap();

        let mount_line = patched.lines().find(|l| l.contains("mount c ")).unwrap();
        assert!(
            mount_line.contains("\"") && mount_line.trim_end().ends_with('"'),
            "mount argument with spaces must be quoted: {}",
            mount_line
        );
        // A config property is read verbatim - quotes there would land in the
        // path itself.
        let sf_line = patched
            .lines()
            .find(|l| l.starts_with("fluid.soundfont="))
            .unwrap();
        assert!(
            !sf_line.contains('"'),
            "config values stay unquoted: {}",
            sf_line
        );
    }

    /// C&C's run.bat mounts its discs by a host path relative to DOSBox's
    /// cwd; only that token changes, guest text and a missing target stay.
    #[test]
    fn rewrite_bat_host_paths_turns_existing_targets_into_forward_slashes() {
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path();
        let game_dir = working_dir.join("eXoDOS/comcon");
        fs::create_dir_all(game_dir.join("cd")).unwrap();
        fs::write(game_dir.join("cd/CD-1.iso"), b"").unwrap();
        let run_bat = game_dir.join("run.bat");
        let content = b"@echo off\r\n\
            imgmount d \".\\eXoDOS\\comcon\\cd\\CD-1.iso\" \".\\eXoDOS\\comcon\\cd\\CD-9.iso\" -t cdrom\r\n\
            game.exe .\\\r\n\
            cd \\\r\n\xC9\xCD\xBB\r\n";
        fs::write(&run_bat, content).unwrap();

        rewrite_bat_host_paths(&game_dir, working_dir);
        let out = fs::read(&run_bat).unwrap();
        let has = |needle: &[u8]| out.windows(needle.len()).any(|w| w == needle);

        let shown = String::from_utf8_lossy(&out);
        assert!(has(b"\"./eXoDOS/comcon/cd/CD-1.iso\""), "existing target rewritten: {}", shown);
        assert!(has(b"\".\\eXoDOS\\comcon\\cd\\CD-9.iso\""), "missing target untouched: {}", shown);
        assert!(has(b"game.exe .\\\r\n"), "bare .\\ is guest text: {}", shown);
        assert!(has(b"cd \\\r\n"), "guest cd stays: {}", shown);
        assert!(has(b"\xC9\xCD\xBB"), "CP437 bytes survive");

        // Idempotent: a second pass writes nothing.
        let mtime = fs::metadata(&run_bat).unwrap().modified().unwrap();
        rewrite_bat_host_paths(&game_dir, working_dir);
        assert_eq!(fs::metadata(&run_bat).unwrap().modified().unwrap(), mtime);
    }

    #[test]
    fn patch_dosbox_conf_lp_overlay_direct_mount() {
        // EN conf mounts the game dir directly: mount target must be routed
        // through the overlay staging dir whose link points at the LP dir.
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path();
        let lp_dir = working_dir.join("eXoDOS/!german/SQ5");
        fs::create_dir_all(&lp_dir).unwrap();
        fs::write(lp_dir.join("SQ5.BAT"), b"").unwrap();

        let conf_content = "[autoexec]\n@mount c .\\eXoDOS\\SQ5\nc:\nSQ5.bat\nexit\n";
        let conf_path = write_conf(working_dir, "dosbox.conf", conf_content);

        let patched_path = patch_dosbox_conf(
            &conf_path,
            working_dir,
            Some(("SQ5", "!german", "eXoDOS", &lp_dir)),
            true,
        )
        .unwrap();
        let patched = fs::read_to_string(&patched_path).unwrap();

        assert!(
            patched.contains(".exorchy_lp/german_SQ5"),
            "mount should be routed through the overlay: {}",
            patched
        );
        assert!(patched.contains("SQ5.bat"), "launch command must survive: {}", patched);
        // The overlay link resolves to the LP dir.
        let linked = working_dir.join(".exorchy_lp/german_SQ5/SQ5");
        assert!(linked.join("SQ5.BAT").exists(), "overlay link should reach LP files");
    }

    #[test]
    fn patch_dosbox_conf_lp_overlay_root_mount_cd() {
        // Cobra Mission (ES) shape: EN conf mounts the eXoDOS root and cd's
        // into the game dir; the LP dir holds a bare root-level EXE.
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path();
        fs::create_dir_all(working_dir.join("eXoDOS")).unwrap();
        let lp_dir = working_dir.join("eXoDOS/!spanish/cobmiss");
        fs::create_dir_all(&lp_dir).unwrap();
        fs::write(lp_dir.join("CM.EXE"), b"").unwrap();

        let conf_content =
            "[autoexec]\n@mount c .\\eXoDOS\\\nc:\ncls\ncd cobmiss\n@cm\nexit\n";
        let conf_path = write_conf(working_dir, "dosbox.conf", conf_content);

        let patched_path = patch_dosbox_conf(
            &conf_path,
            working_dir,
            Some(("cobmiss", "!spanish", "eXoDOS", &lp_dir)),
            true,
        )
        .unwrap();
        let patched = fs::read_to_string(&patched_path).unwrap();

        assert!(
            patched.contains(".exorchy_lp/spanish_cobmiss"),
            "root mount should be routed through the overlay: {}",
            patched
        );
        // The authored launch sequence survives verbatim.
        assert!(patched.contains("cd cobmiss"), "{}", patched);
        assert!(patched.contains("@cm"), "{}", patched);
        // And the overlay resolves cd cobmiss -> LP files.
        let linked = working_dir.join(".exorchy_lp/spanish_cobmiss/cobmiss");
        assert!(linked.join("CM.EXE").exists(), "overlay link should reach LP files");
    }

    #[test]
    fn patch_dosbox_conf_lp_falls_back_when_exe_renamed() {
        // LP variant renamed the executable: the EN launch command can't be
        // validated, so the generated-autoexec fallback must kick in and
        // find the actual root-level EXE.
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path();
        fs::create_dir_all(working_dir.join("eXoDOS")).unwrap();
        let lp_dir = working_dir.join("eXoDOS/!spanish/cobmiss");
        fs::create_dir_all(&lp_dir).unwrap();
        fs::write(lp_dir.join("JUEGO.EXE"), b"").unwrap();

        let conf_content =
            "[autoexec]\n@mount c .\\eXoDOS\\\nc:\ncd cobmiss\n@cm\nexit\n";
        let conf_path = write_conf(working_dir, "dosbox.conf", conf_content);

        let patched_path = patch_dosbox_conf(
            &conf_path,
            working_dir,
            Some(("cobmiss", "!spanish", "eXoDOS", &lp_dir)),
            true,
        )
        .unwrap();
        let patched = fs::read_to_string(&patched_path).unwrap();

        assert!(
            patched.to_ascii_lowercase().contains("juego.exe"),
            "fallback should launch the real executable: {}",
            patched
        );
        assert!(
            patched.contains("!spanish/cobmiss"),
            "fallback mounts the LP dir directly: {}",
            patched
        );
    }

    // ── find_lp_launch ───────────────────────────────────────────────────────

    #[test]
    fn find_lp_launch_parses_run_bat() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();

        // Create the target executable so the directory scan finds it
        fs::write(game_dir.join("sq5.exe"), b"").unwrap();

        let run_bat = "@call sq5.exe\n";
        fs::write(game_dir.join("run.bat"), run_bat).unwrap();

        let result = find_lp_launch(game_dir, None);
        assert!(result.is_some(), "run.bat parsing should find a launch command");
        let (subdir, cmd) = result.unwrap();
        assert_eq!(subdir, "", "game is in root of game_dir");
        assert_eq!(cmd, "sq5.exe");
    }

    #[test]
    fn find_lp_launch_finds_com_file_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();

        // No run.bat, but a .com file exists
        fs::write(game_dir.join("game.com"), b"").unwrap();

        let result = find_lp_launch(game_dir, None);
        assert!(result.is_some(), ".com file should be found as fallback");
        let (_, cmd) = result.unwrap();
        assert!(cmd.to_lowercase().ends_with(".com"));
    }

    #[test]
    fn find_lp_launch_returns_none_for_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(find_lp_launch(tmp.path(), None).is_none());
    }

    #[test]
    fn find_lp_launch_uses_en_autoexec_command() {
        // Regression: Cobra Mission (ES) - bare root-level CM.EXE plus
        // INSTALL.EXE, no .bat. The EN autoexec names the launcher.
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();
        fs::write(game_dir.join("CM.EXE"), b"").unwrap();
        fs::write(game_dir.join("INSTALL.EXE"), b"").unwrap();
        fs::write(game_dir.join("DAT.VOL"), b"").unwrap();

        let en_conf = "[sdl]\nfullscreen=false\n[autoexec]\n\
                       @mount c .\\eXoDOS\\\nc:\ncls\ncd cobmiss\n@cm\nexit\n";
        let (subdir, cmd) = find_lp_launch(game_dir, Some(en_conf)).unwrap();
        assert_eq!(subdir, "");
        assert_eq!(cmd, "cm");
    }

    #[test]
    fn find_lp_launch_falls_back_to_root_exe() {
        // No EN hint, no .bat/.com: the root-level EXE must still be found
        // (installers are skipped).
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();
        fs::write(game_dir.join("CM.EXE"), b"").unwrap();
        fs::write(game_dir.join("INSTALL.EXE"), b"").unwrap();

        let (subdir, cmd) = find_lp_launch(game_dir, None).unwrap();
        assert_eq!(subdir, "");
        assert_eq!(cmd.to_ascii_lowercase(), "cm.exe");
    }

    #[test]
    fn find_lp_launch_en_hint_ignores_missing_program() {
        // EN autoexec references a program the LP dir doesn't have -
        // must fall through to the heuristics, not return a broken command.
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();
        fs::write(game_dir.join("game.com"), b"").unwrap();

        let en_conf = "[autoexec]\nmount c .\\eXoDOS\\\nc:\ncd foo\n@other\nexit\n";
        let (_, cmd) = find_lp_launch(game_dir, Some(en_conf)).unwrap();
        assert_eq!(cmd.to_ascii_lowercase(), "game.com");
    }

    /// eXoWin3x's catalogue spells the pack folder `eXoWin3X` while the torrent
    /// writes `eXoWin3x`. The conf must still be found on a case-sensitive
    /// filesystem, and the resolved path must be the on-disk spelling.
    #[test]
    fn game_conf_is_found_when_the_catalogue_differs_in_case() {
        let dir = std::env::temp_dir().join(format!("exorchy_conf_case_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let conf_dir = dir.join("eXoDOS/eXo/eXoWin3x/!win3x/SimC23x");
        std::fs::create_dir_all(&conf_dir).unwrap();
        std::fs::write(conf_dir.join("dosbox.conf"), "[sdl]\n").unwrap();
        crate::commands::paths::set_root_folder(crate::commands::paths::DEFAULT_ROOT_FOLDER);

        let (conf, root) = super::resolve_game_conf(
            &dir.to_string_lossy(),
            r"eXo\eXoWin3X\!win3x\SimC23x/dosbox.conf",
        )
        .expect("conf resolves despite the case difference");
        assert_eq!(conf, conf_dir.join("dosbox.conf"));
        assert_eq!(root, dir.join("eXoDOS"));
        assert!(super::resolve_game_conf(&dir.to_string_lossy(), r"eXo\eXoWin3X\!win3x\Nope/dosbox.conf").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
