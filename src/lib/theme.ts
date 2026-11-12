// The colour theme (#173): System (follow the OS), Light or Dark. The choice holds no note
// data, so it is stored per device in localStorage like the layout (see `layout.ts`): the
// Unlock screen needs it before any vault is open.
//
// This module is the one source of truth for "is it dark". It sets `data-theme` on <html>
// to the resolved scheme, which `app.css` keys the dark tokens on, and tells listeners
// (diagrams, the graph) when the resolved scheme changes, whether by a manual switch or
// by the OS while System is chosen. Code that needs the scheme asks here, not `matchMedia`:
// the media query says what the OS wants, not what the user chose.
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { KEYS, loadStored, oneOf, saveStored } from "$lib/layout";

export type ThemePreference = "system" | "light" | "dark";
export type Scheme = "light" | "dark";

export const isThemePreference = oneOf<ThemePreference>("system", "light", "dark");

/** The stored choice; System when nothing valid is stored or storage is unavailable. */
export const loadThemePreference = (): ThemePreference =>
  loadStored<ThemePreference>(KEYS.theme, "system", isThemePreference);

/** The scheme to show for `preference`, given whether the OS asks for dark. */
export const resolveScheme = (preference: ThemePreference, osDark: boolean): Scheme =>
  preference === "system" ? (osDark ? "dark" : "light") : preference;

const DARK_QUERY = "(prefers-color-scheme: dark)";

let preference: ThemePreference = "system";
let media: MediaQueryList | null = null;
let scheme: Scheme | null = null;
const listeners = new Set<(scheme: Scheme) => void>();

const osDark = () => (media ?? (typeof window !== "undefined" ? window.matchMedia(DARK_QUERY) : null))?.matches ?? false;

/** The scheme in use now. Before `initTheme` (as in unit tests) it follows the OS. */
export const currentScheme = (): Scheme => scheme ?? resolveScheme(preference, osDark());

export const themePreference = () => preference;

/**
 * Calls `listener` with the new scheme each time it changes, after `data-theme` is set,
 * so CSS tokens read in the listener are already the new ones. Returns the unsubscribe.
 */
export function onSchemeChange(listener: (scheme: Scheme) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function apply() {
  const next = resolveScheme(preference, osDark());
  document.documentElement.dataset.theme = next;
  if (next === scheme) return;
  scheme = next;
  for (const listener of [...listeners]) listener(next);
}

/** Has the native window (title bar, native dialogs) follow the choice; `null` follows the OS. */
function syncWindow() {
  if (!isTauri()) return;
  try {
    getCurrentWindow()
      .setTheme(preference === "system" ? null : preference)
      .catch(() => {});
  } catch {
    // No window to theme (a plain browser): the page alone follows the choice.
  }
}

/**
 * Reads the stored choice and applies it. Call once, before the first render, so the
 * first paint is already in the right scheme.
 */
export function initTheme() {
  if (media || typeof window === "undefined") return;
  preference = loadThemePreference();
  media = window.matchMedia(DARK_QUERY);
  media.addEventListener("change", apply);
  apply();
  syncWindow();
}

/** Switches to `next` at once, stores it, and has the window follow. */
export function setThemePreference(next: ThemePreference) {
  preference = next;
  saveStored(KEYS.theme, next);
  apply();
  syncWindow();
}
