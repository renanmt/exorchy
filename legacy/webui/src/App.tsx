import { createSignal, onMount, onCleanup, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { open } from "@tauri-apps/plugin-dialog";
import { Dialog } from "@ark-ui/solid/dialog";
import { Tooltip } from "@ark-ui/solid/tooltip";
import { Library } from "./pages/Library";
import { Setup } from "./pages/Setup";
import { SearchBar } from "./components/SearchBar";
import { WelcomeModal } from "./components/WelcomeModal";
import { SeedingConsentDialog } from "./components/SeedingConsentDialog";
import { ActivityBadge } from "./components/ActivityBadge";
import { Splash } from "./components/Splash";
import { needsSeedingConsent, applySeeding, loadSeeding } from "./stores/seeding";
import { resumeDownloads, initDependencyDownloads } from "./stores/downloads";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { SettingsDialog, type SettingsSection } from "./components/SettingsDialog";
import { resetStorageCache } from "./components/StorageTab";
import { ToastContainer } from "./components/ToastContainer";
import {
  getSetupStatus,
  initDownloadManager,
  factoryReset,
  getConfig,
  setConfig,
  scanInstalledGames,
  dataDirIsEmpty,
  ensureDosboxStaging,
} from "./api/tauri";
import { fetchGames } from "./stores/games";
import { isOffline, loadNetworkMode } from "./stores/network";
import { loadThumbnailDir } from "./stores/thumbnails";
import { refreshInstalledPacks, initContentPackEvents } from "./stores/contentPacks";
import { showToast } from "./stores/toasts";
import { startTransferPolling } from "./stores/transfer";
import { initTheme } from "./stores/theme";
import { NowPlayingBar } from "./components/NowPlayingBar";
import {
  initMusic, currentTrack, wantedTrack, playerHidden, hidePlayer, showPlayer, startShuffle, musicUnsupported,
} from "./stores/music";
import { IconMusicNote } from "./components/icons";
import "./styles/main.css";
import { Button } from "./components/Button";

type AppPhase = "loading" | "setup" | "ready";

function App() {
  const [phase, setPhase] = createSignal<AppPhase>("loading");
  const [showSettings, setShowSettings] = createSignal(false);
  const [settingsSection, setSettingsSection] = createSignal<SettingsSection>("general");
  const [showWelcomeModal, setShowWelcomeModal] = createSignal(false);
  const [showSeedingConsent, setShowSeedingConsent] = createSignal(false);
  const [dataDir, setDataDir] = createSignal("");
  const [rootFolder, setRootFolder] = createSignal("eXoDOS");
  const [resetError, setResetError] = createSignal("");
  const [resetting, setResetting] = createSignal(false);
  /** The splash stays until the library is ready (or setup shows), and at
   *  least its own minimum time - it is the app's face on every start. */
  const [splashDone, setSplashDone] = createSignal(false);

  // Derived: the actual game storage folder shown to the user.
  const gameFolderPath = () => {
    const dir = dataDir();
    if (!dir) return "";
    // "." means the data dir IS the game root (an imported eXo tree).
    if (rootFolder() === ".") { return dir; }
    return dir.replace(/\/$/, "") + "/" + rootFolder();
  };

  onMount(() => {
    // No native context menu in production; components render their own.
    if (!import.meta.env.DEV) {
      const suppress = (e: MouseEvent) => {
        // Editable fields keep the native menu - it carries cut/copy/paste.
        const t = e.target as HTMLElement | null;
        if (t?.closest('input, textarea, [contenteditable="true"]')) { return; }
        e.preventDefault();
      };
      document.addEventListener("contextmenu", suppress);
      onCleanup(() => document.removeEventListener("contextmenu", suppress));
    }
  });

  /** Make sure an emulator exists once the session is up; the pack download
   *  runs as a content-pack job the Settings → Emulators page shows. */
  const ensureEmulator = () => {
    ensureDosboxStaging()
      .then((s) => {
        if (!s.path && !isOffline()) {
          showToast("Downloading DOSBox Staging", "info", {
            detail: "The emulator is fetched once (29 MB). Games can be downloaded meanwhile.",
          });
        }
      })
      .catch(() => {});
  };

  onMount(async () => {
    // Listeners before the first invoke: the backend can start pack installs
    // on its own (emulator packs), and only the listener makes them visible.
    initTheme().catch(() => {});
    initContentPackEvents().catch(() => {});
    initDependencyDownloads().catch(() => {});
    initMusic().catch(() => {});
    try {
      const status = await getSetupStatus();
      if (status.ready) {
        setPhase("ready");
        await loadNetworkMode();
        // Covers depend on the data dir, not on the torrent session: resolved
        // first, so the grid never stands cover-less while the engine starts.
        loadThumbnailDir();
        try {
          await initDownloadManager();
        } catch (e) {
          console.error("Failed to init download manager:", e);
        }
        // After the manager exists, not before: answering the dialog applies
        // the choice to the running session.
        setShowSeedingConsent(await needsSeedingConsent());
        const dir = await getConfig("data_dir");
        if (dir) { setDataDir(dir); }
        const root = await getConfig("root_folder");
        if (root) { setRootFolder(root); }
        refreshInstalledPacks();
        startTransferPolling();
        loadSeeding();
        ensureEmulator();
        // Re-check the disk: install flags are per game and go stale behind
        // the app's back. Downloads the session resumed by itself need their
        // trackers back - after the scan.
        scanInstalledGames().then(() => { fetchGames(); void resumeDownloads(); }).catch(() => {});
        if (!isOffline()) {
          getConfig("welcome_seen").then((seen) => {
            if (seen !== "1") { setShowWelcomeModal(true); }
          }).catch(() => {});
        }
      } else {
        setPhase("setup");
      }
    } catch {
      setPhase("setup");
    }
  });

  const handleSetupComplete = async () => {
    setPhase("ready");
    await loadNetworkMode();
    const dir = await getConfig("data_dir");
    if (dir) { setDataDir(dir); }
    const root = await getConfig("root_folder");
    if (root) { setRootFolder(root); }
    loadThumbnailDir();
    refreshInstalledPacks();
    fetchGames();
    startTransferPolling();
    loadSeeding();
    ensureEmulator();
    void resumeDownloads();

    // Welcome modal once, never offline (it offers downloads); unwritten
    // `welcome_seen` re-offers it on the first online session.
    const welcomeSeen = await getConfig("welcome_seen");
    if (welcomeSeen !== "1" && !isOffline()) {
      setShowWelcomeModal(true);
    }
  };

  /** Folder the user picked but has not confirmed yet, or null. */
  const [pendingDataDir, setPendingDataDir] = createSignal<string | null>(null);

  const handleChangeDataDir = async () => {
    const selected = await open({ title: "Select new data directory", directory: true });
    if (!selected) return;
    // An empty target usually means the user expected a move: ask, but only
    // when there is something to leave behind.
    const [targetEmpty, currentEmpty] = await Promise.all([
      dataDirIsEmpty(selected).catch(() => false),
      dataDir() ? dataDirIsEmpty(dataDir()).catch(() => true) : Promise.resolve(true),
    ]);
    if (targetEmpty && !currentEmpty) {
      setPendingDataDir(selected);
      return;
    }
    await applyDataDir(selected);
  };

  /** Persist the dir and rebuild what derives from it; the rescan's count
   *  answers "did it find my games?". */
  const applyDataDir = async (selected: string) => {
    await setConfig("data_dir", selected);
    setDataDir(selected);
    resetStorageCache();
    await initDownloadManager();
    loadThumbnailDir();
    refreshInstalledPacks();
    ensureEmulator();
    try {
      const count = await scanInstalledGames(true);
      showToast(`${count} game${count !== 1 ? "s" : ""} found in the new folder`, "success");
    } catch (e) {
      showToast("No games found in that folder", "error", { detail: String(e) });
    }
    fetchGames();
  };

  const [showResetDialog, setShowResetDialog] = createSignal(false);
  const [deleteGameData, setDeleteGameData] = createSignal(false);

  const openSettings = (section: SettingsSection = "general") => {
    setSettingsSection(section);
    setShowSettings(true);
  };

  /** The answer from the one-time consent dialog. Errors propagate so the
   *  dialog can stay open and say so. */
  const handleSeedingConsent = async (enabled: boolean) => {
    await applySeeding(enabled);
    setShowSeedingConsent(false);
    showToast(
      enabled ? "Sharing with other users is on" : "Sharing with other users is off",
      "info",
      { detail: "Change it any time in Settings → Network." },
    );
  };

  const confirmReset = async () => {
    const doDelete = deleteGameData();
    setShowResetDialog(false);
    setDeleteGameData(false);
    setResetError("");
    // Close the dialog first, then overlay whatever was behind it.
    setShowSettings(false);
    setResetting(true);
    try {
      await factoryReset(doDelete);
      setPhase("setup");
      setDataDir("");
      resetStorageCache();
    } catch (e) {
      console.error("[reset] factoryReset failed:", e);
      setResetError(`Reset failed: ${e}`);
      setShowSettings(true);
    } finally {
      setResetting(false);
    }
  };

  /** With a track loaded (or being fetched) the button toggles the bar,
   *  otherwise it starts the shuffle. */
  const musicLoaded = () => currentTrack() != null || wantedTrack() != null;

  const musicButtonLabel = () => {
    if (!musicLoaded()) { return "Play music (shuffle)"; }
    return playerHidden() ? "Show player" : "Hide player";
  };

  const onMusicButton = () => {
    if (!musicLoaded()) { void startShuffle(); return; }
    if (playerHidden()) { showPlayer(); return; }
    hidePlayer();
  };

  return (
    <>
      <Splash ready={phase() !== "loading"} onDone={() => setSplashDone(true)} />

      <Show when={phase() === "setup"}>
        <Setup onComplete={handleSetupComplete} />
      </Show>

      <Show when={phase() === "ready"}>
        <div class="top-bar">
          <div class="top-bar-brand" aria-hidden="true">Exorchy</div>
          <div class="top-bar-center">
            <SearchBar />
          </div>
          <div class="top-bar-actions">
            {/* Offline is a mode with visible consequences (no downloads, no
                videos, no sharing), so it says so permanently rather than only
                inside Settings. */}
            <ActivityBadge onOpenSettings={() => openSettings("network")} />
            <Show when={!musicUnsupported()}>
              <Tooltip.Root openDelay={400}>
                <Tooltip.Trigger asChild={(props) =>
                  <button {...props()} class="icon-btn" aria-label={musicButtonLabel()} onClick={onMusicButton}>
                    <IconMusicNote />
                  </button>
                } />
                <Portal><Tooltip.Positioner><Tooltip.Content class="ark-tooltip">{musicButtonLabel()}</Tooltip.Content></Tooltip.Positioner></Portal>
              </Tooltip.Root>
            </Show>
            <Tooltip.Root openDelay={400}>
              <Tooltip.Trigger asChild={(props) =>
                <button {...props()} class="icon-btn" data-testid="open-settings" onClick={() => openSettings()}>
                  &#9881;
                </button>
              } />
              <Portal><Tooltip.Positioner><Tooltip.Content class="ark-tooltip">Settings</Tooltip.Content></Tooltip.Positioner></Portal>
            </Tooltip.Root>
          </div>
        </div>

        <SettingsDialog
          open={showSettings()}
          onOpenChange={setShowSettings}
          section={settingsSection()}
          onSectionChange={setSettingsSection}
          gameFolderPath={gameFolderPath()}
          onChangeDataDir={() => void handleChangeDataDir()}
          onFactoryReset={() => setShowResetDialog(true)}
          resetError={resetError()}
          onWentOnline={() => {
            ensureEmulator();
            needsSeedingConsent().then(setShowSeedingConsent).catch(() => {});
          }}
        />

        <Show when={showResetDialog()}>
        <Dialog.Root open={showResetDialog()} onOpenChange={(e) => { setShowResetDialog(e.open); if (!e.open) { setDeleteGameData(false); } }}>
          <Portal>
            <Dialog.Backdrop class="ark-dialog-backdrop" />
            <Dialog.Positioner class="ark-dialog-positioner">
              <Dialog.Content class="ark-dialog-content">
                <Dialog.Title class="ark-dialog-title">Factory Reset</Dialog.Title>
                <Dialog.Description class="ark-dialog-desc">
                  Clears the Exorchy database and all settings. Your downloaded game files stay on disk and can be re-imported later.
                </Dialog.Description>
                <label class="reset-option">
                  <input
                    type="checkbox"
                    checked={deleteGameData()}
                    onChange={(e) => setDeleteGameData(e.currentTarget.checked)}
                  />
                  <span>Also delete game folder{gameFolderPath() ? ` (${gameFolderPath()})` : ""}</span>
                </label>
                <Show when={deleteGameData()}>
                  <p class="reset-warning">This will permanently delete all downloaded game files. This cannot be undone.</p>
                </Show>
                <div class="ark-dialog-actions">
                  <Dialog.CloseTrigger class="btn-secondary">Cancel</Dialog.CloseTrigger>
                  <Button variant="danger" onClick={confirmReset}>Reset</Button>
                </div>
              </Dialog.Content>
            </Dialog.Positioner>
          </Portal>
        </Dialog.Root>
        </Show>

        <Library />

        {/* Last flex child on purpose: it becomes the bottom row of the shell
            without any fixed positioning, and the panel steps aside for it. */}
        <NowPlayingBar />

        <WelcomeModal
          open={showWelcomeModal() && splashDone()}
          onClose={() => setShowWelcomeModal(false)}
        />

        {/* Change points at a folder, it never moves one - so the case worth
            catching is an empty target chosen by someone who meant to
            relocate. */}
        <ConfirmDialog
          open={pendingDataDir() !== null}
          title="That folder is empty"
          message={`Exorchy will look for games in ${pendingDataDir() ?? ""}, but it does not move anything there. Your downloaded games stay in ${dataDir()} and keep using that space. Use the empty folder anyway?`}
          confirmLabel="Use it anyway"
          cancelLabel="Cancel"
          onConfirm={() => {
            const dir = pendingDataDir();
            setPendingDataDir(null);
            if (dir) { void applyDataDir(dir); }
          }}
          onClose={() => setPendingDataDir(null)}
        />
        <SeedingConsentDialog
          open={showSeedingConsent() && splashDone()}
          onDecide={handleSeedingConsent}
        />
      </Show>

      <ToastContainer />

      <Show when={resetting()}>
        <div class="reset-overlay">
          <div class="reset-overlay-card">
            <div class="reset-overlay-spinner" />
            <div class="reset-overlay-title">Resetting Exorchy…</div>
            <div class="reset-overlay-hint">Clearing library, downloads and settings. This may take a few seconds.</div>
          </div>
        </div>
      </Show>
    </>
  );
}

export default App;
