import MarkdownIt from "markdown-it";
import DOMPurify from "dompurify";

// Raw HTML in notes is shown as text, never rendered; DOMPurify is the second line of defence.
const md = new MarkdownIt({ html: false, linkify: true });

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
