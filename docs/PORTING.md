# eXorchy native port - conventions for the GTK app (`crates/app`)

Read this before writing UI code. The backend (`crates/core`) is finished and
tested; the UI is being rebuilt in Rust with GTK4 + libadwaita from the web UI
from the previous SolidJS UI. That UI was deleted once every module reached parity;
it is still readable in git history at the baseline commit (`git show main:src/...`).

## Layout

```
crates/app/src/
  main.rs        adw::Application, bootstrap, single instance
  app.rs         the tokio ↔ GTK bridge (see below)
  theme.rs       Omarchy palette → CSS custom properties (tokens), live
  style.css      base rules + styles/<module>.css (one sheet per feature module)
  snapshot.rs    EXORCHY_SNAPSHOT=<png>[:<ms>] renders the window to a PNG and quits
  ui/window.rs   the window: splash, setup ↔ library, toasts, startup order, restart_to_setup(), reinit_library(), show_page()/close_page()
  ui/library.rs  Browse / My Library / Reading tabs, grid, filters, detail panel host; slots: banner_slot, set_reading_widget(), the document page (show_document)
  ui/detail.rs   the detail panel; hooks: media_slot, on_shown(cb), current()
  ui/card.rs     a game card; ui/model.rs GameObject; ui/covers.rs cover loading
  ui/downloads.rs the download trackers (1 Hz poll); ui/actions.rs shared game actions
  ui/bus.rs      library/collections/playlists/running change signals, offline flag, toast()/toast_with()
  ui/dialogs.rs  confirm(), error(), pick_folder()
  ui/util.rs     format_bytes(), esc(), collection_label(), platform_tag(), parse_lang_entries()
  ui/settings.rs, reading.rs (+ pdf.rs), media.rs, playlists.rs, game_settings.rs,
  onboarding.rs, launch_notes.rs   the feature modules being ported (see docs/HANDOVER.md)
```

Everything else in the tree: `crates/core` is the backend (`exorchy_core`,
`host.rs` is the shim the commands run on), the baseline commit on `main` holds the old web UI,
`packaging/` the PKGBUILD and desktop file (`org.exorchy.eXorchy.desktop`,
named after the GApplication id `APP_ID` in `main.rs`).

## Calling the backend

Every backend operation is an `async fn` in `exorchy_core::commands::<module>`
taking `State<'_, T>` arguments (get them with `core.state()`, type inferred)
and sometimes an `AppHandle` (`core.clone()`). They must run on tokio, and
widgets must be touched only on the GTK thread:

```rust
use crate::app;
let core = app::core();
app::spawn(
    async move { games::get_config(core.state(), "network_mode".into()).await },
    move |res: Result<Option<String>, String>| { /* GTK thread: update widgets */ },
);
// or, inside app::local(async move { ... }):  let r = app::call(async move { ... }).await;
app::on_event("game-exited", |payload: &serde_json::Value| { ... });   // backend events
```

Never block the main thread; never hold a `RefCell` borrow across a callback.
Weak references into closures: `glib::clone!(#[weak] widget, move |_| ...)`.

## Theming

No colour literals anywhere. Use the tokens `theme.rs` defines on `:root`:
`--bg-primary/-secondary/-card/-hover`, `--text-primary/-secondary/-muted/-strong`,
`--accent`, `--accent-hover`, `--accent-fill`, `--accent-glow`, `--on-accent`,
`--danger`, `--success`, `--warning`, `--info`, `--line-1..4`, `--fill-1..3`,
`--scrim`, `--shadow`, `--radius`, `--radius-lg`, plus every `--om-*` palette key
and libadwaita's own variables. CSS classes in use: `btn` (+ `primary`,
`danger`, `ghost`, `icon`), `field`, `search`, `tab`, `panel`, `title-1/2/3`,
`muted`, `secondary`, `small`, `badge`, `chip`, `menu-item`, `context-menu`.
Put new rules in your module's sheet under `crates/app/src/styles/`.
`crate::theme::current()` gives the active `Theme` (palette, name, font).

## Dialogs

Use libadwaita: `adw::Dialog` / `adw::AlertDialog`, presented with
`.present(Some(&parent_widget))`, for short, centred things (confirmations,
pickers). No second toplevel windows (Hyprland would tile them). A large
screen of its own (Settings) is a full-body page instead: `window::show_page`
puts it in place of the library and `window::close_page` goes back.

## Running and checking

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build -p exorchy && cargo clippy -p exorchy -- -D warnings && cargo test -p exorchy
S=/tmp/claude-1000/-home-renan-Projects-exorchy/01967169-baec-49c6-959b-94d8e247a281/scratchpad   # isolated profile (offline copy of the catalogue)
XDG_DATA_HOME=$S/xdg/data XDG_STATE_HOME=$S/xdg/state XDG_CONFIG_HOME=$S/xdg/config XDG_CACHE_HOME=$S/xdg/cache \
  RUST_LOG=info EXORCHY_SNAPSHOT=$S/out.png:6000 timeout 40 ./target/debug/exorchy
# EXORCHY_SNAPSHOT_GAME=<id> opens that game's detail panel before the snapshot (9676 = SimCity 2000, installed).
```
The scratch profile is offline (no torrent session), so downloads are not
offered there; the real profile is `~/.local/share/exorchy` (do not modify it).
Look at the PNG (Read tool) to verify layouts. Do not use grim (it hangs on this compositor).
Snapshots need painted frames: with the monitors off (DPMS) a Wayland window never gets one, so
render headlessly instead - `gtk4-broadwayd :7 &` then `GDK_BACKEND=broadway BROADWAY_DISPLAY=:7`
in front of the command above (`GDK_BACKEND=x11` via XWayland works too). The app is single-instance:
a running instance makes later runs exit at once as a remote. Never `pkill` it: it may be the user's
own eXorchy. Isolate the check instead with `dbus-run-session -- ...`, and inside that session also
`unset WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE DISPLAY XDG_CURRENT_DESKTOP` and set
`GDK_DEBUG=no-portals GTK_USE_PORTAL=0`: otherwise GTK starts a private
`xdg-desktop-portal-hyprland` against the user's Hyprland, which segfaults when the session ends
and pops Omarchy's crash notice on their desktop (2026-09-30, once per run).
More switches: `EXORCHY_SNAPSHOT_SEQUENCE=click:<button text>,activate:<list row text>,wait,…` (one step per 250 ms; e.g. `click:Tags,wait,activate:Theme: Fantasy`; `fullscreen` puts the open document in full-screen reading mode, `doc:<path>` opens a document as a dossier's manual would, `close_doc` closes it, `type:<text>` fills the focused (or topmost) entry and presses Enter, `cover` opens the dossier's cover at full size; `click:` finds buttons in dialogs too; a long sequence delays the shot to 1.1 s after its last step, and the capture waits for a fresh paint; snapshots run with GTK animations off, since Broadway never finishes a stack transition), `EXORCHY_SNAPSHOT_SCROLL=<css class>:<px>` (scrolls inside e.g. `detail-panel`; Broadway is capped at 1024x768), `EXORCHY_SNAPSHOT_TAB=library|reading`, `EXORCHY_SNAPSHOT_SETTINGS=<section>`,
`EXORCHY_SNAPSHOT_READING=1` (+ `_VIEW=list`, `EXORCHY_SNAPSHOT_ISSUE=<key>`, `EXORCHY_SNAPSHOT_DOC=<file>`).
`EXORCHY_DUMP_TREE=1` prints the widget tree next to the snapshot.
`EXORCHY_SNAPSHOT_SPLASH=<png>` (with `EXORCHY_SNAPSHOT` set) captures the splash window instead of the app.
Media: `EXORCHY_MEDIA_FAKE_SINKS=1` sends all sound to a fakesink (silent test runs);
`EXORCHY_MEDIA_TEST_FILES=a.mp4:b.mp3 cargo test -p exorchy playbin -- --ignored` (under a
Broadway display, with the fake sinks) switches 60 times between real cached files, the
pattern that aborted GTK's playbin3 backend.

## Rules

- Own only the files assigned to you; other modules are being written in parallel.
- Every feature the web UI has must come across; port the logic, not the DOM.
- Comments state the contract, not the history. Decisions go to `docs/DECISIONS.md` (append; one entry each) - only if you made one.
- The app is called eXorchy (capital X) in every user-facing string.
- Keep `cargo clippy -p exorchy -- -D warnings` clean for your files and add unit tests for pure logic.
