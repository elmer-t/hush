// Loaded by scripts/check-markdown.cjs through the Vite dev server: mounts the Preview, the
// Live editor and the Source editor side by side on one note, outside the app, so the
// renderings of the same markdown can be compared. Nothing in the app imports it, so it is not in the build.
import "../../app.css";
import { mount, unmount } from "svelte";
import MarkdownEditor from "$lib/MarkdownEditor.svelte";
import Preview from "$lib/Preview.svelte";

(window as any).__mountMarkdown = (body: string) => {
  document.body.innerHTML = `
    <div style="display:grid;grid-template-columns:1fr 1fr 1fr;gap:8px;height:100vh">
      <div id="preview" style="overflow:auto"></div>
      <div id="live" style="overflow:auto"></div>
      <div id="source" style="overflow:auto"></div>
    </div>`;
  const el = (id: string) => document.getElementById(id)!;
  const apps = [
    mount(Preview, { target: el("preview"), props: { body, large: true } }),
    // The cursor starts at the top, so the note opens with a line that has no syntax to reveal.
    mount(MarkdownEditor, { target: el("live"), props: { doc: "Live\n\n" + body, mode: "live", onchange: () => {} } }),
    mount(MarkdownEditor, { target: el("source"), props: { doc: "Source\n\n" + body, mode: "source", onchange: () => {} } }),
  ];
  return { unmount: () => apps.forEach((a) => unmount(a)) };
};
