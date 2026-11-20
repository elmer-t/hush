// Before anything renders: the stored theme is applied, so the first paint is in it (#173).
// From here and not an inline script in app.html, which the CSP (`script-src 'self'`) blocks.
import { api } from "$lib/api";
import { setDateLocale } from "$lib/dates";
import { initTheme } from "$lib/theme";

initTheme();

// Dates follow the operating system's format (#202). Waited for here, so nothing renders
// a date in the webview's default first. If it can't be read, that default stays.
export const load = async () => {
  try {
    setDateLocale(await api.osLocale());
  } catch {
    // Keep the webview's default.
  }
};

// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
export const ssr = false;
