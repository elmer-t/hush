// Loaded by scripts/check-settings.cjs through the Vite dev server: mounts Settings alone,
// in a scrolling box like the workspace page. Nothing in the app imports it.
import "../../app.css";
import { mount } from "svelte";
import Settings from "$lib/Settings.svelte";

(window as any).__mountSettings = (width: number) => {
  document.body.innerHTML = `<div class="page" id="scroller" style="height:760px;width:${width}px;overflow-y:auto;padding:32px 16px 48px;box-sizing:border-box"></div>`;
  mount(Settings, { target: document.getElementById("scroller")!, props: { onreviewtag: () => {} } });
};
