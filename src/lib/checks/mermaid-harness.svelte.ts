// Loaded by scripts/check-mermaid.cjs and scripts/check-link-bar.cjs through the Vite dev
// server: mounts the Live editor alone, outside the app. Nothing in the app imports it, so it
// is not in the build.
import "../../app.css";
import { mount, unmount } from "svelte";
import MarkdownEditor from "$lib/MarkdownEditor.svelte";

/** `height`: of the editor, in px. CodeMirror only draws what is in view, so a check that reads every diagram of a note needs it tall (and the window too). */
(window as any).__mountLive = (body: string, height = 700) => {
  document.body.innerHTML = `<div id="scroller" style="height:${height}px;width:900px"></div>`;
  const app: any = mount(MarkdownEditor, {
    target: document.getElementById("scroller")!,
    props: { doc: body, mode: "live", onchange: () => {} },
  });
  // `set` changes the note as typing does: one change covering only what differs.
  return { set: (b: string) => app.replace(b), unmount: () => unmount(app) };
};
