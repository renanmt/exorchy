import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";

const mockInvoke = vi.mocked(invoke);

describe("transfer stats", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    vi.resetModules();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // Keep-alive traffic never quite reaches zero; a flickering "312 B/s" in the
  // top bar reads as activity when there is none.
  it("shows sub-KB traffic as idle", async () => {
    const { formatRate } = await import("./transfer");
    expect(formatRate(0)).toBe("0 KB/s");
    expect(formatRate(900)).toBe("0 KB/s");
    expect(formatRate(1024)).toBe("1 KB/s");
    expect(formatRate(1536 * 1024)).toBe("1.5 MB/s");
  });

  it("polls and exposes the stats", async () => {
    mockInvoke.mockResolvedValue({
      download_bps: 2048, upload_bps: 1024, uploaded_bytes: 99, active: true,
    });
    const { startTransferPolling, stopTransferPolling, transferStats } = await import("./transfer");

    startTransferPolling();
    await vi.advanceTimersByTimeAsync(0);

    expect(transferStats()?.download_bps).toBe(2048);
    stopTransferPolling();
  });

  // Offline drops every torrent manager, so the command can only answer zeroes
  // - and "Offline" must not sit next to a live-looking readout.
  it("reports nothing while offline", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_config") { return "offline"; }
      throw new Error("get_transfer_stats should not be called while offline");
    });
    const { loadNetworkMode } = await import("./network");
    const { startTransferPolling, stopTransferPolling, transferStats } = await import("./transfer");
    await loadNetworkMode();

    startTransferPolling();
    await vi.advanceTimersByTimeAsync(0);

    expect(transferStats()).toBeNull();
    expect(mockInvoke).not.toHaveBeenCalledWith("get_transfer_stats", expect.anything());
    stopTransferPolling();
  });

  it("survives a failing poll without wedging the loop", async () => {
    mockInvoke.mockRejectedValue(new Error("no manager"));
    const { startTransferPolling, stopTransferPolling, transferStats } = await import("./transfer");

    startTransferPolling();
    await vi.advanceTimersByTimeAsync(0);
    expect(transferStats()).toBeNull();

    mockInvoke.mockResolvedValue({
      download_bps: 4096, upload_bps: 0, uploaded_bytes: 0, active: true,
    });
    await vi.advanceTimersByTimeAsync(4000);
    expect(transferStats()?.download_bps).toBe(4096);
    stopTransferPolling();
  });

  // Pieces arrive in bursts, so a mid-download sample regularly reads idle.
  // Acting on that single sample made the badge swap between a rate readout
  // and plain "Online" every few seconds.
  it("keeps a transfer active across a sampling dip", async () => {
    mockInvoke.mockResolvedValue({
      download_bps: 500_000, upload_bps: 0, uploaded_bytes: 0, active: true,
    });
    const { startTransferPolling, stopTransferPolling, isTransferring } =
      await import("./transfer");

    startTransferPolling();
    await vi.advanceTimersByTimeAsync(0);
    expect(isTransferring()).toBe(true);

    // One idle sample must not flip it...
    mockInvoke.mockResolvedValue({
      download_bps: 0, upload_bps: 0, uploaded_bytes: 0, active: true,
    });
    await vi.advanceTimersByTimeAsync(1500);
    expect(isTransferring()).toBe(true);

    // ...but a genuinely finished download settles.
    await vi.advanceTimersByTimeAsync(12_000);
    expect(isTransferring()).toBe(false);
    stopTransferPolling();
  });

  it("does not start a second loop", async () => {
    mockInvoke.mockResolvedValue({
      download_bps: 0, upload_bps: 0, uploaded_bytes: 0, active: false,
    });
    const { startTransferPolling, stopTransferPolling } = await import("./transfer");

    startTransferPolling();
    startTransferPolling();
    await vi.advanceTimersByTimeAsync(0);

    const calls = mockInvoke.mock.calls.filter((c) => c[0] === "get_transfer_stats");
    expect(calls).toHaveLength(1);
    stopTransferPolling();
  });
});
