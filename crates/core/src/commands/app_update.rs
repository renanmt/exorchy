//! Updates of eXorchy itself (not the content packs: that is `updates`).
//!
//! `check_app_update` asks GitHub for the latest release and compares its tag
//! with this build's version. `launch_app_update` opens a terminal (Omarchy's
//! floating one when present) that runs that release's `install.sh` for
//! exactly that version (download, SHA-256 check, `sudo pacman -U`), waits for
//! this process to be gone and starts eXorchy again; the caller quits right
//! after. Installing needs the user's password, and Omarchy has no graphical
//! polkit agent, so a terminal is where pacman can ask. Only a pacman-installed
//! copy is updated this way (`update_supported`): a `~/.local` or source-tree
//! copy would get a second install beside it.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The GitHub repository the releases come from.
pub const RELEASE_REPO: &str = "renanmt/exorchy";

/// This build's version, or `EXORCHY_PRETEND_VERSION` (a developer aid: the
/// update banner and flow can be seen against a real release by pretending
/// to be older).
pub fn current_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| {
            std::env::var("EXORCHY_PRETEND_VERSION")
                .ok()
                .filter(|v| parse_version(v).is_some())
                .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
        })
        .as_str()
}

/// A release newer than this build.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppUpdate {
    /// "0.4.0" (the tag without its "v").
    pub version: String,
    /// "v0.4.0".
    pub tag: String,
    /// The release page, for "What's new".
    pub url: String,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// `"v1.2.3"` / `"1.2.3"` → (1, 2, 3); anything else → None.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let version = (parts.next()??, parts.next().unwrap_or(Some(0))?, parts.next().unwrap_or(Some(0))?);
    parts.next().is_none().then_some(version)
}

/// Whether `latest` is newer than `current` (unparseable = not newer).
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

/// Ask GitHub for the latest release; `Some` when it is newer than this build.
pub async fn check_app_update() -> Result<Option<AppUpdate>, String> {
    let client = reqwest::Client::builder()
        // GitHub's API refuses requests without a User-Agent.
        .user_agent(format!("eXorchy/{}", current_version()))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("https://api.github.com/repos/{RELEASE_REPO}/releases/latest");
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("could not reach GitHub: {e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None); // no release yet
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub answered {}", resp.status()));
    }
    let release: GithubRelease = resp.json().await.map_err(|e| format!("unexpected answer from GitHub: {e}"))?;
    if release.draft || release.prerelease || !is_newer(&release.tag_name, current_version()) {
        return Ok(None);
    }
    Ok(Some(AppUpdate {
        version: release.tag_name.trim_start_matches('v').to_string(),
        tag: release.tag_name,
        url: release.html_url,
    }))
}

/// Whether this copy can update itself: it is the pacman package's binary.
pub async fn update_supported() -> bool {
    tokio::task::spawn_blocking(|| {
        let exe = std::env::current_exe().ok().and_then(|p| std::fs::canonicalize(p).ok());
        let packaged_path = exe.as_deref() == Some(Path::new("/usr/bin/exorchy"));
        packaged_path
            && Command::new("pacman")
                .args(["-Q", "exorchy"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

/// The shell command the terminal runs: install `tag`, wait for this
/// eXorchy to be gone, start the (new, or on failure the old) one again.
fn update_command(tag: &str) -> String {
    format!(
        "echo 'Updating eXorchy to {tag}...'; echo; \
         curl -fsSL https://github.com/{RELEASE_REPO}/releases/download/{tag}/install.sh | bash -s -- --version {tag}; \
         status=$?; \
         for _ in $(seq 60); do pgrep -x exorchy >/dev/null || break; sleep 0.5; done; \
         if command -v uwsm-app >/dev/null; then setsid -f uwsm-app -- exorchy >/dev/null 2>&1; \
         else setsid -f exorchy >/dev/null 2>&1; fi; \
         echo; if [ $status -eq 0 ]; then echo 'eXorchy is starting again.'; \
         else echo 'The update did not finish; eXorchy is starting again as it was.'; fi; \
         exit $status"
    )
}

/// Open the update terminal for `tag` ("v0.4.0"). The caller quits eXorchy
/// once this returns Ok.
pub async fn launch_app_update(tag: String) -> Result<(), String> {
    // The tag goes into a shell command: digits and dots only, no suffixes.
    let bare = tag.strip_prefix('v').unwrap_or(&tag);
    if parse_version(bare).is_none() || !bare.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Err(format!("not a release tag: {tag}"));
    }
    let tag = if tag.starts_with('v') { tag } else { format!("v{tag}") };
    let cmd = update_command(&tag);
    tokio::task::spawn_blocking(move || {
        let spawned = if which("omarchy-launch-floating-terminal-with-presentation") {
            Command::new("omarchy-launch-floating-terminal-with-presentation").arg(&cmd).spawn()
        } else if which("xdg-terminal-exec") {
            // No Omarchy wrapper: keep the window open so the result can be read.
            Command::new("xdg-terminal-exec")
                .args(["-e", "bash", "-c", &format!("{cmd}; echo; read -rp 'Press Enter to close this window.'")])
                .spawn()
        } else {
            return Err("no terminal launcher found (omarchy-launch-floating-terminal-with-presentation or xdg-terminal-exec)".to_string());
        };
        spawned.map(|_| ()).map_err(|e| format!("could not open the update terminal: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn which(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("v0.4.0", "0.3.0"));
        assert!(is_newer("v0.10.0", "0.9.9"), "numeric, not text");
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("v0.3.0", "0.3.0"));
        assert!(!is_newer("v0.2.9", "0.3.0"));
        assert!(!is_newer("nightly", "0.3.0"));
        assert!(is_newer("v0.4", "0.3.9"), "a missing patch reads as 0");
        assert!(!is_newer("v0.4.0.1", "0.3.0"), "four parts is not a version");
        assert!(is_newer("v0.4.0-rc1", "0.3.0"));
    }

    #[tokio::test]
    async fn only_plain_version_tags_reach_the_shell() {
        for bad in ["v0.4.0+$(reboot)", "v0.4.0; rm -rf ~", "v0.4.0-rc1", "latest", ""] {
            assert!(launch_app_update(bad.to_string()).await.is_err(), "{bad}");
        }
    }

    /// Prints the terminal command for a tag, to run it by hand:
    /// `EXORCHY_TAG=v0.3.0 cargo test -p exorchy-core print_update_command -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_update_command() {
        let tag = std::env::var("EXORCHY_TAG").unwrap_or_else(|_| "v0.3.0".into());
        println!("CMD<<{}>>CMD", update_command(&tag));
    }

    #[test]
    fn the_update_command_installs_that_tag_and_restarts() {
        let cmd = update_command("v0.4.0");
        assert!(cmd.contains("/releases/download/v0.4.0/install.sh"));
        assert!(cmd.contains("--version v0.4.0"));
        assert!(cmd.contains("pgrep -x exorchy"), "waits for this process to be gone");
        assert!(cmd.contains("uwsm-app -- exorchy"));
    }
}
