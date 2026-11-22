// The arithmetic of the editor's own scrollbar (#243), apart from the DOM so it can be tested.
//
// Why the editor has one: CodeMirror draws only the lines near the view and estimates the
// height of the rest, too low for wrapped lines, so a long note grows while it is scrolled
// for the first time. A native handle sits at scrollTop / (scrollHeight - clientHeight), so
// during a drag it falls behind the mouse as the note grows under it. Here a drag keeps the
// pointer's place on the track as a fraction and maps it onto the scroll range as it is at
// that moment: the mouse at the bottom is the end of the note, whatever its height turns
// out to be.

/** What a scrolling element reports. */
export type ScrollBox = { scrollTop: number; scrollHeight: number; clientHeight: number };

/** A handle shorter than this is hard to grab. */
export const MIN_HANDLE_PX = 32;
/** The share of a page a click on the track moves, so a strip of the old page stays in view. */
const PAGE_SHARE = 0.875;

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

/** How far the element can scroll; 0 when its content fits. */
const range = (box: Omit<ScrollBox, "scrollTop">) => Math.max(0, box.scrollHeight - box.clientHeight);

/**
 * The handle's size and its offset from the top of a track `track` px long, or null when
 * there is nothing to scroll (or no room for a handle): then no scrollbar is shown.
 */
/** The handle on its track, in px: its length and how far down it sits. */
export type Handle = { size: number; offset: number };

export function handleGeometry(box: ScrollBox, track: number): Handle | null {
  const scrollable = range(box);
  if (scrollable < 1 || track <= MIN_HANDLE_PX) return null;
  const size = clamp((box.clientHeight / box.scrollHeight) * track, MIN_HANDLE_PX, track);
  const offset = clamp(box.scrollTop / scrollable, 0, 1) * (track - size);
  return { size, offset };
}

/**
 * Where a dragged handle is, from 0 (top) to 1 (bottom): the pointer at `pointerY`
 * holds the handle `grab` px below the handle's top.
 */
export function dragFraction(pointerY: number, grab: number, track: { top: number; height: number }, size: number): number {
  const free = track.height - size;
  if (free <= 0) return 0;
  return clamp((pointerY - grab - track.top) / free, 0, 1);
}

/** The scroll position for `fraction` of the scroll range as it is now. */
export function scrollTopAt(fraction: number, box: Omit<ScrollBox, "scrollTop">): number {
  return fraction * range(box);
}

/** The scroll position one page down (`direction` 1) or up (-1): a click on the track. */
export function pageScrollTop(box: ScrollBox, direction: 1 | -1): number {
  return clamp(box.scrollTop + direction * box.clientHeight * PAGE_SHARE, 0, range(box));
}
