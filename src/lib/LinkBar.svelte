<script lang="ts">
  import { api } from "$lib/api";
  import type { Link } from "$lib/linkOpen";
  import { barLayout, describeLink, type LinkTarget, type WikiResolution } from "$lib/linkTarget";

  // The link bar (#131): where the hovered or focused link goes, at the bottom of the window.
  // The Live editor (#137) calls `show` and `hide`.

  let bar = $state<{ target: LinkTarget; right: boolean } | null>(null);
  let key: Element | null = null;
  let resolved = new Map<string, Promise<WikiResolution>>();

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

  /** Shows the bar for `link`, drawn as `el`; on the right when the pointer at `x`, `y` is where it would sit. */
  export function show(el: Element, link: Link, x?: number, y?: number) {
    key = el;
    const { max, right } = barLayout(window.innerWidth, window.innerHeight, x, y);
    bar = { target: describeLink(link, { kind: "pending" }, max), right };
    if (link.kind !== "wiki") return;
    void resolveWiki(link.target).then((r) => {
      if (key === el) bar = { target: describeLink(link, r, max), right };
    });
  }

  export function hide() {
    key = null;
    bar = null;
  }

  /** The element the bar is shown for, if any. */
  export function shown(): Element | null {
    return key;
  }

  /** Forgets resolved wikilinks: notes may have been made since. */
  export function reset() {
    resolved = new Map();
  }

  $effect(() => hide);
</script>

<svelte:window onblur={hide} />

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
</style>
