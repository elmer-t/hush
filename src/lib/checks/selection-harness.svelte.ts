// Loaded by scripts/check-selection.cjs through the Vite dev server: mounts the editor alone,
// in Source or Live, next to a plain input. Nothing in the app imports it, so it is not in
// the build.
import "../../app.css";
import { mount } from "svelte";
import MarkdownEditor from "$lib/MarkdownEditor.svelte";
import type { EditorMode } from "$lib/layout";

(window as any).__mountEditor = (doc: string, mode: EditorMode) => {
  document.body.innerHTML = `<input id="field" value="some text in a field" style="width:300px"><div id="editor" style="height:500px;width:800px"></div>`;
  const editor: any = mount(MarkdownEditor, {
    target: document.getElementById("editor")!,
    props: { doc, mode, onchange: () => {} },
  });
  return { find: (query: string) => editor.setFind({ query, caseSensitive: false, wholeWord: false, regexp: false, replace: "" }) };
};
