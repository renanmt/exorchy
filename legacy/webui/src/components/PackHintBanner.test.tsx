import { describe, it, expect, vi, beforeEach } from "vitest";
import { render } from "solid-js/web";
import { invoke } from "@tauri-apps/api/core";

const mockInvoke = vi.mocked(invoke);

// The banner asks what the grid is rendering. Default: this collection has no
// poster dir, i.e. it is showing the bundled low-res previews.
let posterDir: string | null = null;
vi.mock("../stores/thumbnails", () => ({
  posterDirForCollection: () => posterDir,
  thumbnailDirsLoaded: () => true,
}));

// Packs come from the shared store, filled once at startup - the banner must
// not fetch per collection, or it lands after the grid and shifts it.
let storePacks: Record<string, unknown[]> = {};
const startInstall = vi.fn().mockResolvedValue(undefined);
vi.mock("../stores/contentPacks", () => ({
  packsByCollection: () => storePacks,
  activeJobs: () => ({}),
  startContentPackInstall: (...a: unknown[]) => startInstall(...a),
}));

const PACK = {
  id: "posters", display_name: "Box Art", description: "",
  size_bytes: 66_784_296, version: 1, supersedes: [],
  available: true, installed: false,
};

/** Stub the config reads and seed the pack store. */
function backend(opts: { dismissed?: string; packs?: unknown[]; mode?: string } = {}) {
  storePacks = { eXoWin3x: opts.packs ?? [PACK], eXoDOS_SLP: opts.packs ?? [PACK] };
  mockInvoke.mockImplementation((async (cmd: string, args?: { key?: string }) => {
    if (cmd === "get_config") {
      if (args?.key === "pack_hint_dismissed") { return opts.dismissed ?? null; }
      if (args?.key === "network_mode") { return opts.mode ?? "live"; }
      return null;
    }
    if (cmd === "set_config") { return null; }
    throw new Error(`unexpected command ${cmd}`);
  }) as typeof invoke);
}

async function mount(collection = "eXoWin3x") {
  const { PackHintBanner } = await import("./PackHintBanner");
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <PackHintBanner collection={collection} />, host);
  // The banner waits for the dismissed list before asking for packs, so the
  // chain is several microtasks deep. Flush generously - the negative cases
  // need a settled DOM, not a first paint.
  for (let i = 0; i < 20; i++) { await Promise.resolve(); }
  return { host, dispose };
}

describe("PackHintBanner", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    vi.resetModules();
    posterDir = null;
    storePacks = {};
  });

  it("offers an available pack that is not installed yet", async () => {
    backend();
    const { host, dispose } = await mount();
    expect(host.textContent).toContain("Better covers available");
    expect(host.textContent).toContain("Box Art");
    dispose();
  });

  // The point of the hint is that it is a one-off. Asking again after the user
  // said "not now" turns a suggestion into nagging.
  it("stays quiet for a collection the user dismissed", async () => {
    backend({ dismissed: "eXoDOS,eXoWin3x" });
    const { host, dispose } = await mount();
    expect(host.textContent).toBe("");
    dispose();
  });

  it("stays quiet when the pack is already installed", async () => {
    backend({ packs: [{ ...PACK, installed: true }] });
    const { host, dispose } = await mount();
    expect(host.textContent).toBe("");
    dispose();
  });

  // Language packs have no poster pack of their own but resolve to eXoDOS's,
  // so their covers are already sharp. Offering anything there is noise.
  it("stays quiet when the grid already has hi-res covers", async () => {
    posterDir = "/data/content/posters/eXoDOS";
    backend();
    const { host, dispose } = await mount("eXoDOS_SLP");
    expect(host.textContent).toBe("");
    dispose();
  });

  // The metadata pack does not change a single cover - it is gallery art and
  // manuals, up to 24 GB. It stays a Settings decision.
  it("never offers the metadata pack", async () => {
    backend({
      packs: [
        { ...PACK, installed: true },
        { ...PACK, id: "metadata", display_name: "Game Metadata", installed: false },
      ],
    });
    const { host, dispose } = await mount();
    expect(host.textContent).toBe("");
    dispose();
  });

  // Offline promises no network at all - CLAUDE.md §11.
  it("offers nothing while offline", async () => {
    backend({ mode: "offline" });
    const { loadNetworkMode } = await import("../stores/network");
    await loadNetworkMode();
    const { host, dispose } = await mount();
    expect(host.textContent).toBe("");
    dispose();
  });

  // Switching collections must not re-fetch: the hint has to be on screen in
  // the same frame as the grid, otherwise it drops in and shifts it down.
  it("renders without any round trip once the store is filled", async () => {
    backend();
    const { PackHintBanner } = await import("./PackHintBanner");
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PackHintBanner collection="eXoWin3x" />, host);
    for (let i = 0; i < 20; i++) { await Promise.resolve(); }
    expect(host.textContent).toContain("Better covers available");
    const packCalls = mockInvoke.mock.calls.filter((c) => c[0] === "list_content_packs");
    expect(packCalls).toHaveLength(0);
    dispose();
  });
});
