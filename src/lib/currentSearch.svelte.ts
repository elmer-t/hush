// The sidebar's search (text and filters) while one is active, so Discover tags can
// group its results. Module state, so Settings sees it while the sidebar is hidden;
// lock reloads the page, which clears it.
import type { SearchQuery } from "$lib/api";

export const currentSearch = $state<{ query: SearchQuery | null }>({ query: null });
