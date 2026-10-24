# Search notes

A user types in the sidebar search box and gets ranked full-text hits with a snippet, can open a hit in the editor, narrow by filters, and clear the search to get the tree back.

## Sub-features

- `search-open` focuses search from the box or `Ctrl K`.
- `search-match` lists hits, best match first, with a count.
- `search-empty` shows `No matching notes.` for a miss.
- `search-open-hit` opens a hit in the editor.
- `search-clear` restores the tree.
- `search-filters` narrows by tag, date or attribute (**Filters**).

## How to get to it (user POV)

- Click **Search notes** in the sidebar, or press `Ctrl K`.
- **Filters** button under the search box.

## Driving it with hush.mjs

Preconditions:

- Baseline, plus the fixture folder imported ([import.md](./import.md)), which gives three notes mentioning `ferry`.

- **Keyboard entry.** Run `node $h press Control+K`. `node $h eval "document.activeElement.getAttribute('aria-label')"` prints `"Search notes"`.
- **Match.** Run `node $h type "input[aria-label='Search notes']" "ferry"` and `node $h wait ".sidebar .summary"`. `node $h text ".sidebar .summary"` prints `3 results · best match first`, and `node $h text ".results"` lists `ferry`, `thread-0002` and `thread-0001` with snippets.
- **Open hit.** Run `node $h click "text=thread-0001"`. `node $h eval "document.querySelector('input[aria-label=Title]').value"` prints `"thread-0001"`.
- **Miss.** Run `node $h click "input[aria-label='Search notes']"`, `node $h press Control+A` and `node $h type "input[aria-label='Search notes']" "volcano"`. `node $h text ".results"` contains `No matching notes.`
- **Clear.** Run `node $h click "button[aria-label='Clear search']"`. `node $h wait "[role=tree]"` succeeds.
- **Proof.** Run `node $h shot search-results` after **Match**.

## Gotchas

- Results are debounced. Wait for `.sidebar .summary` or the empty text, not a sleep.
- Note titles can repeat across folders. When clicking a hit by text, check the opened note's folder field (`input[placeholder="No folder"]`).
- Trashed notes never appear in search.
