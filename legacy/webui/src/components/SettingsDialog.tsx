import { createSignal, createEffect, on, Show, For, type Component } from "solid-js";
import {
  SlidersHorizontal, Layers, Cpu, Palette, HardDrive, Globe, Package, Info, FolderOpen, Gamepad2, Music, Download,
  Network, Activity, Heart, TriangleAlert, Type, type LucideProps,
} from "lucide-solid";
import { Portal } from "solid-js/web";
import { Dialog } from "@ark-ui/solid/dialog";
import { Button } from "./Button";
import { AutoProgress } from "./ProgressBar";
import { ContentPackSettings } from "./ContentPackSettings";
import { StorageTab } from "./StorageTab";
import { SettingRow, SettingSwitch, SectionTitle } from "./SettingRow";
import {
  getConfig, setConfig, setRateLimits, scanInstalledGames, openLogFolder,
  getDosboxStatus, ensureDosboxStaging, getDosSupportStatus,
  win9xNetworkStatus, enableWin9xNetwork, disableWin9xNetwork,
  type DosboxStatus, type SupportStatus, type Win9xNetworkStatus,
} from "../api/tauri";
import { applyNetworkMode, isOffline, loadNetworkMode } from "../stores/network";
import { seedingOn, applySeeding, loadSeeding } from "../stores/seeding";
import { fetchGames } from "../stores/games";
import { showToast } from "../stores/toasts";
import { activeJobs, installedPacks } from "../stores/contentPacks";
import { theme } from "../stores/theme";
import {
  availableCollections, enabledCollections, collectionsBusy, loadCollections,
  isCollectionEnabled, setCollectionEnabled, BASE_COLLECTION, COLLECTION_BLURBS,
} from "../stores/collections";
import {
  musicAutoplay, setMusicAutoplay, ensureMusicAutoplayLoaded,
  musicContinuous, setMusicContinuous, ensureMusicContinuousLoaded,
} from "../stores/music";
import { formatBytes } from "../util";

export type SettingsSection = "general" | "collections" | "emulators" | "appearance" | "storage" | "network" | "packs" | "about";

const APP_VERSION = "0.1.0";

/** The emulator pack's job key in the content-pack store (collection:pack). */
const DOSBOX_JOB_KEY = "eXoDOS:dosbox-staging";

const SECTIONS: { id: SettingsSection; label: string; icon: Component<LucideProps> }[] = [
  { id: "general", label: "General", icon: SlidersHorizontal },
  { id: "collections", label: "Collections", icon: Layers },
  { id: "emulators", label: "Emulators", icon: Cpu },
  { id: "appearance", label: "Appearance", icon: Palette },
  { id: "storage", label: "Storage", icon: HardDrive },
  { id: "network", label: "Network", icon: Globe },
  { id: "packs", label: "Content Packs", icon: Package },
  { id: "about", label: "About", icon: Info },
];

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  section: SettingsSection;
  onSectionChange: (section: SettingsSection) => void;
  gameFolderPath: string;
  onChangeDataDir: () => void;
  onFactoryReset: () => void;
  resetError: string;
  /** Going online is where an old install owes its seeding answer. */
  onWentOnline: () => void;
}

/** Where the emulator comes from, in words. */
function dosboxSourceLabel(s: DosboxStatus): string {
  switch (s.source) {
    case "pack": return "Downloaded by Exorchy";
    case "system": return "System package";
    case "custom": return "Custom path";
    default: return "Not found";
  }
}

function supportLabel(s: SupportStatus | null): string {
  if (!s) { return "Checking…"; }
  switch (s.phase) {
    case "ready": return "Ready";
    case "downloading": return `Downloading… ${Math.round(s.progress * 100)}%`;
    case "failed": return "Download failed - it is retried on the next start";
    default: return s.total_bytes > 0
      ? `Not installed - fetched with the first game (${formatBytes(s.total_bytes)})`
      : "Not installed - fetched with the first game";
  }
}

export function SettingsDialog(props: Props) {
  // Game defaults mirror launch_game's own defaults until the load lands.
  const [crtAuto, setCrtAuto] = createSignal(true);
  const [defaultFullscreen, setDefaultFullscreen] = createSignal(false);
  const [scanning, setScanning] = createSignal(false);
  const [scanResult, setScanResult] = createSignal("");
  const [logOpenError, setLogOpenError] = createSignal("");
  const [netStatus, setNetStatus] = createSignal<Win9xNetworkStatus | null>(null);
  const [enablingNet, setEnablingNet] = createSignal(false);
  const [switchingMode, setSwitchingMode] = createSignal(false);
  const [modeError, setModeError] = createSignal("");
  // Strings: an empty field means unlimited, which no number can say.
  const [limitDown, setLimitDown] = createSignal("");
  const [limitUp, setLimitUp] = createSignal("");
  const [limitError, setLimitError] = createSignal("");
  // Emulators
  const [dosbox, setDosbox] = createSignal<DosboxStatus | null>(null);
  const [useSystemDosbox, setUseSystemDosbox] = createSignal(false);
  const [installingDosbox, setInstallingDosbox] = createSignal(false);
  const [dosboxError, setDosboxError] = createSignal("");
  const [support, setSupport] = createSignal<SupportStatus | null>(null);

  const refreshEmulators = () => {
    getDosboxStatus().then(setDosbox).catch((e) => setDosboxError(`Could not read the emulator status: ${e}`));
    getDosSupportStatus().then(setSupport).catch(() => setSupport(null));
  };

  const loadAll = async () => {
    setLogOpenError("");
    setModeError("");
    setScanResult("");
    setDosboxError("");
    ensureMusicAutoplayLoaded();
    ensureMusicContinuousLoaded();
    loadNetworkMode();
    loadSeeding();
    refreshEmulators();
    void loadCollections();
    win9xNetworkStatus().then(setNetStatus).catch(() => {});
    try {
      const [shader, fs, down, up, sysDosbox] = await Promise.all([
        getConfig("global_glshader"),
        getConfig("default_fullscreen"),
        getConfig("rate_limit_down_kbps"),
        getConfig("rate_limit_up_kbps"),
        getConfig("use_system_dosbox"),
      ]);
      setCrtAuto(shader == null || shader === "crt-auto");
      setDefaultFullscreen(fs === "fullscreen");
      setLimitDown(down ?? "");
      setLimitUp(up ?? "");
      setUseSystemDosbox(sysDosbox === "1");
    } catch (e) {
      console.warn("[settings] failed to load:", e);
    }
  };
  createEffect(on(() => props.open, (open) => { if (open) { void loadAll(); } }));

  const dosboxJob = () => activeJobs()[DOSBOX_JOB_KEY];
  const dosboxJobActive = () => !!dosboxJob() && !dosboxJob()!.finished;

  // The pack lands through the content-pack store (a finished job refreshes
  // `installedPacks`); the status row has to follow without a reopen. Also
  // covers the poll during a download: the row says "downloading" meanwhile.
  createEffect(on([installedPacks, dosboxJobActive], () => {
    if (props.open) { refreshEmulators(); }
  }, { defer: true }));

  const saveToggle = (key: string, on: string, off: string, set: (v: boolean) => void) => async (next: boolean) => {
    set(next);
    try {
      await setConfig(key, next ? on : off);
    } catch (e) {
      console.error(`[settings] failed to save ${key}:`, e);
      set(!next);
    }
  };

  const handleToggleSystemDosbox = async (next: boolean) => {
    await saveToggle("use_system_dosbox", "1", "0", setUseSystemDosbox)(next);
    refreshEmulators();
  };

  const handleInstallDosbox = async () => {
    setDosboxError("");
    setInstallingDosbox(true);
    try {
      // Idempotent: starts the pack job when nothing resolves; the job's
      // progress arrives through activeJobs.
      setDosbox(await ensureDosboxStaging());
    } catch (e) {
      setDosboxError(`Could not start the download: ${e}`);
    } finally {
      setInstallingDosbox(false);
    }
  };

  const dosboxStatusText = () => {
    const j = dosboxJob();
    if (j && !j.finished) {
      if (j.error) { return j.error; }
      const pct = Math.round((j.progress ?? 0) * 100);
      switch (j.phase) {
        case "starting": return "Starting download…";
        case "downloading": return `Downloading… ${pct}%`;
        case "verifying": return "Verifying checksum…";
        case "extracting": return "Extracting…";
        case "installing": return "Installing…";
        default: return j.phase;
      }
    }
    return "";
  };

  const handleRescan = async () => {
    setScanning(true);
    setScanResult("");
    try {
      const count = await scanInstalledGames(true);
      setScanResult(`${count} game${count !== 1 ? "s" : ""} marked as installed`);
      fetchGames();
    } catch (e) {
      setScanResult(`Error: ${e}`);
    } finally {
      setScanning(false);
    }
  };

  /** Rebuilds the torrent state: offline drops every manager, online creates
   *  a fresh session and re-adopts interrupted downloads. */
  const handleToggleOnline = async (online: boolean) => {
    setModeError("");
    setSwitchingMode(true);
    try {
      const stopped = await applyNetworkMode(online ? "live" : "offline");
      const notes: string[] = [];
      if (stopped.downloads > 0) {
        notes.push(`${stopped.downloads} game download${stopped.downloads === 1 ? "" : "s"} paused - resumes when you go back online`);
      }
      if (stopped.packs > 0) {
        notes.push(`${stopped.packs} content pack download${stopped.packs === 1 ? "" : "s"} cancelled`);
      }
      showToast(
        online ? "Online mode - downloads enabled" : "Offline mode - torrent client stopped",
        "info",
        notes.length > 0 ? { detail: `${notes.join("; ")}.` } : {},
      );
      if (online) { props.onWentOnline(); }
    } catch (e) {
      setModeError(`Could not switch mode: ${e}`);
    } finally {
      setSwitchingMode(false);
    }
  };

  const handleToggleSeeding = async (next: boolean) => {
    try {
      await applySeeding(next);
    } catch (e) {
      console.error("[settings] failed to save seeding preference:", e);
    }
  };

  const toggleWin9xNetwork = async (enable: boolean) => {
    setEnablingNet(true);
    try {
      setNetStatus(enable ? await enableWin9xNetwork() : await disableWin9xNetwork());
      showToast(enable ? "Windows 9x multiplayer enabled" : "Windows 9x multiplayer disabled", "success");
    } catch (e) {
      const msg = String(e);
      // "cancelled" is the user dismissing the OS dialog - not a failure.
      if (!msg.includes("cancelled")) {
        showToast(enable ? "Could not enable multiplayer" : "Could not disable multiplayer", "error", { detail: msg });
      }
    } finally {
      setEnablingNet(false);
    }
  };

  /** Saves on blur: applying mid-typing would throttle to "5" on the way to "500". */
  const handleSaveLimits = async () => {
    setLimitError("");
    // The command takes u32; anything larger fails deserialization with an
    // error about integers rather than about speed limits.
    const MAX_KBPS = 4_000_000;
    const parse = (raw: string): number | null => {
      const v = parseInt(raw, 10);
      if (!Number.isFinite(v) || v <= 0) { return null; }
      return Math.min(v, MAX_KBPS);
    };
    const up = parse(limitUp());
    const down = parse(limitDown());
    setLimitUp(up === null ? "" : String(up));
    setLimitDown(down === null ? "" : String(down));
    try {
      await setRateLimits(up, down);
    } catch (e) {
      setLimitError(`Could not apply the limits: ${e}`);
    }
  };

  const handleOpenLogFolder = async () => {
    setLogOpenError("");
    try {
      await openLogFolder();
    } catch (e) {
      setLogOpenError(`Could not open log folder: ${e}`);
    }
  };

  const themeName = () => {
    const t = theme();
    if (!t || t.source !== "omarchy" || !t.name) { return "Built-in fallback"; }
    return t.name;
  };

  const collectionHint = (id: string, count: number) => {
    const games = `${count.toLocaleString()} game${count === 1 ? "" : "s"}`;
    const blurb = COLLECTION_BLURBS[id];
    return blurb ? `${games} · ${blurb}` : games;
  };

  return (
    <Show when={props.open}>
      <Dialog.Root open={props.open} onOpenChange={(e) => props.onOpenChange(e.open)}>
        <Portal>
          <Dialog.Backdrop class="ark-dialog-backdrop" />
          <Dialog.Positioner class="ark-dialog-positioner">
            <Dialog.Content class="ark-dialog-content ark-dialog-settings" data-testid="settings-dialog">
              <Dialog.Title class="ark-dialog-title">Settings</Dialog.Title>
              <div class="settings-layout">
                <nav class="settings-nav" aria-label="Settings sections">
                  <For each={SECTIONS}>
                    {(s) => (
                      <button
                        class={`settings-nav-item${props.section === s.id ? " active" : ""}`}
                        data-testid={`settings-tab-${s.id}`}
                        onClick={() => props.onSectionChange(s.id)}
                      >
                        <s.icon size={15} strokeWidth={1.8} class="settings-nav-icon" aria-hidden="true" />
                        {s.label}
                      </button>
                    )}
                  </For>
                </nav>

                <div class="settings-tab-body">
                  <Show when={props.section === "general"}>
                    <div class="settings-body">
                      <section class="settings-section">
                        <SectionTitle icon={FolderOpen}>Library</SectionTitle>
                        <SettingRow
                          label="Game folder"
                          value={props.gameFolderPath || "Not set"}
                          hint="Points Exorchy at an existing folder - nothing is moved."
                        >
                          <Button variant="small" onClick={props.onChangeDataDir}>Change…</Button>
                        </SettingRow>
                        <SettingRow
                          label="Installed games"
                          hint={scanResult() || "Re-scan the disk for games that are already there."}
                        >
                          <Button variant="small" loading={scanning()} loadingLabel="Scanning…" onClick={() => void handleRescan()}>Scan</Button>
                        </SettingRow>
                      </section>

                      <section class="settings-section">
                        <SectionTitle icon={Gamepad2}>Game defaults</SectionTitle>
                        <p class="settings-section-hint">Applied on every launch, on top of eXoDOS's own configs.</p>
                        <SettingRow htmlFor="crt-auto" label="Auto CRT shaders" hint="A CRT shader matched to the game's video mode. DOSBox ECE (Windows) has none.">
                          <SettingSwitch id="crt-auto" checked={crtAuto()} label="Auto CRT shaders"
                            onChange={saveToggle("global_glshader", "crt-auto", "default", setCrtAuto)} />
                        </SettingRow>
                        <SettingRow htmlFor="fullscreen" label="Launch in fullscreen" hint="Alt+Enter still toggles at runtime.">
                          <SettingSwitch id="fullscreen" checked={defaultFullscreen()} label="Launch in fullscreen"
                            onChange={saveToggle("default_fullscreen", "fullscreen", "window", setDefaultFullscreen)} />
                        </SettingRow>
                      </section>

                      <section class="settings-section">
                        <SectionTitle icon={Music}>Music</SectionTitle>
                        <SettingRow htmlFor="music-autoplay" label="Play theme music" hint="Starts a game's theme when you open its details.">
                          <SettingSwitch id="music-autoplay" checked={musicAutoplay()} label="Play theme music" onChange={(v) => { void setMusicAutoplay(v); }} />
                        </SettingRow>
                        <SettingRow htmlFor="music-continuous" label="Continue with the next theme" hint="When a theme ends, the next one plays.">
                          <SettingSwitch id="music-continuous" checked={musicContinuous()} label="Continue with the next theme" onChange={(v) => { void setMusicContinuous(v); }} />
                        </SettingRow>
                      </section>
                    </div>
                  </Show>

                  <Show when={props.section === "collections"}>
                    <div class="settings-body">
                      <section class="settings-section">
                        <SectionTitle icon={Layers}>Collections</SectionTitle>
                        <p class="settings-section-hint">Only enabled collections appear in the library; their games and box art are downloaded only when enabled.</p>
                        <For each={availableCollections()} fallback={<span class="setting-hint">Loading collections…</span>}>
                          {(col) => {
                            const isBase = col.id === BASE_COLLECTION;
                            const inputId = `collection-${col.id}`;
                            return (
                              <SettingRow
                                htmlFor={inputId}
                                label={col.display_name}
                                hint={collectionHint(col.id, col.game_count)}
                              >
                                <SettingSwitch
                                  id={inputId}
                                  checked={isBase || isCollectionEnabled(col.id)}
                                  disabled={isBase || collectionsBusy()}
                                  label={col.display_name}
                                  onChange={(v) => void setCollectionEnabled(col.id, v)}
                                />
                              </SettingRow>
                            );
                          }}
                        </For>
                        <Show when={collectionsBusy()}>
                          <p class="settings-section-hint">Applying… the library reloads when it is done.</p>
                        </Show>
                        <Show when={!collectionsBusy() && enabledCollections().length > 1}>
                          <p class="settings-section-hint">{enabledCollections().length} collections enabled.</p>
                        </Show>
                      </section>
                    </div>
                  </Show>

                  <Show when={props.section === "emulators"}>
                    <div class="settings-body">
                      <section class="settings-section">
                        <SectionTitle icon={Cpu}>DOSBox Staging</SectionTitle>
                        <p class="settings-section-hint">Every DOS game runs in DOSBox Staging. Exorchy downloads its own build, or uses the one installed on the system.</p>
                        <SettingRow
                          label="Emulator"
                          value={dosbox() ? dosboxSourceLabel(dosbox()!) : "Checking…"}
                          hint={dosboxError() || dosboxStatusText() || dosbox()?.path || (dosbox() ? "No dosbox-staging found. Download the bundled build or install the system package." : "")}
                          stacked={dosboxJobActive()}
                        >
                          <Show when={dosboxJobActive()}>
                            <div class="pack-progress">
                              <AutoProgress value={dosboxJob()?.progress ?? 0} class="mini" indeterminate={dosboxJob()?.phase !== "downloading" || undefined} />
                            </div>
                          </Show>
                          <Show when={!dosboxJobActive() && dosbox() && !dosbox()!.packInstalled}>
                            <Button
                              variant="small"
                              loading={installingDosbox()}
                              loadingLabel="Starting…"
                              disabled={isOffline()}
                              title={isOffline() ? "Offline mode - nothing is downloaded. Enable downloads in Settings → Network." : undefined}
                              onClick={() => void handleInstallDosbox()}
                            >
                              Install DOSBox Staging (29 MB)
                            </Button>
                          </Show>
                        </SettingRow>
                        <SettingRow
                          htmlFor="use-system-dosbox"
                          label="Prefer system dosbox-staging"
                          hint={dosbox()?.systemAvailable
                            ? "A dosbox-staging on PATH is used instead of the downloaded build."
                            : "No dosbox-staging on PATH right now. AUR: dosbox-staging-bin"}
                        >
                          <SettingSwitch id="use-system-dosbox" checked={useSystemDosbox()} label="Prefer system dosbox-staging"
                            onChange={(v) => void handleToggleSystemDosbox(v)} />
                        </SettingRow>
                        <Show when={dosbox()?.systemAvailable}>
                          <p class="settings-section-hint">Tip: the system package is kept current by pacman. AUR: dosbox-staging-bin</p>
                        </Show>
                      </section>

                      <section class="settings-section">
                        <SectionTitle icon={Activity}>MIDI music</SectionTitle>
                        <SettingRow
                          label="MT-32 and SoundCanvas"
                          value={supportLabel(support())}
                          hint="Roland MT-32 ROMs and the SoundCanvas soundfont from eXo's support files. Games that use MIDI music need them."
                          stacked={support()?.phase === "downloading"}
                        >
                          <Show when={support()?.phase === "downloading"}>
                            <div class="pack-progress">
                              <AutoProgress value={support()!.progress} class="mini" />
                            </div>
                          </Show>
                        </SettingRow>
                      </section>
                    </div>
                  </Show>

                  <Show when={props.section === "appearance"}>
                    <div class="settings-body">
                      <section class="settings-section">
                        <SectionTitle icon={Palette}>Omarchy theme</SectionTitle>
                        <p class="settings-section-hint">The palette follows the desktop: switch with <code class="setting-code">omarchy theme set</code> and Exorchy changes with it, live.</p>
                        <SettingRow label="Theme" value={themeName()} hint={theme()?.source === "omarchy" ? "Read from Omarchy's current theme." : "No Omarchy theme found - the built-in colours are used."} />
                        <SettingRow label="Mode" value={theme()?.palette.mode === "light" ? "Light" : "Dark"} />
                      </section>
                      <section class="settings-section">
                        <SectionTitle icon={Type}>Type</SectionTitle>
                        <SettingRow label="Monospace font" value={theme()?.monoFont ?? "System default"} hint="Omarchy's terminal font, used for paths and codes." />
                        <SettingRow label="Base size" value={theme() ? `${theme()!.fontBaseSize}px` : "—"} hint="Follows Omarchy's font size setting." />
                      </section>
                    </div>
                  </Show>

                  <Show when={props.section === "storage"}>
                    <div class="settings-body">
                      <StorageTab active={props.open && props.section === "storage"} onGoToPacks={() => props.onSectionChange("packs")} />
                    </div>
                  </Show>

                  <Show when={props.section === "network"}>
                    <div class="settings-body">
                      <section class="settings-section">
                        <SectionTitle icon={Download}>Downloads</SectionTitle>
                        <p class="settings-section-hint">Games come from the eXoDOS BitTorrent swarm.</p>
                        <SettingRow
                          htmlFor="online-mode"
                          label={isOffline() ? "Offline mode" : "Online mode"}
                          hint={isOffline()
                            ? "The torrent client stays off - only games already on disk can be played."
                            : "Games, previews and content packs are downloaded from the torrents."}
                        >
                          <SettingSwitch id="online-mode" checked={!isOffline()} disabled={switchingMode()} label="Online mode" onChange={(v) => void handleToggleOnline(v)} />
                        </SettingRow>
                        <Show when={modeError()}>
                          <div class="error">{modeError()}</div>
                        </Show>
                        {/* Inert but visible while offline: the choice still
                            matters for when you go back online. */}
                        <SettingRow
                          htmlFor="seeding"
                          label="Share with other users"
                          hint={isOffline()
                            ? "Nothing is shared while offline. Your choice is kept."
                            : "Uploads parts of your games to other users while Exorchy runs. Distributing game files carries legal risk in some countries."}
                        >
                          <SettingSwitch id="seeding" checked={seedingOn() && !isOffline()} disabled={isOffline()} label="Share with other users" onChange={(v) => void handleToggleSeeding(v)} />
                        </SettingRow>
                        <SettingRow label="Speed limits" hint="Whole session, both directions. Empty means unlimited." stacked>
                          <div class="limit-inputs">
                            <label class="limit-field">
                              <span>Down</span>
                              <input type="number" min="1" placeholder="∞" disabled={isOffline()}
                                value={limitDown()} onInput={(e) => setLimitDown(e.currentTarget.value)} onChange={() => void handleSaveLimits()} />
                              <span>KB/s</span>
                            </label>
                            <label class="limit-field">
                              <span>Up</span>
                              <input type="number" min="1" placeholder="∞" disabled={isOffline() || !seedingOn()}
                                value={limitUp()} onInput={(e) => setLimitUp(e.currentTarget.value)} onChange={() => void handleSaveLimits()} />
                              <span>KB/s</span>
                            </label>
                          </div>
                        </SettingRow>
                        <Show when={limitError()}>
                          <div class="error">{limitError()}</div>
                        </Show>
                      </section>

                      {/* The emulated PC's network card, not the torrent client;
                          the grant is system-wide, so the row says what it costs. */}
                      <Show when={netStatus()}>
                        {(st) => (
                          <section class="settings-section">
                            <SectionTitle icon={Network}>Windows 9x multiplayer</SectionTitle>
                            <SettingRow label="Packet capture" hint={st().detail} stacked={!!st().manual_hint}>
                              <Show when={st().can_enable || st().enabled}>
                                <Button variant="small" loading={enablingNet()} loadingLabel="Waiting…" onClick={() => void toggleWin9xNetwork(!st().enabled)}>
                                  {st().enabled ? "Remove…" : "Enable…"}
                                </Button>
                              </Show>
                              <Show when={st().manual_hint}>
                                <code class="setting-code">{st().manual_hint}</code>
                              </Show>
                            </SettingRow>
                          </section>
                        )}
                      </Show>
                    </div>
                  </Show>

                  <Show when={props.section === "packs"}>
                    <div class="settings-body">
                      <ContentPackSettings />
                    </div>
                  </Show>

                  <Show when={props.section === "about"}>
                    <div class="settings-body">
                      <section class="settings-section">
                        <SectionTitle icon={Info}>Exorchy</SectionTitle>
                        <SettingRow label="Version" value={APP_VERSION} hint="An eXoDOS launcher for Omarchy (Arch + Hyprland)." />
                        <SettingRow label="Log folder" hint={logOpenError() || "Share exorchy.log when a download stalls or the app misbehaves."}>
                          <Button variant="small" onClick={() => void handleOpenLogFolder()}>Open</Button>
                        </SettingRow>
                      </section>
                      <section class="settings-section">
                        <SectionTitle icon={Heart}>Credits</SectionTitle>
                        <p class="settings-section-hint">Exorchy is derived from Exodium by Thomas Vollstädt - nearly everything that makes it work is his.</p>
                        {/* Plain, selectable text on purpose: a link would open
                            inside the webview, and there is no opener command. */}
                        <SettingRow
                          label="eXoDOS"
                          value={<code class="setting-code">https://www.retro-exo.com/</code>}
                          hint="The collections, their metadata and the emulator configs come from the eXo project."
                        />
                        <SettingRow
                          label="Exodium"
                          value={<code class="setting-code">https://github.com/tvollstaedt/exodium</code>}
                          hint="By Thomas Vollstädt, MIT licence. Exorchy is its Omarchy port."
                        />
                        <SettingRow
                          label="Support Thomas"
                          value={<code class="setting-code">github.com/sponsors/tvollstaedt · ko-fi.com/tvollstaedt</code>}
                          hint="If Exorchy is useful to you, the person to thank is upstream."
                        />
                      </section>
                      <section class="settings-section danger">
                        <SectionTitle icon={TriangleAlert}>Danger zone</SectionTitle>
                        <SettingRow label="Factory reset" hint={props.resetError || "Clears all data and returns to setup."}>
                          <button class="btn-danger" onClick={props.onFactoryReset}>Reset…</button>
                        </SettingRow>
                      </section>
                    </div>
                  </Show>
                </div>
              </div>

              <div class="ark-dialog-actions">
                <Dialog.CloseTrigger class="btn-secondary">Close</Dialog.CloseTrigger>
              </div>
            </Dialog.Content>
          </Dialog.Positioner>
        </Portal>
      </Dialog.Root>
    </Show>
  );
}
