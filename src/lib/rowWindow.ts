/**
 * Which rows of a long list of equal-height rows to draw: those in view and a few around
 * them. The note tree draws only these, so what a note switch costs does not grow with the
 * number of notes the expanded folders hold.
 */

/** Rows drawn above and below the ones in view, so a scroll shows rows at once. */
export const OVERSCAN = 12;

/** The rows `first`..`last` (`last` not included) to draw for a list of `count` rows, each `pitch` px apart. */
export function rowWindow(
  scrollTop: number,
  viewHeight: number,
  count: number,
  pitch: number,
  overscan = OVERSCAN,
): { first: number; last: number } {
  const first = Math.max(0, Math.min(count, Math.floor(scrollTop / pitch) - overscan));
  const last = Math.max(first, Math.min(count, Math.ceil((scrollTop + viewHeight) / pitch) + overscan));
  return { first, last };
}

/**
 * The scroll position that brings row `index` (`height` px high) into view with the least
 * movement, like `scrollIntoView({ block: "nearest" })`: unchanged when it is in view already.
 */
export function scrollTopFor(index: number, scrollTop: number, viewHeight: number, pitch: number, height: number): number {
  const top = index * pitch;
  if (top < scrollTop) return top;
  if (top + height > scrollTop + viewHeight) return Math.max(0, top + height - viewHeight);
  return scrollTop;
}
