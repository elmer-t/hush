<script lang="ts">
  // The editor's own vertical scrollbar (#243). The native one is hidden: its handle falls
  // behind the mouse while a long note is dragged through for the first time, because the
  // note grows as the editor measures it (see `$lib/scrollbar`). This one keeps the handle
  // under the pointer and maps the pointer's place on the track onto the note's height as
  // it is at that moment. Wheel, keys and touchpad still scroll the element itself.
  import { onMount } from "svelte";
  import { dragFraction, handleGeometry, pageScrollTop, scrollTopAt } from "$lib/scrollbar";

  type Props = {
    /** The element that scrolls: the editor's scroller. */
    scroller: HTMLElement;
    /** Called when the bar appears or goes: the editor leaves room for it. */
    onshown?: (shown: boolean) => void;
  };
  let { scroller, onshown }: Props = $props();

  // A held click on the track pages again after this long, then at this pace.
  const REPEAT_DELAY_MS = 400;
  const REPEAT_EVERY_MS = 60;

  let bar: HTMLDivElement;
  let handle = $state<{ size: number; offset: number } | null>(null);
  let dragging = $state(false);
  /** The drag under way: where the pointer holds the handle, and the handle's place from 0 to 1. */
  let drag: { grab: number; fraction: number } | null = null;
  let frame = 0;
  let repeat: ReturnType<typeof setTimeout> | undefined;

  const shown = $derived(handle !== null);
  $effect(() => onshown?.(shown));

  /** Draws the handle from the scroller, or under the pointer while it is dragged. */
  function draw() {
    const track = scroller.clientHeight;
    const next = handleGeometry(scroller, track);
    if (!next) {
      endDrag();
      handle = null;
      return;
    }
    if (drag) next.offset = drag.fraction * (track - next.size);
    if (next.size !== handle?.size || next.offset !== handle?.offset) handle = next;
  }

  /** Puts the note at the dragged fraction of its height as it is now. */
  function follow() {
    if (!drag) return;
    const target = scrollTopAt(drag.fraction, scroller);
    if (Math.abs(scroller.scrollTop - target) > 0.5) scroller.scrollTop = target;
  }

  /** After a scroll from any source, or a change of the note's height. */
  function changed() {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      // The note grew under a held handle: the same fraction is further down now.
      follow();
      draw();
    });
  }

  function endDrag() {
    drag = null;
    dragging = false;
  }

  function handleDown(e: PointerEvent) {
    if (e.button !== 0 || !handle) return;
    e.preventDefault();
    e.stopPropagation();
    const el = e.currentTarget as HTMLElement;
    // Capture keeps the pointer events here, away from the editor, until the drag ends.
    el.setPointerCapture(e.pointerId);
    const grab = e.clientY - el.getBoundingClientRect().top;
    const track = bar.getBoundingClientRect();
    drag = { grab, fraction: dragFraction(e.clientY, grab, track, handle.size) };
    dragging = true;
  }

  function handleMove(e: PointerEvent) {
    if (!drag || !handle) return;
    drag.fraction = dragFraction(e.clientY, drag.grab, bar.getBoundingClientRect(), handle.size);
    follow();
    draw();
  }

  function handleUp(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement;
    if (el.hasPointerCapture(e.pointerId)) el.releasePointerCapture(e.pointerId);
    endDrag();
    draw();
  }

  /** A click on the track, above or below the handle, pages that way; held, it keeps paging up to the pointer. */
  function trackDown(e: PointerEvent) {
    if (e.button !== 0 || !handle) return;
    e.preventDefault();
    const y = e.clientY - bar.getBoundingClientRect().top;
    const page = () => {
      if (!handle) return false;
      const direction = y < handle.offset ? -1 : y > handle.offset + handle.size ? 1 : 0;
      if (direction === 0) return false;
      scroller.scrollTop = pageScrollTop(scroller, direction);
      draw();
      return true;
    };
    if (!page()) return;
    const again = () => {
      if (page()) repeat = setTimeout(again, REPEAT_EVERY_MS);
    };
    repeat = setTimeout(again, REPEAT_DELAY_MS);
  }

  const stopPaging = () => clearTimeout(repeat);

  function wheel(e: WheelEvent) {
    // Lines and pages (deltaMode 1 and 2) are rare; a line is about 40 px.
    const unit = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? scroller.clientHeight : 1;
    scroller.scrollTop += e.deltaY * unit;
  }

  onMount(() => {
    scroller.addEventListener("scroll", changed, { passive: true });
    // The note's height changes without a scroll: typing, a mode switch, lines being measured.
    const sizes = new ResizeObserver(changed);
    sizes.observe(scroller);
    for (const child of scroller.children) sizes.observe(child);
    window.addEventListener("pointerup", stopPaging);
    draw();
    return () => {
      cancelAnimationFrame(frame);
      clearTimeout(repeat);
      scroller.removeEventListener("scroll", changed);
      sizes.disconnect();
      window.removeEventListener("pointerup", stopPaging);
    };
  });
</script>

<!-- Not in the tab order, like a native scrollbar: the keyboard scrolls the editor itself. -->
<div class="bar" class:shown bind:this={bar} onpointerdown={trackDown} onwheel={wheel} aria-hidden="true">
  {#if handle}
    <!-- svelte-ignore a11y_no_static_element_interactions (pointer only, as a native scrollbar is) -->
    <div
      class="handle"
      class:dragging
      style:height="{handle.size}px"
      style:transform="translateY({handle.offset}px)"
      onpointerdown={handleDown}
      onpointermove={handleMove}
      onpointerup={handleUp}
      onpointercancel={handleUp}
    ></div>
  {/if}
</div>

<style>
  .bar {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: var(--scrollbar-size);
    display: none;
    /* Above the editor's text layers, which come later in the document. */
    z-index: 1;
    /* The handle is dragged, never the text under it. */
    user-select: none;
    touch-action: none;
  }

  .bar.shown {
    display: block;
  }

  /* The same handle as the native scrollbars of the other panes (app.css). */
  .handle {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    border: var(--scrollbar-inset) solid transparent;
    border-radius: var(--scrollbar-size);
    background: var(--scrollbar-handle);
    background-clip: content-box;
  }

  .handle:hover {
    background-color: var(--scrollbar-handle-hover);
  }

  .handle.dragging {
    background-color: var(--scrollbar-handle-active);
  }
</style>
