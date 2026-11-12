// Before anything renders: the stored theme is applied, so the first paint is in it (#173).
// From here and not an inline script in app.html, which the CSP (`script-src 'self'`) blocks.
import { initTheme } from "$lib/theme";

initTheme();

// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
export const ssr = false;
