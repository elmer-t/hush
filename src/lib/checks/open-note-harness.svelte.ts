// Loaded by scripts/check-open-note.cjs through the Vite dev server: mounts the whole
// workspace, outside the app, on the backend that script mocks. Nothing in the app
// imports it, so it is not in the build.
import "../../app.css";
import { mount } from "svelte";
import Workspace from "$lib/Workspace.svelte";

(window as any).__mountWorkspace = () => {
  document.body.innerHTML = "";
  mount(Workspace, { target: document.body, props: { vault: { name: "bench", path: "/bench.hush" } } });
};
