import { createSignal } from "solid-js";

export type ToastType = "info" | "success" | "warning" | "error";

export interface Toast {
  id: number;
  message: string;
  type: ToastType;
  detail?: string;
  /** Optional action button (e.g. "Restart now" on an update toast). */
  action?: { label: string; onClick: () => void };
}

const [toasts, setToasts] = createSignal<Toast[]>([]);
export { toasts };

let nextId = 1;
const timers = new Map<number, ReturnType<typeof setTimeout>>();
// Cap so a runaway error storm (e.g. download poll firing repeatedly while
// the network is down) can't fill the viewport. Oldest toasts get evicted.
const MAX_TOASTS = 5;

export function showToast(
  message: string,
  type: ToastType = "info",
  opts: { detail?: string; durationMs?: number; action?: Toast["action"] } = {},
): number {
  const id = nextId++;
  const duration = opts.durationMs ?? (type === "error" ? 8000 : 4000);
  setToasts((prev) => {
    const next = [...prev, { id, message, type, detail: opts.detail, action: opts.action }];
    if (next.length <= MAX_TOASTS) { return next; }
    // Evict overflow from the front and clear their pending timers so
    // we don't try to dismiss them after they're already gone.
    const overflow = next.length - MAX_TOASTS;
    for (let i = 0; i < overflow; i++) {
      const t = timers.get(next[i].id);
      if (t) { clearTimeout(t); timers.delete(next[i].id); }
    }
    return next.slice(overflow);
  });
  if (duration > 0) {
    timers.set(id, setTimeout(() => dismissToast(id), duration));
  }
  return id;
}

export function dismissToast(id: number) {
  const t = timers.get(id);
  if (t) {
    clearTimeout(t);
    timers.delete(id);
  }
  setToasts((prev) => prev.filter((toast) => toast.id !== id));
}
