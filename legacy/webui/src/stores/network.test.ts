import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";

const mockInvoke = vi.mocked(invoke);

describe("network mode store", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    vi.resetModules();
  });

  it("treats an unset network_mode as live", async () => {
    mockInvoke.mockResolvedValue(null);
    const { loadNetworkMode, isOffline } = await import("./network");
    await loadNetworkMode();
    expect(isOffline()).toBe(false);
  });

  it("reads offline from config", async () => {
    mockInvoke.mockResolvedValue("offline");
    const { loadNetworkMode, isOffline } = await import("./network");
    await loadNetworkMode();
    expect(isOffline()).toBe(true);
  });

  // The backend decides whether to create a librqbit session while handling
  // init_download_manager, reading network_mode from the DB - so the write has
  // to land first or the session comes up in the mode the user just left.
  it("persists the mode before re-initializing the download manager", async () => {
    const calls: string[] = [];
    mockInvoke.mockImplementation(async (cmd: string) => {
      calls.push(cmd);
      return null;
    });
    const { applyNetworkMode, isOffline } = await import("./network");
    await applyNetworkMode("offline");
    expect(calls).toEqual(["set_config", "init_download_manager"]);
    expect(isOffline()).toBe(true);
  });

  it("reverts the signal when the switch fails", async () => {
    mockInvoke.mockRejectedValue(new Error("db locked"));
    const { applyNetworkMode, isOffline } = await import("./network");
    await expect(applyNetworkMode("offline")).rejects.toThrow("db locked");
    expect(isOffline()).toBe(false);
  });

  // A stored mode that disagrees with the running session is worse than either
  // state on its own, so a failed switch puts the config back.
  it("rolls the stored mode back when init fails", async () => {
    const calls: Array<{ cmd: string; args: any }> = [];
    mockInvoke.mockImplementation(async (cmd: string, args: any) => {
      calls.push({ cmd, args });
      if (cmd === "init_download_manager") { throw new Error("session failed"); }
      return null;
    });
    const { applyNetworkMode } = await import("./network");

    await expect(applyNetworkMode("offline")).rejects.toThrow("session failed");

    const writes = calls.filter((c) => c.cmd === "set_config");
    expect(writes.map((w) => w.args.value)).toEqual(["offline", "live"]);
  });

  // Content packs download over HTTP, so nothing in the torrent layer stops
  // them - but "Offline" must mean no traffic at all, or the badge lies.
  it("stops content pack downloads when going offline", async () => {
    mockInvoke.mockResolvedValue(null);
    const packs = await import("./contentPacks");
    const spy = vi.spyOn(packs, "cancelAllPackJobs").mockResolvedValue(1);
    const { applyNetworkMode } = await import("./network");

    const stopped = await applyNetworkMode("offline");

    expect(spy).toHaveBeenCalled();
    // Reported separately from game downloads: pack transfers do not resume.
    expect(stopped.packs).toBe(1);
    spy.mockRestore();
  });

  it("leaves content packs alone when going online", async () => {
    mockInvoke.mockResolvedValue(null);
    const packs = await import("./contentPacks");
    const spy = vi.spyOn(packs, "cancelAllPackJobs");
    const { applyNetworkMode } = await import("./network");

    await applyNetworkMode("live");

    expect(spy).not.toHaveBeenCalled();
    spy.mockRestore();
  });

  // Going offline drops the torrent managers; any poll still running would
  // read the resulting null progress as "Download didn't start".
  it("stops in-flight download tracking before going offline", async () => {
    mockInvoke.mockResolvedValue(null);
    const downloads = await import("./downloads");
    const spy = vi.spyOn(downloads, "stopAllDownloadTracking").mockReturnValue(2);
    const { applyNetworkMode } = await import("./network");

    const stopped = await applyNetworkMode("offline");

    expect(spy).toHaveBeenCalled();
    expect(stopped.downloads).toBe(2);
    spy.mockRestore();
  });

  it("leaves downloads alone when going online", async () => {
    mockInvoke.mockResolvedValue(null);
    const downloads = await import("./downloads");
    const spy = vi.spyOn(downloads, "stopAllDownloadTracking");
    const { applyNetworkMode } = await import("./network");

    await applyNetworkMode("live");

    expect(spy).not.toHaveBeenCalled();
    spy.mockRestore();
  });
});
