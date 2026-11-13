import { markdownLanguage } from "@codemirror/lang-markdown";
import { LanguageDescription, languageDataProp, syntaxHighlighting } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import type { Extension } from "@codemirror/state";
import { classHighlighter } from "@lezer/highlight";

/**
 * Syntax highlighting in fenced code blocks (#180), for both editor modes: the code is parsed
 * by the CodeMirror language named on the fence and its tokens get `@lezer/highlight`'s `tok-…`
 * classes, which `app.css` colours.
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

/** Loads the grammar for `info`, for tests and checks; resolves to whether there is one. */
export async function loadGrammar(info: string): Promise<boolean> {
  const language = codeLanguage(info);
  if (!language) return false;
  await language.load();
  return true;
}
