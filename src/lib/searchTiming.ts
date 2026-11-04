// When the sidebar's search-as-you-type runs. Typing waits for a pause so a word starts
// one query, not one per letter; every query on a large vault is a prefix search plus a
// count probe. Anything else (a filter, the folder, clearing the box, Enter) is a single
// deliberate action and searches at once.

/** How long typing must pause before the search runs. */
export const TYPING_DELAY_MS = 300;

/** What a search depends on: the typed text, and everything else as one comparable key. */
export type SearchInputs = { text: string; scope: string };

/** The wait before searching for `next`, coming from `prev` (null on the first run). */
export function searchDelay(prev: SearchInputs | null, next: SearchInputs): number {
  if (prev === null || prev.scope !== next.scope || next.text.trim() === "") return 0;
  return prev.text === next.text ? 0 : TYPING_DELAY_MS;
}

/**
 * Runs `run` after each change, waiting `searchDelay`. A newer change replaces a waiting
 * one; `now` runs a waiting search at once (Enter), and `cancel` drops it.
 */
export function searchScheduler(run: () => void) {
  let last: SearchInputs | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let waiting = false;
  const fire = () => {
    clearTimeout(timer);
    waiting = false;
    run();
  };
  return {
    change(next: SearchInputs) {
      const delay = searchDelay(last, next);
      last = next;
      clearTimeout(timer);
      waiting = true;
      timer = setTimeout(fire, delay);
    },
    now() {
      if (waiting) fire();
    },
    cancel() {
      clearTimeout(timer);
      waiting = false;
    },
  };
}
