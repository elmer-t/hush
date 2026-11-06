<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { untrack } from "svelte";
  import { api } from "$lib/api";
  import { findPattern, step, type FindSpec } from "$lib/find";
  import { renderMarkdown } from "$lib/markdown";
  import { diagrams } from "$lib/mermaid";
  import { describeHref, describeWikilink, type LinkTarget, type WikiResolution } from "$lib/linkTarget";
  import { clear, findRanges, firstVisible, liveHighlights, paint, reveal } from "$lib/previewFind";

  type Props = {
    body: string;
    /** The reading layout of "Preview only": one centred column in a bigger size. */
    large?: boolean;
    /** A `[[wikilink]]` was clicked at viewport position `x`, `y`. */
    onwikilink?: (target: string, x: number, y: number) => void;
    /** The find bar's query, highlighted in the rendered text; null when the bar is closed. */
    find?: FindSpec | null;
    /** Whether this pane leads the find: it then has a current match and reports it. */
    findLead?: boolean;
    /** The current match (-1 for none) and the number of matches, after each change. */
    onfindresult?: (current: number, total: number) => void;
  };
  let { body, large = false, onwikilink, find = null, findLead = false, onfindresult }: Props = $props();

  const html = $derived(renderMarkdown(body));

  let article: HTMLElement;
  let mermaid: ReturnType<typeof diagrams> | undefined;

  // `{@html}` replaces every diagram placeholder on each change: diagrams are put back after it.
  $effect.pre(() => {
    html;
    untrack(() => mermaid?.snapshot());
  });

  $effect(() => {
    html;
    untrack(() => (mermaid ??= diagrams(article)).update());
  });

  $effect(() => () => mermaid?.destroy());

  let ranges: Range[] = [];
  let current = -1;
  let lastQuery = "";

  // Finds again after every render and query change; the highlights don't alter the DOM.
  $effect(() => {
    html;
    const spec = find;
    const lead = findLead;
    untrack(() => {
      clear(article);
      const pattern = spec && findPattern(spec.query, spec);
      ranges = pattern ? findRanges(article, pattern) : [];
      const key = spec ? JSON.stringify([spec.query, spec.caseSensitive, spec.wholeWord, spec.regexp]) : "";
      if (!lead || ranges.length === 0) current = -1;
      else if (key !== lastQuery || current < 0) current = firstVisible(ranges);
      else current = Math.min(current, ranges.length - 1);
      const moved = key !== lastQuery;
      lastQuery = key;
      paint(ranges, current);
      if (lead && moved) reveal(ranges[current]);
      if (lead) onfindresult?.(current, ranges.length);
    });
  });

  $effect(() => () => clear(article));

  /** Moves to the next (`dir` 1) or previous (-1) match, wrapping around. */
  export function findStep(dir: 1 | -1) {
    if (!findLead || ranges.length === 0) return;
    current = step(current, ranges.length, dir);
    clear(article);
    // The `<mark>` fallback rebuilt the text nodes, so the ranges are found again.
    const pattern = find && findPattern(find.query, find);
    if (pattern && !liveHighlights()) ranges = findRanges(article, pattern);
    paint(ranges, current);
    reveal(ranges[current]);
    onfindresult?.(current, ranges.length);
  }

  /** The text selected in the preview, if any. */
  export function selectedText(): string {
    const selection = getSelection();
    if (!selection || selection.isCollapsed || !article.contains(selection.anchorNode)) return "";
    return selection.toString();
  }

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
    // Links in diagrams are SVG `<a>` elements, which may only carry `xlink:href`.
    const href = link.getAttribute("href") ?? link.getAttribute("xlink:href") ?? "";
    if (/^(https?:|mailto:)/i.test(href)) void openUrl(href);
  }

  // The link bar (#131): where the hovered or focused link goes, at the bottom of the window.
  let bar = $state<{ target: LinkTarget; right: boolean } | null>(null);
  let barLink: Element | null = null;
  let resolved = new Map<string, Promise<WikiResolution>>();

  // A re-render may drop or change the link under the pointer, and notes may have been made since.
  $effect(() => {
    html;
    untrack(() => {
      hideBar();
      resolved = new Map();
    });
  });

  function resolveWiki(target: string): Promise<WikiResolution> {
    let p = resolved.get(target);
    if (!p) {
      p = api.resolveLink(target).then(
        (l): WikiResolution =>
          l.resolution.kind === "unique"
            ? { kind: "unique", folder: l.notes[0]?.folder ?? "", title: l.notes[0]?.title ?? target }
            : l.resolution.kind === "ambiguous"
              ? { kind: "ambiguous", count: l.resolution.ids.length }
              : { kind: "missing" },
        (): WikiResolution => ({ kind: "pending" }),
      );
      resolved.set(target, p);
    }
    return p;
  }

  /** Shows the bar for `link`; on the right when the pointer at `x`, `y` is where it would sit. */
  function showBar(link: Element, x?: number, y?: number) {
    barLink = link;
    // About 7px a character in the bar's font; it takes at most half the window.
    const max = Math.max(24, Math.floor(window.innerWidth / 2 / 7) - 2);
    const right = x !== undefined && y !== undefined && y > window.innerHeight - 48 && x < window.innerWidth / 2;
    const wiki = link.classList.contains("wikilink") ? (link as HTMLElement).dataset.target : undefined;
    if (wiki) {
      bar = { target: describeWikilink(wiki, { kind: "pending" }, max), right };
      void resolveWiki(wiki).then((r) => {
        if (barLink === link) bar = { target: describeWikilink(wiki, r, max), right };
      });
      return;
    }
    const href = link.getAttribute("href") ?? link.getAttribute("xlink:href") ?? "";
    bar = { target: describeHref(href, link.textContent ?? "", max), right };
  }

  function hideBar() {
    barLink = null;
    bar = null;
  }

  function linkAt(node: EventTarget | null): Element | null {
    return node instanceof Element ? node.closest("a") : null;
  }

  function onmouseover(e: MouseEvent) {
    const link = linkAt(e.target);
    if (link && link !== barLink) showBar(link, e.clientX, e.clientY);
  }

  function onmouseout(e: MouseEvent) {
    if (barLink && linkAt(e.relatedTarget) !== barLink) hideBar();
  }

  function onfocusin(e: FocusEvent) {
    const link = linkAt(e.target);
    if (link) showBar(link);
  }

  function onfocusout(e: FocusEvent) {
    if (linkAt(e.relatedTarget) !== barLink) hideBar();
  }

  $effect(() => hideBar);
</script>

<svelte:window onblur={hideBar} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions, a11y_mouse_events_have_key_events -->
<article
  class="preview"
  class:large
  aria-label="Preview"
  bind:this={article}
  {onclick}
  {onmouseover}
  {onmouseout}
  {onfocusin}
  {onfocusout}
>
  {@html html}
</article>

{#if bar}
  <div class="link-bar" class:right={bar.right} aria-hidden="true">
    {#if bar.target.warning}<span class="warning">⚠ {bar.target.warning}</span>{/if}
    <span class="url">{#each bar.target.parts as part}<span class:muted={part.muted}>{part.text}</span>{/each}</span>
  </div>
{/if}

<style>
  .link-bar {
    position: fixed;
    left: 0;
    bottom: 0;
    z-index: 1000;
    display: flex;
    gap: 8px;
    max-width: 50vw;
    padding: 3px 8px;
    font-family: var(--sans);
    font-size: 12px;
    line-height: 18px;
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--border);
    border-left: none;
    border-bottom: none;
    border-top-right-radius: 4px;
    box-shadow: var(--shadow);
    white-space: nowrap;
    pointer-events: none;
  }

  .link-bar.right {
    left: auto;
    right: 0;
    border-left: 1px solid var(--border);
    border-right: none;
    border-top-right-radius: 0;
    border-top-left-radius: 4px;
  }

  .link-bar .url {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .link-bar .muted {
    color: var(--muted);
  }

  .link-bar .warning {
    flex: none;
    color: var(--danger);
    font-weight: 600;
  }

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

  .preview :global(.mermaid-block) {
    margin: 0 0 16px;
    font-family: var(--sans);
  }

  .preview :global(.mermaid-block svg) {
    display: block;
    height: auto;
    margin: 0 auto;
  }

  /* The source, muted, until the diagram is ready; while it is edited, the last rendering, faded. */
  .preview :global(.mermaid-block[data-state="pending"] > .mermaid-source) {
    color: var(--muted);
  }

  .preview :global(.mermaid-block[data-stale]) {
    opacity: 0.6;
  }

  .preview :global(.mermaid-block svg p) {
    margin: 0;
  }

  .preview :global(.mermaid-error) {
    padding: 10px 14px;
    border: 1px solid var(--danger);
    border-radius: 8px;
    background: var(--danger-soft);
    font-size: 14px;
  }

  .preview :global(.mermaid-error strong) {
    color: var(--danger);
  }

  /* Parse errors point at a column with a caret line, so they need a fixed-width font. */
  .preview :global(.mermaid-error p) {
    margin: 4px 0 8px;
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre-wrap;
  }

  .preview :global(.mermaid-error pre) {
    max-height: 240px;
    margin: 0;
    overflow: auto;
  }

  .preview :global(hr) {
    border: none;
    border-top: 1px solid var(--border);
  }
</style>
