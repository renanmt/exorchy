import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";

const mockInvoke = vi.mocked(invoke);

const AVAILABLE = [
  { id: "eXoDOS_GLP", display_name: "German Language Pack", torrent_file: "", game_count: 10 },
  { id: "eXoDOS", display_name: "eXoDOS", torrent_file: "", game_count: 7000 },
  { id: "eXoWin9x", display_name: "eXoWin9x", torrent_file: "", game_count: 800 },
];

/** A backend that answers every command the store touches. */
function backend(overrides: Record<string, unknown> = {}) {
  const calls: Array<{ cmd: string; args: any }> = [];
  mockInvoke.mockImplementation((async (cmd: string, args: any) => {
    calls.push({ cmd, args });
    if (cmd in overrides) {
      const v = overrides[cmd];
      if (v instanceof Error) { throw v; }
      return v;
    }
    switch (cmd) {
      case "get_config": return args?.key === "collections" ? "eXoDOS" : null;
      case "get_available_collections": return AVAILABLE;
      case "get_games": return { games: [], total: 0 };
      case "list_content_packs": return [];
      case "scan_installed_games": return 0;
      default: return null;
    }
  }) as typeof invoke);
  return calls;
}

describe("collections store", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    vi.resetModules();
  });

  it("defaults to eXoDOS when the key is unset", async () => {
    backend({ get_config: null });
    const { loadCollections, enabledCollections, isCollectionEnabled } = await import("./collections");
    await loadCollections();
    expect(enabledCollections()).toEqual(["eXoDOS"]);
    expect(isCollectionEnabled("eXoDOS")).toBe(true);
    expect(isCollectionEnabled("eXoWin9x")).toBe(false);
  });

  it("orders the available list for display, eXoDOS first", async () => {
    backend();
    const { loadCollections, availableCollections } = await import("./collections");
    await loadCollections();
    expect(availableCollections().map((c) => c.id)).toEqual(["eXoDOS", "eXoDOS_GLP", "eXoWin9x"]);
  });

  it("reads the stored comma list and drops unknown ids", async () => {
    backend({ get_config: "eXoWin9x,eXoDOS,eXoNope" });
    const { loadCollections, enabledCollections } = await import("./collections");
    await loadCollections();
    expect(enabledCollections()).toEqual(["eXoDOS", "eXoWin9x"]);
  });

  // The download manager reads the key when it builds its sessions, so the
  // write has to land first; covers and packs are per collection too.
  it("persists, then reloads every per-collection cache in order", async () => {
    const calls = backend();
    const { loadCollections, setEnabledCollections, enabledCollections, collectionsVersion } = await import("./collections");
    await loadCollections();
    calls.length = 0;

    await setEnabledCollections(["eXoWin9x", "eXoDOS_GLP"]);

    const cmds = calls.map((c) => c.cmd);
    const write = calls.find((c) => c.cmd === "set_config");
    expect(write?.args).toEqual({ key: "collections", value: "eXoDOS,eXoDOS_GLP,eXoWin9x" });
    const order = ["set_config", "init_download_manager", "get_preview_dir", "list_content_packs", "scan_installed_games", "get_games"]
      .map((c) => cmds.indexOf(c));
    expect(order.every((i) => i >= 0)).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
    expect(enabledCollections()).toEqual(["eXoDOS", "eXoDOS_GLP", "eXoWin9x"]);
    expect(collectionsVersion()).toBe(1);
  });

  it("keeps eXoDOS even when asked to drop it", async () => {
    const calls = backend();
    const { loadCollections, setEnabledCollections, enabledCollections } = await import("./collections");
    await loadCollections();
    await setEnabledCollections(["eXoWin9x"]);
    expect(enabledCollections()).toEqual(["eXoDOS", "eXoWin9x"]);
    await setEnabledCollections([]);
    expect(enabledCollections()).toEqual(["eXoDOS"]);
    expect(calls.filter((c) => c.cmd === "set_config").map((c) => c.args.value)).toEqual(["eXoDOS,eXoWin9x", "eXoDOS"]);
  });

  it("rolls the signal and the stored key back when a reload fails", async () => {
    const calls = backend({ init_download_manager: new Error("session failed") });
    const toasts = await import("./toasts");
    const spy = vi.spyOn(toasts, "showToast");
    const { loadCollections, setEnabledCollections, enabledCollections, collectionsVersion } = await import("./collections");
    await loadCollections();

    await setEnabledCollections(["eXoDOS", "eXoWin9x"]);

    expect(enabledCollections()).toEqual(["eXoDOS"]);
    expect(collectionsVersion()).toBe(0);
    expect(calls.filter((c) => c.cmd === "set_config").map((c) => c.args.value)).toEqual(["eXoDOS,eXoWin9x", "eXoDOS"]);
    expect(spy).toHaveBeenCalledWith("Could not change collections", "error", expect.objectContaining({ detail: expect.stringContaining("session failed") }));
    spy.mockRestore();
  });

  it("does nothing when the set is unchanged", async () => {
    const calls = backend();
    const { loadCollections, setCollectionEnabled } = await import("./collections");
    await loadCollections();
    calls.length = 0;
    await setCollectionEnabled("eXoDOS", true);
    expect(calls).toEqual([]);
  });

  // A failed installed-scan is a warning, not a reason to undo the toggle.
  it("survives a failing installed scan", async () => {
    backend({ scan_installed_games: new Error("disk") });
    const { loadCollections, setCollectionEnabled, enabledCollections } = await import("./collections");
    await loadCollections();
    await setCollectionEnabled("eXoWin9x", true);
    expect(enabledCollections()).toEqual(["eXoDOS", "eXoWin9x"]);
  });
});
