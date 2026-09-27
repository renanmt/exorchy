import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";

const mockInvoke = vi.mocked(invoke);

/** Returns config values by key, so a test can leave `seeding_enabled` unset
 *  while still answering the `network_mode` read that `loadNetworkMode` makes. */
function config(values: Record<string, string | null>) {
  return async (cmd: string, args: any) => {
    if (cmd === "get_config") { return values[args.key] ?? null; }
    return null;
  };
}

describe("seeding consent", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    vi.resetModules();
  });

  // The whole point of the dialog: an install that predates opt-in has no key,
  // so its wish is unknown and must be asked rather than assumed.
  it("asks when the preference was never set", async () => {
    mockInvoke.mockImplementation(config({ network_mode: "live" }));
    const { loadNetworkMode } = await import("./network");
    const { needsSeedingConsent } = await import("./seeding");
    await loadNetworkMode();

    expect(await needsSeedingConsent()).toBe(true);
  });

  it("stays quiet once the user has answered", async () => {
    for (const answer of ["1", "0"]) {
      vi.resetModules();
      mockInvoke.mockImplementation(config({ network_mode: "live", seeding_enabled: answer }));
      const { loadNetworkMode } = await import("./network");
      const { needsSeedingConsent } = await import("./seeding");
      await loadNetworkMode();

      expect(await needsSeedingConsent()).toBe(false);
    }
  });

  // Offline uploads nothing either way, so the question is noise - it comes up
  // when the user first goes online.
  it("does not ask while offline", async () => {
    mockInvoke.mockImplementation(config({ network_mode: "offline" }));
    const { loadNetworkMode } = await import("./network");
    const { needsSeedingConsent } = await import("./seeding");
    await loadNetworkMode();

    expect(await needsSeedingConsent()).toBe(false);
  });

  // Asking on a failed read would mean asking on every start; the safe state
  // (not seeding) already holds without an answer.
  it("stays quiet when the config read fails", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_config") { throw new Error("db locked"); }
      return null;
    });
    const { needsSeedingConsent } = await import("./seeding");

    expect(await needsSeedingConsent()).toBe(false);
  });
});
