<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { renderMarkdown } from "$lib/markdown";

  type Props = {
    body: string;
    /** The reading layout of "Preview only": one centred column in a bigger size. */
    large?: boolean;
    /** A `[[wikilink]]` was clicked at viewport position `x`, `y`. */
    onwikilink?: (target: string, x: number, y: number) => void;
  };
  let { body, large = false, onwikilink }: Props = $props();

  const html = $derived(renderMarkdown(body));

  // Wikilinks open notes; external links open in the system browser; nothing else navigates.
  function onclick(e: MouseEvent) {
    const link = (e.target as Element).closest("a");
    if (!link) return;
    e.preventDefault();
    const target = link.classList.contains("wikilink") ? link.dataset.target : undefined;
    if (target) {
      onwikilink?.(target, e.clientX, e.clientY);
      return;
    }
    const href = link.getAttribute("href") ?? "";
    if (/^(https?:|mailto:)/i.test(href)) void openUrl(href);
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<article class="preview" class:large aria-label="Preview" {onclick}>
  {@html html}
</article>

<style>
  .preview {
    padding: 24px 36px;
    font-family: var(--serif);
    font-size: 17px;
    line-height: 1.65;
    overflow-wrap: anywhere;
  }

  .preview.large {
    max-width: calc(680px + 48px);
    margin: 0 auto;
    padding: 28px 24px;
    font-size: 18px;
    line-height: 1.7;
  }

  .preview :global(:first-child) {
    margin-top: 0;
  }

  .preview :global(p),
  .preview :global(ul),
  .preview :global(ol) {
    margin: 0 0 16px;
  }

  .preview :global(ul),
  .preview :global(ol) {
    padding-left: 22px;
  }

  .preview :global(h1),
  .preview :global(h2),
  .preview :global(h3) {
    margin: 24px 0 8px;
    font-weight: 600;
    line-height: 1.3;
  }

  .preview :global(h1) {
    font-size: 26px;
  }

  .preview :global(h2) {
    font-size: 21px;
  }

  .preview :global(h3) {
    font-size: 18px;
  }

  .preview :global(a) {
    color: var(--accent);
    text-decoration: underline;
    text-decoration-thickness: 1px;
    text-underline-offset: 3px;
  }

  .preview :global(a.wikilink) {
    text-decoration-style: dotted;
  }

  .preview :global(code) {
    font-family: var(--mono);
    font-size: 0.8em;
    background: var(--code);
    border: 1px solid var(--border);
    padding: 0.1em 0.3em;
    border-radius: 4px;
  }

  .preview :global(pre) {
    background: var(--code);
    border: 1px solid var(--border);
    padding: 12px 14px;
    border-radius: 8px;
    overflow-x: auto;
    line-height: 1.6;
  }

  .preview :global(pre code) {
    background: none;
    border: none;
    padding: 0;
  }

  .preview :global(blockquote) {
    margin: 0 0 16px;
    padding-left: 16px;
    border-left: 3px solid var(--border-strong);
    color: var(--text-2);
  }

  .preview :global(table) {
    border-collapse: collapse;
    margin: 0 0 16px;
    font-family: var(--sans);
    font-size: 14px;
  }

  .preview :global(th),
  .preview :global(td) {
    border: 1px solid var(--border);
    padding: 6px 10px;
  }

  .preview :global(img) {
    max-width: 100%;
  }

  .preview :global(hr) {
    border: none;
    border-top: 1px solid var(--border);
  }
</style>
