// Loaded by scripts/check-markdown.cjs through the Vite dev server: mounts the Live editor and
// the Source editor side by side on one note, outside the app, so both can be checked on the
// same markdown. Nothing in the app imports it, so it is not in the build.
import "../../app.css";
import { mount, unmount } from "svelte";
import MarkdownEditor from "$lib/MarkdownEditor.svelte";

(window as any).__mountMarkdown = (body: string) => {
  document.body.innerHTML = `
    <div style="display:grid;grid-template-columns:1fr 1fr;gap:8px;height:100vh">
      <div id="live" style="overflow:auto"></div>
      <div id="source" style="overflow:auto"></div>
    </div>`;
  const el = (id: string) => document.getElementById(id)!;
  const apps = [
    // The cursor starts at the top, so the note opens with a line that has no syntax to reveal.
    mount(MarkdownEditor, { target: el("live"), props: { doc: "Live\n\n" + body, mode: "live", onchange: () => {} } }),
    mount(MarkdownEditor, { target: el("source"), props: { doc: "Source\n\n" + body, mode: "source", onchange: () => {} } }),
  ];
  return {
    /** Searches the Live editor for `query`, as the find bar does. */
    find: (query: string) => (apps[0] as any).setFind({ query, replace: "", caseSensitive: false, wholeWord: false, regexp: false }),
    unmount: () => apps.forEach((a) => unmount(a)),
  };
};
