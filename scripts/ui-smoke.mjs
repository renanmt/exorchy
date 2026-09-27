#!/usr/bin/env node
/**
 * Headless UI smoke test: renders the frontend from the Vite dev server in
 * headless Chromium with a mocked Tauri IPC (real catalogue rows out of
 * metadata/exorchy.db, the real Omarchy palette) and writes screenshots.
 * Needs: `pnpm dev` (or `pnpm tauri dev`) running on :1420, chromium, sqlite3.
 *
 *   node scripts/ui-smoke.mjs [out-dir]        -> out-dir/{library,detail,settings-*}.png
 *
 * This is not an end-to-end test of the Rust side; it proves the UI mounts,
 * lays out and themes with realistic data, which is what a screenshot can show.
 */
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { resolve, join } from "node:path";
import { homedir } from "node:os";

const ROOT = resolve(new URL("..", import.meta.url).pathname);
const OUT = resolve(process.argv[2] ?? join(ROOT, "work", "ui-smoke"));
const PORT = 9222;
const URL_ = "http://localhost:1420/";
mkdirSync(OUT, { recursive: true });

// ── Data: real rows from the bundled DB ─────────────────────────────────────
const DB = join(ROOT, "metadata", "exorchy.db");
if (!existsSync(DB)) throw new Error(`missing ${DB} - run pnpm gen-db`);
const sql = (q) => JSON.parse(execFileSync("sqlite3", ["-json", DB, q]).toString() || "[]");
const GAME_COLS = `id,title,sort_title,platform,developer,publisher,release_date,year,genre,series,play_mode,rating,rating_votes,description,notes,source,application_path,dosbox_conf,status,region,max_players,language,shortcode,torrent_source,in_library,installed,favorited,game_torrent_index,gamedata_torrent_index,download_size,has_thumbnail,dosbox_variant,thumbnail_key,manual_path,last_played,music_file`;
const fix = (g) => ({ ...g, in_library: !!g.in_library, installed: !!g.installed, favorited: !!g.favorited, has_thumbnail: !!g.has_thumbnail, available_languages: null, variant_titles: null });
const games = sql(`SELECT ${GAME_COLS} FROM games WHERE torrent_source='eXoDOS' AND description IS NOT NULL ORDER BY rating_votes DESC, title LIMIT 60`).map(fix);
// Pretend the first few are installed / in library so shelves and badges render.
games[0].installed = true; games[0].in_library = true; games[0].last_played = "2026-09-26T10:00:00Z";
games[1].in_library = true;
games[2].favorited = true;
const genres = sql(`SELECT DISTINCT genre FROM games WHERE genre IS NOT NULL LIMIT 200`).flatMap((r) => r.genre.split(";").map((s) => s.trim())).filter((v, i, a) => a.indexOf(v) === i).sort().slice(0, 40);
const playlists = sql(`SELECT id,name,kind,description,(SELECT COUNT(*) FROM playlist_games pg WHERE pg.playlist_id=p.id) AS game_count FROM playlists p`);
const PREVIEWS = join(ROOT, "src-tauri", "resources", "previews", "eXoDOS");

// ── Palette: the real Omarchy theme, or the fallback ────────────────────────
function readTheme() {
  const cur = join(process.env.XDG_STATE_HOME || join(homedir(), ".local/state"), "omarchy", "current");
  const palette = {};
  try {
    for (const line of readFileSync(join(cur, "theme", "colors.toml"), "utf8").split("\n")) {
      const m = line.match(/^\s*([a-z_]+)\s*=\s*"([^"]*)"/);
      if (m) palette[m[1]] = m[2];
    }
    const name = readFileSync(join(cur, "theme.name"), "utf8").trim();
    return { name, source: "omarchy", palette, fontBaseSize: 12, monoFont: "JetBrainsMono Nerd Font" };
  } catch {
    return null;
  }
}
const theme = readTheme();

const responses = {
  get_setup_status: { phase: "ready", metadata_progress: null, dosbox_metadata_progress: null, games_imported: games.length, ready: true },
  get_config: (a) => ({ data_dir: "/home/user/Games", root_folder: "eXoDOS", network_mode: "live", seeding_enabled: "1", collections: "eXoDOS", welcome_seen: "1", view_mode: "grid", global_glshader: "crt-auto", default_fullscreen: "window", keep_archives: "1", dismissed_notes: "" }[a.key] ?? null),
  set_config: null,
  init_download_manager: true,
  get_available_collections: [
    { id: "eXoDOS_GLP", display_name: "German Language Pack", torrent_file: "eXoDOS_GLP.torrent", game_count: 651 },
    { id: "eXoDOS_PLP", display_name: "Polish Language Pack", torrent_file: "eXoDOS_PLP.torrent", game_count: 238 },
    { id: "eXoDOS_SLP", display_name: "Spanish Language Pack", torrent_file: "eXoDOS_SLP.torrent", game_count: 642 },
    { id: "eXoDOS", display_name: "eXoDOS", torrent_file: "eXoDOS.torrent", game_count: 7666 },
    { id: "eXoWin3x", display_name: "eXoWin3x", torrent_file: "eXoWin3x.torrent", game_count: 1138 },
    { id: "eXoWin9x", display_name: "eXoWin9x", torrent_file: "eXoWin9x.torrent", game_count: 664 },
    { id: "eXoScummVM", display_name: "eXoScummVM", torrent_file: "eXoScummVM.torrent", game_count: 832 },
  ],
  get_preview_dir: PREVIEWS,
  get_poster_dir: "",
  get_games: (a) => {
    let rows = games;
    if (a.query) rows = rows.filter((g) => g.title.toLowerCase().includes(a.query.toLowerCase()));
    if (a.favoritesOnly) rows = rows.filter((g) => g.favorited);
    const per = a.perPage ?? 50, page = a.page ?? 1;
    return { games: rows.slice((page - 1) * per, page * per), total: rows.length };
  },
  get_genres: genres,
  get_section_keys: (a) => (a.sortBy === "year_desc" || a.sortBy === "year_asc") ? [...new Set(games.map((g) => String(g.year)))] : [...new Set(games.map((g) => g.title[0].toUpperCase()))].sort(),
  get_game: (a) => games.find((g) => g.id === a.id) ?? null,
  get_game_variants: (a) => games.filter((g) => g.shortcode === a.shortcode),
  get_installed_games: games.filter((g) => g.installed),
  get_recently_played: games.filter((g) => g.installed),
  get_playlists: playlists,
  get_game_playlists: [],
  get_transfer_stats: { download_bps: 0, upload_bps: 0, uploaded_bytes: 0, peers: 3, active: true },
  list_active_downloads: [],
  scan_installed_games: 1,
  list_content_packs: (a) => a.collection === "eXoDOS" ? [
    { id: "posters", display_name: "Box Art", description: "HD box art", size_bytes: 396948419, version: 5, supersedes: [], available: true, installed: false },
    { id: "dosbox-staging", display_name: "DOSBox Staging 0.83.0", description: "emulator", size_bytes: 29377664, version: 830, supersedes: [], available: true, installed: true, installed_version: 830 },
  ] : [],
  get_dosbox_status: { source: "pack", path: "/home/user/Games/content/emulators/dosbox-staging/dosbox", packInstalled: true, systemAvailable: false, useSystem: false },
  ensure_dosbox_staging: { source: "pack", path: "/home/user/Games/content/emulators/dosbox-staging/dosbox", packInstalled: true, systemAvailable: false, useSystem: false },
  get_dos_support_status: { phase: "downloading", progress: 0.42, total_bytes: 630000000 },
  running_game_ids: [],
  game_printing_unavailable: false,
  game_disk_usage: { game_bytes: 15_400_000, archive_bytes: 4_100_000, save_bytes: 12_000 },
  get_game_metadata: { manual_path: null, manual_kind: null, images: [], thumbnails: [] },
  get_game_settings: { engine: null, glshader: null, fullscreen: null, cycles: null, custom_conf: null },
  get_theme: theme ?? { name: null, source: "fallback", palette: { mode: "dark" }, fontBaseSize: 12, monoFont: null },
  get_system_font: theme?.monoFont ?? null,
  get_log_dir: "/home/user/.local/state/exorchy/logs",
  // Full-feature commands the re-baselined UI asks for at startup.
  video_playback_supported: false,
  video_mirror_needed: false,
  music_playback_supported: { mp3: false, ogg: false },
  music_cache_index: { cached: [], none: [] },
  music_shuffle_candidates: [],
  list_publications: [],
  list_issues: [],
  reading_cache_index: [],
  game_articles: [],
  win9x_network_status: { enabled: false, can_enable: false, detail: "No Windows 9x games enabled.", manual_hint: null },
  game_engine_info: { ece_available: false, uses_ece: false },
  storage_overview: { folder: "/home/user/Games", free_bytes: 400e9, total_bytes: 1e12, used_bytes: 21e9, other_bytes: 579e9, categories: [{ id: "games", bytes: 12e9, items: 30 }, { id: "archives", bytes: 8e9, items: 30 }, { id: "packs", bytes: 1e9, items: 2 }] },
  installed_games_storage: [],
  archive_usage: { count: 30, bytes: 8e9 },
};

const MOCK = `
(() => {
  const R = ${JSON.stringify(Object.fromEntries(Object.entries(responses).map(([k, v]) => [k, typeof v === "function" ? null : v])))};
  const F = { ${Object.entries(responses).filter(([, v]) => typeof v === "function").map(([k, v]) => `${k}: ${v.toString()}`).join(",\n")} };
  const games = ${JSON.stringify(games)}; const genres = ${JSON.stringify(genres)}; const playlists = ${JSON.stringify(playlists)}; const PREVIEWS = ${JSON.stringify(PREVIEWS)}; const theme = ${JSON.stringify(theme)};
  let cb = 0;
  window.__EXORCHY_CALLS__ = [];
  window.__TAURI_INTERNALS__ = {
    transformCallback(fn) { const id = ++cb; window["_" + id] = fn; return id; },
    convertFileSrc(p) { return "http://localhost:1420/@fs" + p; },
    async invoke(cmd, args) {
      window.__EXORCHY_CALLS__.push(cmd);
      if (cmd.startsWith("plugin:event|")) return 1;
      if (cmd in F) return F[cmd](args ?? {});
      if (cmd in R) return R[cmd];
      console.warn("unmocked command", cmd);
      return null;
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
  };
  window.__TAURI__ = {};
})();
`;

// ── CDP driver (Node 22 has WebSocket) ──────────────────────────────────────
const chrome = spawn("chromium", ["--headless=new", "--no-sandbox", "--disable-gpu", "--hide-scrollbars", `--remote-debugging-port=${PORT}`, "--window-size=1400,900", "about:blank"], { stdio: "ignore" });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function json(path) { for (let i = 0; i < 40; i++) { try { return await (await fetch(`http://127.0.0.1:${PORT}${path}`)).json(); } catch { await sleep(250); } } throw new Error("chrome did not start"); }
const target = await json("/json/list").then((l) => l.find((t) => t.type === "page")) ?? await json("/json/new?about:blank");
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0; const pending = new Map();
ws.onmessage = (m) => { const d = JSON.parse(m.data); if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); } };
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const evaluate = async (expression) => (await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true })).result?.result?.value;
async function shot(name) { const r = await send("Page.captureScreenshot", { format: "png" }); writeFileSync(join(OUT, `${name}.png`), Buffer.from(r.result.data, "base64")); console.log("wrote", name); }
const click = async (sel) => evaluate(`(() => { const el = document.querySelector(${JSON.stringify(sel)}); if (!el) return "missing " + ${JSON.stringify(sel)}; el.click(); return "ok"; })()`);
const clickText = async (sel, text) => evaluate(`(() => { const el = [...document.querySelectorAll(${JSON.stringify(sel)})].find(e => e.textContent.trim().startsWith(${JSON.stringify(text)})); if (!el) return "missing " + ${JSON.stringify(text)}; el.click(); return "ok"; })()`);

await send("Page.enable");
await send("Emulation.setDeviceMetricsOverride", { width: 1400, height: 900, deviceScaleFactor: 1, mobile: false });
await send("Page.addScriptToEvaluateOnNewDocument", { source: MOCK });
await send("Page.navigate", { url: URL_ });
await sleep(2200);
await shot("splash");
await sleep(2500);
console.log("phase:", await evaluate(`document.querySelector(".top-bar") ? "ready" : (document.querySelector(".setup") ? "setup" : document.body.className)`));
// The library opens on My Library when something was played; Browse is the grid.
console.log(await clickText(".lib-tabs button, .lib-tab", "Browse"));
await sleep(900);
await shot("library");
console.log(await click(".game-card .game-card-art"));
await sleep(900);
await shot("detail");
await evaluate(`window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))`);
await sleep(400);
console.log(await clickText(".lib-tabs button, .lib-tab", "My Library"));
await sleep(700);
await shot("my-library");
console.log(await clickText(".lib-tabs button, .lib-tab", "Browse"));
await sleep(300);
console.log(await click('[data-testid="open-settings"]'));
await sleep(700);
await shot("settings-general");
for (const [label, name] of [["Collections", "settings-collections"], ["Emulators", "settings-emulators"], ["Appearance", "settings-appearance"], ["Storage", "settings-storage"], ["Network", "settings-network"], ["Content Packs", "settings-packs"], ["About", "settings-about"]]) {
  console.log(label, await clickText(".settings-nav button, .settings-nav-item", label));
  await sleep(600);
  await shot(name);
}
const unmocked = await evaluate(`[...new Set(window.__EXORCHY_CALLS__)].join(" ")`);
console.log("commands called:", unmocked);
ws.close();
chrome.kill();
