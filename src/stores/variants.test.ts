import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";

const mockInvoke = vi.mocked(invoke);

/** Minimal row: loadVariants only reads these two fields. */
const g = (shortcode: string, torrent_source = "eXoDOS") =>
  ({ shortcode, torrent_source }) as never;

describe("variant cache", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    vi.resetModules();
  });

  // Rendering the whole catalogue mounts one card per game; before the cache
  // each multi-language card fired its own get_game_variants (~734 in a burst).
  it("shares one request between concurrent callers", async () => {
    mockInvoke.mockResolvedValue([{ id: 1 }]);
    const { loadVariants } = await import("./variants");

    const results = await Promise.all([
      loadVariants(g("MagCarp")),
      loadVariants(g("MagCarp")),
      loadVariants(g("MagCarp")),
    ]);

    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(results.every((r) => r.length === 1)).toBe(true);
  });

  it("serves later callers from cache", async () => {
    mockInvoke.mockResolvedValue([{ id: 1 }]);
    const { loadVariants } = await import("./variants");

    await loadVariants(g("SQ5"));
    await loadVariants(g("SQ5"));
    expect(mockInvoke).toHaveBeenCalledTimes(1);
  });

  it("refetches when forced - install state must not be served stale", async () => {
    mockInvoke.mockResolvedValue([{ id: 1 }]);
    const { loadVariants } = await import("./variants");

    await loadVariants(g("DESCENT"));
    await loadVariants(g("DESCENT"), true);
    expect(mockInvoke).toHaveBeenCalledTimes(2);
  });

  // The same shortcode names different games in different packs, so the two
  // must not share a cache slot (eXoWin3x reuses ten eXoDOS codes).
  it("keys the cache by collection as well as shortcode", async () => {
    mockInvoke.mockResolvedValue([{ id: 1 }]);
    const { loadVariants } = await import("./variants");

    await loadVariants(g("EarthQue"));
    await loadVariants(g("EarthQue", "eXoWin3x"));
    expect(mockInvoke).toHaveBeenCalledTimes(2);
  });

  // A failed lookup must not be remembered, or one hiccup would leave a card
  // without variants for the rest of the session.
  it("does not cache failures", async () => {
    mockInvoke.mockRejectedValueOnce(new Error("db locked"));
    mockInvoke.mockResolvedValue([{ id: 2 }]);
    const { loadVariants } = await import("./variants");

    await expect(loadVariants(g("BOOM"))).rejects.toThrow("db locked");
    const second = await loadVariants(g("BOOM"));
    expect(second).toEqual([{ id: 2 }]);
    expect(mockInvoke).toHaveBeenCalledTimes(2);
  });
});
