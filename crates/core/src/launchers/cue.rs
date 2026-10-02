//! CD images on a case-sensitive filesystem. eXo's cue sheets were written
//! on Windows, where `FILE "OUT-SIMCOPTER.BIN"` finds `out-simcopter.bin`.
//! On Linux DOSBox-X (and Staging) look the name up as written, find
//! nothing, and mount an empty drive ("No. of data tracks=0"): the game asks
//! for its CD. Before a launch, every cue the conf `IMGMOUNT`s gets a symlink
//! for each track file whose spelling only differs in case. eXo's files are
//! not rewritten; the link sits beside the real file.

use std::path::{Path, PathBuf};

use super::dosbox::resolve_rel_ignoring_case;

/// The cue sheets a conf mounts: `.cue` targets of `IMGMOUNT` lines, as
/// written (quoted or bare, `.\` or absolute).
fn mounted_cues(conf: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in conf.lines() {
        let t = line.trim_start().trim_start_matches('@');
        if !t.get(..8).is_some_and(|w| w.eq_ignore_ascii_case("imgmount")) {
            continue;
        }
        let rest = &t[8..];
        // Quoted targets may hold spaces; bare ones are single tokens.
        let mut tokens = Vec::new();
        let mut chars = rest.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '"' {
                let end = rest[i + 1..].find('"').map(|e| i + 1 + e).unwrap_or(rest.len());
                tokens.push(rest[i + 1..end].to_string());
                while chars.peek().is_some_and(|(j, _)| *j <= end) {
                    chars.next();
                }
            } else if !c.is_whitespace() {
                let end = rest[i..].find(char::is_whitespace).map(|e| i + e).unwrap_or(rest.len());
                tokens.push(rest[i..end].to_string());
                while chars.peek().is_some_and(|(j, _)| *j < end) {
                    chars.next();
                }
            }
        }
        out.extend(tokens.into_iter().filter(|t| t.to_ascii_lowercase().ends_with(".cue")));
    }
    out
}

/// Where a mounted cue is on disk: absolute as is, `.\`-relative under
/// `base`, either matched ignoring case.
fn locate(target: &str, base: &Path) -> Option<PathBuf> {
    let fwd = target.replace('\\', "/");
    let path = Path::new(&fwd);
    if path.is_absolute() {
        if path.is_file() {
            return Some(path.to_path_buf());
        }
        let rel = fwd.trim_start_matches('/');
        return resolve_rel_ignoring_case(Path::new("/"), rel).filter(|p| p.is_file());
    }
    resolve_rel_ignoring_case(base, fwd.trim_start_matches("./")).filter(|p| p.is_file())
}

/// The track files a cue names (`FILE "name" BINARY`), as written.
fn track_files(cue: &str) -> Vec<String> {
    cue.lines()
        .filter_map(|l| {
            let t = l.trim_start();
            if !t.get(..5).is_some_and(|w| w.eq_ignore_ascii_case("file ")) {
                return None;
            }
            let rest = t[5..].trim_start();
            let name = match rest.strip_prefix('"') {
                Some(q) => q.split('"').next()?,
                None => rest.split_whitespace().next()?,
            };
            Some(name.to_string())
        })
        .collect()
}

/// Link every track a mounted cue spells in another case. Returns the links
/// made, for the log.
pub(crate) fn alias_cue_tracks(conf: &str, base: &Path) -> Vec<PathBuf> {
    let mut made = Vec::new();
    for target in mounted_cues(conf) {
        let Some(cue_path) = locate(&target, base) else { continue };
        let Some(dir) = cue_path.parent() else { continue };
        let Ok(bytes) = std::fs::read(&cue_path) else { continue };
        for name in track_files(&String::from_utf8_lossy(&bytes)) {
            let rel = name.replace('\\', "/");
            if dir.join(&rel).exists() {
                continue;
            }
            let Some(real) = resolve_rel_ignoring_case(dir, &rel) else { continue };
            let link = dir.join(&rel);
            #[cfg(unix)]
            {
                let target = real.strip_prefix(link.parent().unwrap_or(dir)).map(Path::to_path_buf).unwrap_or(real.clone());
                match std::os::unix::fs::symlink(&target, &link) {
                    Ok(()) => made.push(link),
                    Err(e) => log::warn!("cue: could not link {} -> {}: {}", link.display(), real.display(), e),
                }
            }
            #[cfg(not(unix))]
            let _ = (real, link);
        }
    }
    if !made.is_empty() {
        log::info!("cue: linked {} track file(s) spelled in another case: {:?}", made.len(), made);
    }
    made
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_mounted_cues_in_any_spelling() {
        let conf = "[autoexec]\nIMGMOUNT c .\\emulators\\dosbox\\x98\\W98-C.vhd\nIMGMOUNT e \".\\eXoWin9x\\1996\\SimCopter (1996)\\out-simcopter.cue\" -t cdrom -ide 2m\n@imgmount d /games/x/cd/GAME.CUE -t iso\nmount c .\nREM imgmount z a.cue\n";
        assert_eq!(mounted_cues(conf), [".\\eXoWin9x\\1996\\SimCopter (1996)\\out-simcopter.cue", "/games/x/cd/GAME.CUE"]);
    }

    #[test]
    fn reads_track_names() {
        let cue = "FILE \"OUT-SIMCOPTER.BIN\" BINARY\n  TRACK 01 MODE1/2352\nfile Track02.wav WAVE\n";
        assert_eq!(track_files(cue), ["OUT-SIMCOPTER.BIN", "Track02.wav"]);
    }

    #[cfg(unix)]
    #[test]
    fn links_a_track_spelled_in_another_case() {
        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("eXoWin9x/1996/SimCopter (1996)");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("out-simcopter.cue"), "FILE \"OUT-SIMCOPTER.BIN\" BINARY\n  TRACK 01 MODE1/2352\n").unwrap();
        std::fs::write(game.join("out-simcopter.bin"), b"data").unwrap();
        let conf = "IMGMOUNT e \".\\eXoWin9x\\1996\\SimCopter (1996)\\out-simcopter.cue\" -t cdrom";
        let made = alias_cue_tracks(conf, root.path());
        assert_eq!(made, [game.join("OUT-SIMCOPTER.BIN")]);
        assert_eq!(std::fs::read(game.join("OUT-SIMCOPTER.BIN")).unwrap(), b"data");
        // Idempotent: the second launch finds the link and adds nothing.
        assert!(alias_cue_tracks(conf, root.path()).is_empty());
        // An absolute mount (the DOS launcher's patched conf) works too.
        let abs = format!("imgmount d \"{}\" -t iso", game.join("out-simcopter.cue").display());
        std::fs::remove_file(game.join("OUT-SIMCOPTER.BIN")).unwrap();
        assert_eq!(alias_cue_tracks(&abs, Path::new("/nonexistent")).len(), 1);
    }
}
