<script lang="ts">
  type Props = {
    /** The column's width now. */
    width: number;
    min: number;
    max: number;
    label: string;
    /** Which edge of its column the handle is on: dragging away from the column widens it. */
    edge: "right" | "left";
    /** Called with each new width while it changes. */
    onresize: (width: number) => void;
    /** Called once a drag or key press is done, with the final width. */
    oncommit: (width: number) => void;
    onreset: () => void;
  };
  let { width, min, max, label, edge, onresize, oncommit, onreset }: Props = $props();

  const STEP = 16;

  let drag: { x: number; width: number } | null = $state(null);

  const clamp = (w: number) => Math.round(Math.min(max, Math.max(min, w)));
  // Pointer movement to the right widens a column whose handle is on its right edge.
  const sign = $derived(edge === "right" ? 1 : -1);

  function pointerdown(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    // Capture keeps the pointer events here, away from the editor, until the drag ends.
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = { x: e.clientX, width };
    document.body.classList.add("resizing");
  }

  function pointermove(e: PointerEvent) {
    if (!drag) return;
    onresize(clamp(drag.width + sign * (e.clientX - drag.x)));
  }

  function pointerup(e: PointerEvent) {
    if (!drag) return;
    const el = e.currentTarget as HTMLElement;
    if (el.hasPointerCapture(e.pointerId)) el.releasePointerCapture(e.pointerId);
    drag = null;
    document.body.classList.remove("resizing");
    oncommit(width);
  }

  function keydown(e: KeyboardEvent) {
    let next: number | null = null;
    if (e.key === "ArrowLeft") next = width - sign * STEP;
    else if (e.key === "ArrowRight") next = width + sign * STEP;
    else if (e.key === "Home") next = min;
    else if (e.key === "End") next = max;
    else if (e.key === "Enter") {
      e.preventDefault();
      onreset();
      return;
    } else return;
    e.preventDefault();
    const w = clamp(next);
    onresize(w);
    oncommit(w);
  }
</script>

<!-- A focusable separator with a value is a widget (WAI-ARIA window splitter); Svelte's check doesn't know that. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="handle {edge}"
  class:dragging={drag !== null}
  role="separator"
  aria-orientation="vertical"
  aria-label={label}
  aria-valuemin={min}
  aria-valuemax={max}
  aria-valuenow={width}
  tabindex="0"
  title="Drag to resize, double-click to reset"
  onpointerdown={pointerdown}
  onpointermove={pointermove}
  onpointerup={pointerup}
  onpointercancel={pointerup}
  onlostpointercapture={pointerup}
  ondblclick={onreset}
  onkeydown={keydown}
></div>

<style>
  /* Placed in its column's grid cell, over the column's border, so it takes no space. */
  .handle {
    position: relative;
    z-index: 5;
    width: 8px;
    min-height: 0;
    cursor: col-resize;
    touch-action: none;
  }

  .handle.right {
    justify-self: end;
    margin-right: -4px;
  }

  .handle.left {
    justify-self: start;
    margin-left: -4px;
  }

  /* The visible line, centred on the column border. */
  .handle::after {
    content: "";
    position: absolute;
    inset: 0 3px;
    border-radius: 1px;
    transition: background 0.12s;
  }

  .handle:hover::after,
  .handle.dragging::after {
    background: var(--accent);
  }

  .handle:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
    border-radius: 3px;
  }

  :global(body.resizing) {
    cursor: col-resize;
    user-select: none;
  }
</style>
