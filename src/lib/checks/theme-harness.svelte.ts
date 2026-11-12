// Loaded by scripts/check-theme.cjs through the Vite dev server: mounts the Settings page,
// the Preview, the Live editor and the graph side by side, so one theme switch can be seen
// in all of them. Nothing in the app imports it, so it is not in the build.
import "../../app.css";
import { mount } from "svelte";
import GraphView from "$lib/GraphView.svelte";
import MarkdownEditor from "$lib/MarkdownEditor.svelte";
import Preview from "$lib/Preview.svelte";
import Settings from "$lib/Settings.svelte";
import * as theme from "$lib/theme";

(window as any).__theme = theme;

(window as any).__mountAll = (body: string) => {
  document.body.innerHTML = `
    <div style="display:grid;grid-template-columns:1fr 1fr;gap:8px;height:100vh">
      <div id="settings" style="overflow:auto;height:50vh"></div>
      <div id="graph" style="height:50vh;position:relative"></div>
      <div id="preview" style="overflow:auto;height:48vh"></div>
      <div id="live" style="overflow:auto;height:48vh"></div>
    </div>`;
  const el = (id: string) => document.getElementById(id)!;
  mount(Settings, { target: el("settings"), props: { onreviewtag: () => {} } });
  mount(Preview, { target: el("preview"), props: { body } });
  mount(MarkdownEditor, { target: el("live"), props: { doc: "Text\n\n" + body, mode: "live", onchange: () => {} } });
  mount(GraphView, { target: el("graph"), props: { centre: { id: 1, folder: "" }, onopen: () => {}, onedit: () => {} } });
};
