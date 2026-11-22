<script lang="ts">
  import { onMount } from "svelte";
  import { isTauri } from "@tauri-apps/api/core";
  import { api } from "$lib/api";
  import "../app.css";

  let { children } = $props();

  // The window starts hidden, so nobody sees it empty or at the wrong size (#239). Not
  // after an animation frame: a hidden webview may not run any.
  onMount(() => {
    if (isTauri()) api.windowReady().catch(() => {});
  });
</script>

{@render children()}
