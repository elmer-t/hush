import MarkdownIt from "markdown-it";
import DOMPurify from "dompurify";
import { highlightFence } from "$lib/codeHighlight";
import { highlightDelimiter } from "$lib/highlightMark";
import { parseSubSuper } from "$lib/subSuper";
import { parseWikilink } from "$lib/wikilink";

// Raw HTML in notes is shown as text, never rendered; DOMPurify is the second line of defence.
// Code in a fence with a known language gets a `span` per token (`codeHighlight.ts`, #180).
const md = new MarkdownIt({ html: false, linkify: true, highlight: highlightFence });

/**
 * `[[target#heading|alias]]` as `<a class="wikilink" data-target="target">`, the way
 * `markdown::parse` reads links: not in code, not `![[embeds]]`, and an empty target
 * (`[[#heading]]`) stays text (`parseWikilink`, shared with the Live editor). The preview
 * resolves the target when it is clicked.
 */
md.inline.ruler.before("link", "wikilink", (state, silent) => {
  const link = parseWikilink(state.src, state.pos);
  if (!link) return false;
  if (!silent) {
    const open = state.push("wikilink_open", "a", 1);
    open.attrSet("href", "#");
    open.attrSet("class", "wikilink");
    open.attrSet("data-target", link.target);
    const text = state.push("text", "", 0);
    text.content = link.text;
    state.push("wikilink_close", "a", -1);
  }
  state.pos = link.to;
  return true;
});

/**
 * `==text==` as `<mark>` (#181). The delimiters are found by `highlightDelimiter`, shared
 * with the Live editor, and paired by markdown-it like those of `~~strikethrough~~`, so other
 * inline formatting works inside and an unclosed `==` stays text.
 */
md.inline.ruler.after("emphasis", "mark", (state, silent) => {
  if (silent || state.pos + 1 >= state.posMax) return false;
  const delimiter = highlightDelimiter(state.src, state.pos);
  if (!delimiter) return false;
  const token = state.push("text", "", 0);
  token.content = "==";
  state.delimiters.push({ marker: 0x3d, length: 0, token: state.tokens.length - 1, end: -1, open: delimiter.open, close: delimiter.close });
  state.pos += 2;
  return true;
});

md.inline.ruler2.after("emphasis", "mark", (state) => {
  // `tokens_meta` has holes (a task's box has no entry), which `map` would keep.
  const lists = [state.delimiters];
  for (const meta of state.tokens_meta) if (meta?.delimiters) lists.push(meta.delimiters);
  for (const delimiters of lists) {
    for (const open of delimiters) {
      if (open.marker !== 0x3d || open.end === -1) continue;
      for (const [index, type, nesting] of [[open.token, "mark_open", 1], [delimiters[open.end].token, "mark_close", -1]] as const) {
        const token = state.tokens[index];
        token.type = type;
        token.tag = "mark";
        token.nesting = nesting;
        token.markup = "==";
        token.content = "";
      }
    }
  }
  return true;
});

/**
 * `~text~` as `<sub>` and `^text^` as `<sup>` (#182), as `parseSubSuper` reads them, shared
 * with the Live editor. The text between the markers is shown as it is.
 */
md.inline.ruler.after("emphasis", "sub_sup", (state, silent) => {
  const found = parseSubSuper(state.src, state.pos, state.posMax);
  if (!found) return false;
  if (!silent) {
    state.push(`${found.tag}_open`, found.tag, 1);
    state.push("text", "", 0).content = found.text;
    state.push(`${found.tag}_close`, found.tag, -1);
  }
  state.pos = found.to;
  return true;
});

/**
 * A ```mermaid fence becomes a placeholder holding its source as escaped text, which the
 * markdown sanitizer keeps; `mermaid.ts` swaps in the diagram after the HTML is in the DOM,
 * since its SVG needs `<style>` and `style`, which the sanitizer here removes.
 */
const fence = md.renderer.rules.fence!;
md.renderer.rules.fence = (tokens, idx, options, env, self) => {
  const token = tokens[idx];
  if (token.info.trim().split(/\s+/)[0].toLowerCase() !== "mermaid") return fence(tokens, idx, options, env, self);
  return `<div class="mermaid-block"><pre class="mermaid-source"><code>${md.utils.escapeHtml(token.content)}</code></pre></div>\n`;
};

/** A task marker at the start of a list item's text, as `@lezer/markdown`'s `TaskList` reads it. */
const TASK = /^\[([ xX])\](?:[ \t]+|$)/;

/**
 * `- [ ] todo` and `- [x] done` as a checkbox, for the same items the Live editor shows one
 * (`liveBlocks.ts`, `listItem`): the marker opens the item's first paragraph, on the line of
 * the list marker, and is followed by a space or tab. The box is a `span` drawn by
 * `Preview.svelte`, since the sanitizer forbids `input`; it is read-only.
 *
 * Runs before the inline rule, on the paragraph's source: the marker is cut off and the box
 * put in front of the tokens the inline rule adds, so `[x]` is never read as a reference link.
 */
md.core.ruler.before("inline", "task_list", (state) => {
  const tokens = state.tokens;
  let lines: string[] | undefined;
  for (let i = 2; i < tokens.length; i++) {
    const inline = tokens[i];
    const item = tokens[i - 2];
    if (inline.type !== "inline" || tokens[i - 1].type !== "paragraph_open" || item.type !== "list_item_open") continue;
    const match = TASK.exec(inline.content);
    const line = inline.map?.[0];
    if (!match || line === undefined || item.map?.[0] !== line) continue;
    // markdown-it trims the paragraph, so a marker with nothing after it is checked in the source.
    if (match[0].length === 3 && !/\][ \t]+$/.test((lines ??= state.src.split("\n"))[line] ?? "")) continue;
    inline.content = inline.content.slice(match[0].length);
    const box = new state.Token("task_box", "span", 0);
    box.meta = { checked: match[1] !== " " };
    (inline.children ??= []).push(box);
    item.attrJoin("class", "task-item");
  }
});

md.renderer.rules.task_box = (tokens, idx) => {
  const checked = tokens[idx].meta?.checked === true;
  return `<span class="task-box${checked ? " task-done" : ""}" role="checkbox" aria-checked="${checked}" aria-disabled="true" aria-label="${checked ? "Done" : "Not done"}"></span>`;
};

/** Matches the frontmatter block the way `markdown::split_frontmatter` does in Rust. */
export const FRONTMATTER = /^---\r?\n(?:[\s\S]*?\n)?(?:---|\.\.\.)\r?(?:\n|$)/;

export function stripFrontmatter(body: string): string {
  return body.replace(FRONTMATTER, "");
}

export function renderMarkdown(body: string): string {
  return DOMPurify.sanitize(md.render(stripFrontmatter(body)), {
    FORBID_TAGS: ["style", "form", "input", "button"],
    FORBID_ATTR: ["style"],
  });
}

/** `/a\b//c/` → `a/b/c`, like `markdown::normalize_folder`. */
export function normalizeFolder(folder: string): string {
  return folder
    .split(/[\\/]/)
    .map((s) => s.trim())
    .filter(Boolean)
    .join("/");
}

/** Whether `folder` is `filter` or inside it; `null` matches everything. */
export function inFolder(folder: string, filter: string | null): boolean {
  if (filter === null || filter === "") return true;
  const f = folder.toLowerCase();
  const p = filter.toLowerCase();
  return f === p || f.startsWith(`${p}/`);
}
