import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";

const mockInvoke = vi.mocked(invoke);

function config(values: Record<string, string | null>) {
  return async (cmd: string, args: any) => {
    if (cmd === "get_config") { return values[args.key] ?? null; }
    return null;
  };
}

describe("dismissed compatibility notes", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    vi.resetModules();
  });

  // Until the stored list arrives, "nothing dismissed" would be a guess - and
  // a wrong one flashes a note the user silenced weeks ago on every start.
  it("reports not-loaded until the config read resolves", async () => {
    mockInvoke.mockImplementation(config({}));
    const notes = await import("./notes");

    expect(notes.dismissedNotesLoaded()).toBe(false);
    notes.ensureDismissedNotesLoaded();
    await vi.waitFor(() => expect(notes.dismissedNotesLoaded()).toBe(true));
  });

  it("remembers a note the user dismissed in an earlier session", async () => {
    mockInvoke.mockImplementation(config({ dismissed_notes: "ece,x98-boot" }));
    const notes = await import("./notes");
    notes.ensureDismissedNotesLoaded();
    await vi.waitFor(() => expect(notes.dismissedNotesLoaded()).toBe(true));

    expect(notes.isNoteDismissed("ece")).toBe(true);
    expect(notes.isNoteDismissed("86box-perf")).toBe(false);
  });

  it("appends to the stored list rather than replacing it", async () => {
    mockInvoke.mockImplementation(config({ dismissed_notes: "ece" }));
    const notes = await import("./notes");
    notes.ensureDismissedNotesLoaded();
    await vi.waitFor(() => expect(notes.dismissedNotesLoaded()).toBe(true));

    await notes.dismissNote("printing");

    const write = mockInvoke.mock.calls.find(
      ([cmd, args]: any) => cmd === "set_config" && args?.key === "dismissed_notes",
    );
    expect(write?.[1]).toMatchObject({ value: "ece,printing" });
    expect(notes.isNoteDismissed("ece")).toBe(true);
  });

  // A failed write must not un-hide what the user just dismissed; the click
  // has to hold for this session even if the config never lands.
  it("keeps the note hidden when the write fails", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_config") { return null; }
      if (cmd === "set_config") { throw new Error("disk full"); }
      return null;
    });
    const notes = await import("./notes");
    notes.ensureDismissedNotesLoaded();
    await vi.waitFor(() => expect(notes.dismissedNotesLoaded()).toBe(true));

    await notes.dismissNote("ece");

    expect(notes.isNoteDismissed("ece")).toBe(true);
  });
});
