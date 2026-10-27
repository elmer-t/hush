<script lang="ts">
  type Props = {
    /** The panel's width now, or its height for a handle on its top edge. */
    size: number;
    min: number;
    max: number;
    label: string;
    /** Which edge of its panel the handle is on: dragging away from the panel grows it. */
    edge: "right" | "left" | "top";
    /** Called with each new size while it changes. */
    onresize: (size: number) => void;
    /** Called once a drag or key press is done, with the final size. */
    oncommit: (size: number) => void;
    onreset: () => void;
  };
  let { size, min, max, label, edge, onresize, oncommit, onreset }: Props = $props();

  const STEP = 16;

  let drag: { at: number; size: number } | null = $state(null);

  const clamp = (w: number) => Math.round(Math.min(max, Math.max(min, w)));
  // A handle on a top edge resizes rows and follows the pointer's y; the others follow x.
  const rows = $derived(edge === "top");
  const pointerAt = (e: PointerEvent) => (rows ? e.clientY : e.clientX);
  // Pointer movement to the right widens a column whose handle is on its right edge;
  // movement up or to the left grows a panel whose handle is on its top or left edge.
  const sign = $derived(edge === "right" ? 1 : -1);

  function pointerdown(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    // Capture keeps the pointer events here, away from the editor, until the drag ends.
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = { at: pointerAt(e), size };
    document.body.classList.add("resizing");
    document.body.classList.toggle("rows", rows);
  }

  function pointermove(e: PointerEvent) {
    if (!drag) return;
    onresize(clamp(drag.size + sign * (pointerAt(e) - drag.at)));
  }

  function pointerup(e: PointerEvent) {
    if (!drag) return;
    const el = e.currentTarget as HTMLElement;
    if (el.hasPointerCapture(e.pointerId)) el.releasePointerCapture(e.pointerId);
    drag = null;
    document.body.classList.remove("resizing", "rows");
    oncommit(size);
  }

  function keydown(e: KeyboardEvent) {
    let next: number | null = null;
    if (e.key === (rows ? "ArrowUp" : "ArrowLeft")) next = size + (rows ? STEP : -sign * STEP);
    else if (e.key === (rows ? "ArrowDown" : "ArrowRight")) next = size - (rows ? STEP : -sign * STEP);
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
  aria-orientation={rows ? "horizontal" : "vertical"}
  aria-label={label}
  aria-valuemin={min}
  aria-valuemax={max}
  aria-valuenow={size}
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

  .handle.top {
    align-self: start;
    width: auto;
    height: 8px;
    margin-top: -4px;
    cursor: row-resize;
  }

  /* The visible line, centred on the column border. */
  .handle::after {
    content: "";
    position: absolute;
    inset: 0 3px;
    border-radius: 1px;
    transition: background 0.12s;
  }

  .handle.top::after {
    inset: 3px 0;
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

  :global(body.resizing.rows) {
    cursor: row-resize;
  }
</style>
