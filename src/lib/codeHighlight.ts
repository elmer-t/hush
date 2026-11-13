import { markdownLanguage } from "@codemirror/lang-markdown";
import { LanguageDescription, languageDataProp, syntaxHighlighting } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import type { Extension } from "@codemirror/state";
import type { Parser } from "@lezer/common";
import { classHighlighter, highlightCode } from "@lezer/highlight";

/**
 * Syntax highlighting in fenced code blocks (#180), for the preview and both editor modes
 * from the same grammars: the code is parsed by the CodeMirror language named on the fence
 * and its tokens get `@lezer/highlight`'s `tok-…` classes, which `app.css` colours. Classes,
 * not `style` attributes, which the markdown sanitizer strips.
 *
 * `@codemirror/language-data` only lists the languages; each grammar is a chunk of the bundle
 * that is loaded the first time a note names it. Nothing is fetched from outside the app.
 */

/** Markdown in a fence stays plain: a note inside a note would be formatted twice. */
const LANGUAGES = languages.filter((l) => l.name !== "Markdown");

/**
 * The language a fence's info string names: its first word, by name or alias, case-insensitive
 * (`js`, `Rust`, `c++`). Null for an unknown language, no language, and `mermaid`.
 */
export function codeLanguage(info: string): LanguageDescription | null {
  const name = info.trim().split(/\s+/)[0];
  return name ? LanguageDescription.matchLanguageName(LANGUAGES, name, false) : null;
}

/**
 * The `tok-…` classes for the code in a fence, in either editor mode. Markdown's own tokens
 * get none: Source styles those itself and Live formats them in place.
 */
export function codeHighlighting(): Extension {
  return syntaxHighlighting({
    style: (tags) => classHighlighter.style(tags),
    scope: (type) => type.prop(languageDataProp) !== markdownLanguage.data,
  });
}

const parsers = new Map<LanguageDescription, Parser>();
const loading = new Set<LanguageDescription>();
const listeners = new Set<() => void>();

/** Calls `listener` whenever a grammar has loaded: what `highlightFence` left plain can now be highlighted. */
export function onGrammar(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function load(language: LanguageDescription) {
  if (loading.has(language)) return;
  loading.add(language);
  language.load().then(
    (support) => {
      parsers.set(language, support.language.parser);
      for (const listener of [...listeners]) listener();
    },
    // A grammar that fails to load leaves its code plain; it is not tried again.
    () => {},
  );
}

/** Loads the grammar for `info`, for tests and checks; resolves to whether there is one. */
export async function loadGrammar(info: string): Promise<boolean> {
  const language = codeLanguage(info);
  if (!language) return false;
  if (!parsers.has(language)) parsers.set(language, (await language.load()).language.parser);
  return true;
}

/** Longer code stays plain in the preview: it is parsed in one go, on the keystroke that changes it. */
export const HIGHLIGHT_LIMIT = 50_000;

/**
 * The preview renders on every edit; code that did not change is not parsed again. The cache
 * is bounded by the characters it holds (code and HTML), not by entries, so a long note of
 * many small blocks fits. It holds note text: in memory only, like the preview itself.
 */
const CACHE_CHARS = 4_000_000;
const cache = new Map<string, string>();
let cached = 0;

const ESCAPES: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" };
const escapeHtml = (text: string) => text.replace(/[&<>"]/g, (c) => ESCAPES[c]);

/**
 * `code` as HTML with a `span` per token, for markdown-it's `highlight` option. An empty
 * string (markdown-it then escapes the code itself) for a language that is unknown or not
 * loaded yet; a language seen for the first time starts loading, see `onGrammar`.
 */
export function highlightFence(code: string, info: string): string {
  const language = codeLanguage(info);
  if (!language || code.length > HIGHLIGHT_LIMIT) return "";
  const parser = parsers.get(language);
  if (!parser) {
    load(language);
    return "";
  }
  const key = `${language.name}\n${code}`;
  const hit = cache.get(key);
  if (hit !== undefined) {
    // Most recently used last.
    cache.delete(key);
    cache.set(key, hit);
    return hit;
  }
  let html = "";
  highlightCode(
    code,
    parser.parse(code),
    classHighlighter,
    (text, classes) => (html += classes ? `<span class="${classes}">${escapeHtml(text)}</span>` : escapeHtml(text)),
    () => (html += "\n"),
  );
  cache.set(key, html);
  cached += key.length + html.length;
  for (const [oldest, value] of cache) {
    if (cached <= CACHE_CHARS) break;
    cache.delete(oldest);
    cached -= oldest.length + value.length;
  }
  return html;
}
