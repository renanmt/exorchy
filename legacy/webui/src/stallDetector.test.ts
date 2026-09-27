import { describe, it, expect, vi, afterEach } from "vitest";
import { createRoot, createSignal } from "solid-js";
import { createStallDetector } from "./stallDetector";

/** Solid queues effects until the enclosing root's body returns, so the
 *  detector has to be built inside createRoot and asserted on outside it -
 *  otherwise no value change ever reaches its tracking effect. */
function withDetector(
  initial: number,
  body: (stalled: () => boolean, setValue: (v: number) => void) => void,
) {
  const [value, setValue] = createSignal(initial);
  let stalled!: () => boolean;
  let dispose!: () => void;
  createRoot((d) => {
    dispose = d;
    stalled = createStallDetector(value);
  });
  try {
    body(stalled, setValue);
  } finally {
    dispose();
  }
}

describe("progress stall detector", () => {
  afterEach(() => vi.useRealTimers());

  // Torrent file progress lands one piece at a time, so several flat seconds
  // between jumps is a HEALTHY download. A short window made the bar alternate
  // between the real value and the indeterminate sweep for the whole download.
  it("stays determinate across multi-second gaps between piece bursts", () => {
    vi.useFakeTimers();
    withDetector(0.1, (stalled, setValue) => {
      let v = 0.1;
      for (let i = 0; i < 6; i++) {
        vi.advanceTimersByTime(5000);
        v += 0.05;
        setValue(v);
        expect(stalled()).toBe(false);
      }
    });
  });

  it("reports a stall once nothing arrives for the full window", () => {
    vi.useFakeTimers();
    withDetector(0.42, (stalled) => {
      vi.advanceTimersByTime(14_000);
      expect(stalled()).toBe(false);
      vi.advanceTimersByTime(2_000);
      expect(stalled()).toBe(true);
    });
  });

  it("clears the stall as soon as data arrives again", () => {
    vi.useFakeTimers();
    withDetector(0.42, (stalled, setValue) => {
      vi.advanceTimersByTime(16_000);
      expect(stalled()).toBe(true);
      setValue(0.5);
      expect(stalled()).toBe(false);
    });
  });

  // A finished transfer waiting on extraction must not start sweeping.
  it("never stalls at 100%", () => {
    vi.useFakeTimers();
    withDetector(1, (stalled) => {
      vi.advanceTimersByTime(60_000);
      expect(stalled()).toBe(false);
    });
  });
});
