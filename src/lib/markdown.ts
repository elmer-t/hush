import MarkdownIt from "markdown-it";
import DOMPurify from "dompurify";

// Raw HTML in notes is shown as text, never rendered; DOMPurify is the second line of defence.
const md = new MarkdownIt({ html: false, linkify: true });

/**
 * `[[target#heading|alias]]` as `<a class="wikilink" data-target="target">`, the way
 * `markdown::parse` reads links: not in code, not `![[embeds]]`, and an empty target
 * (`[[#heading]]`) stays text. The preview resolves the target when it is clicked.
 */
md.inline.ruler.before("link", "wikilink", (state, silent) => {
  const { src, pos } = state;
  if (src.charCodeAt(pos) !== 0x5b || src.charCodeAt(pos + 1) !== 0x5b || src.charCodeAt(pos - 1) === 0x21) return false;
  const end = src.indexOf("]]", pos + 2);
  if (end < 0) return false;
  const inner = src.slice(pos + 2, end);
  if (/[[\]\n]/.test(inner)) return false;
  const bar = inner.indexOf("|");
  const targetPart = bar < 0 ? inner : inner.slice(0, bar);
  const alias = bar < 0 ? "" : inner.slice(bar + 1).trim();
  const hash = targetPart.indexOf("#");
  const target = (hash < 0 ? targetPart : targetPart.slice(0, hash)).trim();
  if (!target) return false;
  if (!silent) {
    const open = state.push("wikilink_open", "a", 1);
    open.attrSet("href", "#");
    open.attrSet("class", "wikilink");
    open.attrSet("data-target", target);
    const text = state.push("text", "", 0);
    text.content = alias || targetPart.trim();
    state.push("wikilink_close", "a", -1);
  }
  state.pos = end + 2;
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

/** Matches the frontmatter block the way `markdown::split_frontmatter` does in Rust. */
const FRONTMATTER = /^---\r?\n(?:[\s\S]*?\n)?(?:---|\.\.\.)\r?(?:\n|$)/;

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
