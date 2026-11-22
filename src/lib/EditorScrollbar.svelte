<script lang="ts">
  // The editor's own vertical scrollbar (#243). The native one is hidden: its handle falls
  // behind the mouse while a long note is dragged through for the first time, because the
  // note grows as the editor measures it (see `$lib/scrollbar`). This one keeps the handle
  // under the pointer and maps the pointer's place on the track onto the note's height as
  // it is at that moment. Wheel, keys and touchpad still scroll the element itself.
  import { onMount } from "svelte";
  import { dragFraction, handleGeometry, pageScrollTop, scrollTopAt, type Handle } from "$lib/scrollbar";

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
  // After the handle is let go the editor may still measure the lines it just drew: the note
  // stays at the released place for this long, unless something else scrolls it.
  const SETTLE_MS = 300;
  // A wheel that reports lines, not pixels: about this much per line.
  const WHEEL_LINE_PX = 40;

  let bar: HTMLDivElement;
  let handle = $state<Handle | null>(null);
  let dragging = $state(false);
  /** The drag under way: where the pointer holds the handle, and the handle's place from 0 to 1. */
  let drag: { grab: number; fraction: number } | null = null;
  /** The place a drag ended at, from 0 to 1, while the note's height may still change under it. */
  let settling: number | null = null;
  let settleTimer: ReturnType<typeof setTimeout> | undefined;
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
      stopSettling();
      clearTimeout(repeat);
      handle = null;
      return;
    }
    if (drag) next.offset = drag.fraction * (track - next.size);
    if (next.size !== handle?.size || next.offset !== handle?.offset) handle = next;
  }

  /** Puts the note at the dragged fraction of its height as it is now. */
  function follow() {
    const fraction = drag?.fraction ?? settling;
    if (fraction == null) return;
    const target = scrollTopAt(fraction, scroller);
    if (Math.abs(scroller.scrollTop - target) > 0.5) scroller.scrollTop = target;
  }

  /** After a scroll from any source, or a change of the note's height. */
  function changed() {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      // The note grew under a held or just released handle: the same fraction is further down now.
      follow();
      draw();
    });
  }

  /** The track on screen, with the length the handle is drawn against. */
  const track = () => ({ top: bar.getBoundingClientRect().top, height: scroller.clientHeight });

  function endDrag() {
    drag = null;
    dragging = false;
  }

  /** Something else scrolls the note now: the released place no longer holds. */
  function stopSettling() {
    clearTimeout(settleTimer);
    settling = null;
  }

  function handleDown(e: PointerEvent) {
    if (e.button !== 0 || !handle) return;
    e.preventDefault();
    e.stopPropagation();
    const el = e.currentTarget as HTMLElement;
    // Capture keeps the pointer events here, away from the editor, until the drag ends.
    el.setPointerCapture(e.pointerId);
    stopSettling();
    const grab = e.clientY - el.getBoundingClientRect().top;
    drag = { grab, fraction: dragFraction(e.clientY, grab, track(), handle.size) };
    dragging = true;
  }

  function handleMove(e: PointerEvent) {
    if (!drag || !handle) return;
    drag.fraction = dragFraction(e.clientY, drag.grab, track(), handle.size);
    follow();
    draw();
  }

  function handleUp(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement;
    if (el.hasPointerCapture(e.pointerId)) el.releasePointerCapture(e.pointerId);
    if (drag) {
      stopSettling();
      settling = drag.fraction;
      settleTimer = setTimeout(stopSettling, SETTLE_MS);
    }
    endDrag();
    draw();
  }

  /** A click on the track, above or below the handle, pages that way; held, it keeps paging up to the pointer. */
  function trackDown(e: PointerEvent) {
    if (e.button !== 0 || !handle) return;
    e.preventDefault();
    stopSettling();
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
    // With Ctrl held the wheel zooms.
    if (e.ctrlKey) return;
    stopSettling();
    const unit = e.deltaMode === 1 ? WHEEL_LINE_PX : e.deltaMode === 2 ? scroller.clientHeight : 1;
    scroller.scrollTop += e.deltaY * unit;
  }

  onMount(() => {
    scroller.addEventListener("scroll", changed, { passive: true });
    // The note's height changes without a scroll: typing, a mode switch, lines being measured.
    const sizes = new ResizeObserver(changed);
    sizes.observe(scroller);
    for (const child of scroller.children) sizes.observe(child);
    window.addEventListener("pointerup", stopPaging);
    window.addEventListener("pointercancel", stopPaging);
    window.addEventListener("blur", stopPaging);
    // Wheel, keys and clicks in the editor scroll it themselves.
    const own = ["wheel", "keydown", "pointerdown"] as const;
    for (const type of own) scroller.addEventListener(type, stopSettling, { passive: true });
    draw();
    return () => {
      cancelAnimationFrame(frame);
      clearTimeout(repeat);
      clearTimeout(settleTimer);
      for (const type of own) scroller.removeEventListener(type, stopSettling);
      scroller.removeEventListener("scroll", changed);
      sizes.disconnect();
      window.removeEventListener("pointerup", stopPaging);
      window.removeEventListener("pointercancel", stopPaging);
      window.removeEventListener("blur", stopPaging);
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
      onlostpointercapture={handleUp}
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

  /* High contrast replaces backgrounds: the handle keeps a system colour that shows. */
  @media (forced-colors: active) {
    .handle,
    .handle:hover,
    .handle.dragging {
      forced-color-adjust: none;
      background-color: ButtonText;
    }
  }
</style>
