<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { renderMarkdown } from "$lib/markdown";

  let { body }: { body: string } = $props();

  const html = $derived(renderMarkdown(body));

  // Nothing in the preview navigates the app; external links open in the system browser.
  function onclick(e: MouseEvent) {
    const link = (e.target as Element).closest("a");
    if (!link) return;
    e.preventDefault();
    const href = link.getAttribute("href") ?? "";
    if (/^(https?:|mailto:)/i.test(href)) void openUrl(href);
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<article class="preview" {onclick}>
  {@html html}
</article>

<style>
  .preview {
    padding: 1rem 1.25rem;
    line-height: 1.6;
    overflow-wrap: anywhere;
  }

  .preview :global(:first-child) {
    margin-top: 0;
  }

  .preview :global(a) {
    color: var(--accent);
  }

  .preview :global(code) {
    font-family: var(--mono);
    font-size: 0.9em;
    background: var(--active-line);
    padding: 0.1em 0.3em;
    border-radius: 4px;
  }

  .preview :global(pre) {
    background: var(--active-line);
    padding: 0.75em 1em;
    border-radius: 6px;
    overflow-x: auto;
  }

  .preview :global(pre code) {
    background: none;
    padding: 0;
  }

  .preview :global(blockquote) {
    margin-left: 0;
    padding-left: 1em;
    border-left: 3px solid var(--border);
    color: var(--muted);
  }

  .preview :global(table) {
    border-collapse: collapse;
  }

  .preview :global(th),
  .preview :global(td) {
    border: 1px solid var(--border);
    padding: 0.3em 0.6em;
  }

  .preview :global(img) {
    max-width: 100%;
  }

  .preview :global(hr) {
    border: none;
    border-top: 1px solid var(--border);
  }
</style>
