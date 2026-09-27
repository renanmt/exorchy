/**
 * The Omarchy theme, applied as CSS custom properties on `<html>`.
 *
 * The backend reads `~/.local/state/omarchy/current/theme/colors.toml` and
 * pushes every change as a `theme-changed` event (see src-tauri/src/omarchy.rs).
 * `main.css` defines the `--om-*` fallbacks and derives every semantic token
 * from them with `color-mix()`, so this store only writes the raw palette,
 * the mode and the type scale. Nothing else in the UI knows about themes.
 */
import { createSignal } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { getTheme, type Theme, type ThemePalette } from "../api/tauri";

const [theme, setTheme] = createSignal<Theme | null>(null);
export { theme };

/** `--om-<key-with-dashes>` for every palette key. */
export function paletteToCssVars(p: ThemePalette): Record<string, string> {
  const vars: Record<string, string> = {};
  for (const [k, v] of Object.entries(p)) {
    if (k === "mode") continue;
    vars[`--om-${k.replace(/_/g, "-")}`] = v;
  }
  return vars;
}

/** Write the theme onto the document. Idempotent; cheap enough to call on
 *  every event (a theme switch produces one). */
export function applyTheme(t: Theme, root: HTMLElement = document.documentElement) {
  for (const [name, value] of Object.entries(paletteToCssVars(t.palette))) {
    root.style.setProperty(name, value);
  }
  root.dataset.mode = t.palette.mode === "light" ? "light" : "dark";
  root.style.setProperty("--font-size-base", `${t.fontBaseSize}px`);
  if (t.monoFont) {
    root.style.setProperty("--font-mono", `"${t.monoFont}", ui-monospace, monospace`);
  }
  // A soft switch: the class enables transitions for one frame set.
  root.classList.add("theme-transition");
  window.setTimeout(() => root.classList.remove("theme-transition"), 300);
  setTheme(t);
}

let started = false;

/** Fetch the current theme and follow changes. Safe to call more than once. */
export async function initTheme(): Promise<void> {
  if (started) return;
  started = true;
  try {
    // Listener first, so a switch during the first fetch is not lost.
    await listen<Theme>("theme-changed", (e) => applyTheme(e.payload));
  } catch {
    /* outside Tauri (tests): CSS fallbacks stay */
  }
  try {
    applyTheme(await getTheme());
  } catch {
    /* CSS fallbacks stay */
  }
}
