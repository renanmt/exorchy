/**
 * Which eXo collections the library shows.
 *
 * The set lives in the `collections` config key (comma list). eXoDOS is
 * always part of it: the base metadata, the DOSBox configs and the
 * emulator pack all hang off it. Everything that caches per-collection
 * state (download manager, cover dirs, content packs, installed scan)
 * re-reads the key, so a toggle re-runs those loads in dependency order.
 */
import { createSignal } from "solid-js";
import {
  getConfig, setConfig, getAvailableCollections, initDownloadManager, scanInstalledGames,
  type CollectionInfo,
} from "../api/tauri";
import { loadThumbnailDir } from "./thumbnails";
import { refreshInstalledPacks } from "./contentPacks";
import { fetchGames } from "./games";
import { showToast } from "./toasts";

export const BASE_COLLECTION = "eXoDOS";

/** Display order for the switches and the shelf. The backend lists the
 *  language packs first (their rows shadow eXoDOS's); the user reads the
 *  base collection first. Unknown ids keep the backend's order after these. */
export const COLLECTION_ORDER: readonly string[] = [
  "eXoDOS", "eXoDOS_GLP", "eXoDOS_SLP", "eXoDOS_PLP", "eXoWin3x", "eXoWin9x", "eXoScummVM",
];

/** One line per collection for the Settings hint. */
export const COLLECTION_BLURBS: Record<string, string> = {
  eXoDOS: "DOS games, the base collection - always on.",
  eXoDOS_GLP: "German releases of DOS games, with German metadata.",
  eXoDOS_SLP: "Spanish releases of DOS games, with Spanish metadata.",
  eXoDOS_PLP: "Polish releases of DOS games, with Polish metadata.",
  eXoWin3x: "Windows 3.x games, run in DOSBox Staging with a bundled Windows 3.1.",
  eXoWin9x: "Windows 9x games, needs DOSBox-X / 86Box (fetched automatically).",
  eXoScummVM: "Adventure games run through ScummVM (fetched automatically).",
};

const [enabledCollections, setEnabledSignal] = createSignal<string[]>([BASE_COLLECTION]);
const [availableCollections, setAvailable] = createSignal<CollectionInfo[]>([]);
/** Bumped after every successful change so views with their own fetches
 *  (the library shelf) know to refresh. */
const [collectionsVersion, setCollectionsVersion] = createSignal(0);
/** True while a toggle's reloads are in flight - the switches lock meanwhile. */
const [collectionsBusy, setCollectionsBusy] = createSignal(false);

export { enabledCollections, availableCollections, collectionsVersion, collectionsBusy };

export function parseCollections(raw: string | null | undefined): string[] {
  const ids = (raw ?? "").split(",").map((s) => s.trim()).filter(Boolean);
  return ids.length > 0 ? ids : [BASE_COLLECTION];
}

function sortByDisplayOrder<T extends { id: string }>(items: T[]): T[] {
  const rank = (id: string) => {
    const i = COLLECTION_ORDER.indexOf(id);
    return i === -1 ? COLLECTION_ORDER.length : i;
  };
  return [...items].sort((a, b) => rank(a.id) - rank(b.id));
}

/** Normalises a wanted set: eXoDOS always in, unknown ids dropped once the
 *  available list is known, display order. */
function normalise(ids: Iterable<string>): string[] {
  const wanted = new Set(ids);
  wanted.add(BASE_COLLECTION);
  const available = availableCollections();
  const known = available.length > 0
    ? available.map((c) => c.id)
    : [...wanted];
  return sortByDisplayOrder(known.filter((id) => wanted.has(id)).map((id) => ({ id }))).map((c) => c.id);
}

/** Reads the stored set and the catalogue's collections. Safe to call
 *  from every mount; each call re-reads. */
export async function loadCollections(): Promise<void> {
  const [raw, available] = await Promise.all([
    getConfig("collections").catch(() => null),
    getAvailableCollections().catch(() => [] as CollectionInfo[]),
  ]);
  setAvailable(sortByDisplayOrder(available));
  setEnabledSignal(normalise(parseCollections(raw)));
}

export function isCollectionEnabled(id: string): boolean {
  return enabledCollections().includes(id);
}

/**
 * Persists a new set and re-runs every per-collection load: the download
 * manager (torrent sessions per collection), cover dirs, content packs,
 * then the installed scan and the grid. Rolls the signal and the stored
 * key back when a step fails.
 */
export async function setEnabledCollections(ids: string[]): Promise<void> {
  const previous = enabledCollections();
  const next = normalise(ids);
  if (next.join(",") === previous.join(",")) { return; }
  setEnabledSignal(next);
  setCollectionsBusy(true);
  try {
    await setConfig("collections", next.join(","));
    await initDownloadManager();
    await loadThumbnailDir();
    await refreshInstalledPacks();
    await scanInstalledGames().catch(() => {});
    await fetchGames();
    setCollectionsVersion((v) => v + 1);
  } catch (e) {
    setEnabledSignal(previous);
    try { await setConfig("collections", previous.join(",")); } catch { /* the signal already tells the truth */ }
    showToast("Could not change collections", "error", { detail: String(e) });
  } finally {
    setCollectionsBusy(false);
  }
}

/** Convenience for one switch. */
export async function setCollectionEnabled(id: string, enabled: boolean): Promise<void> {
  const current = enabledCollections();
  const next = enabled ? [...current, id] : current.filter((c) => c !== id);
  await setEnabledCollections(next);
}
