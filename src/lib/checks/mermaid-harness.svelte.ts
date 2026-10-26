// Loaded by scripts/check-mermaid.cjs through the Vite dev server: mounts the preview alone,
// outside the app. Nothing in the app imports it, so it is not in the build.
import "../../app.css";
import { mount, unmount } from "svelte";
import Preview from "$lib/Preview.svelte";

(window as any).__mountPreview = (body: string) => {
  document.body.innerHTML = '<div id="scroller" style="height:700px;width:900px;overflow:auto"></div>';
  const props = $state({ body });
  const app = mount(Preview, { target: document.getElementById("scroller")!, props });
  return { set: (b: string) => (props.body = b), unmount: () => unmount(app) };
};
