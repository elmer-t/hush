// Loaded by scripts/check-scrollbar.cjs through the Vite dev server: mounts one editor in a
// box of a fixed size, outside the app. Nothing in the app imports it, so it is not in the build.
import "../../app.css";
import { mount, unmount } from "svelte";
import type { EditorMode } from "$lib/layout";
import MarkdownEditor from "$lib/MarkdownEditor.svelte";

let app: Record<string, any> | undefined;

(window as any).__mountEditor = (doc: string, mode: EditorMode, width = 700, height = 600) => {
  if (app) unmount(app);
  document.body.innerHTML = `<div id="box" style="width:${width}px;height:${height}px;margin:20px"></div>`;
  app = mount(MarkdownEditor, { target: document.getElementById("box")!, props: { doc, mode, onchange: () => {} } });
  /** Searches for `query` and selects its first match, as the find bar does. */
  return { find: (query: string) => app!.setFind({ query, replace: "", caseSensitive: false, wholeWord: false, regexp: false }) };
};
