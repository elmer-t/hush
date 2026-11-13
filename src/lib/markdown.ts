import MarkdownIt from "markdown-it";
import DOMPurify from "dompurify";
import { highlightDelimiter } from "$lib/highlightMark";
import { parseSubSuper } from "$lib/subSuper";
import { parseWikilink } from "$lib/wikilink";

// The app shows this renderer's output in one place: the tables and images of the Live editor
// (`liveBlocksView.ts`). Until #193 it also fed a read-only preview pane; comments here and in
// the shared parsers (`wikilink.ts`, `highlightMark.ts`, `subSuper.ts`) that say "the preview"
// mean this renderer. Only inline syntax gets rules of its own here: what a table cell can
// hold. Task lists, callouts, code highlighting and diagrams are the Live editor's alone.

// Raw HTML in notes is shown as text, never rendered; DOMPurify is the second line of defence.
const md = new MarkdownIt({ html: false, linkify: true });

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
  // `tokens_meta` can have holes, which `map` would keep.
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
