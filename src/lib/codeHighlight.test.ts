import { describe, expect, it, vi } from "vitest";
import { EditorState } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
import { classHighlighter, highlightTree } from "@lezer/highlight";
import { codeLanguage, HIGHLIGHT_LIMIT, highlightFence, loadGrammar, onGrammar } from "$lib/codeHighlight";
import { liveLanguage } from "$lib/livePreview";

// The markdown sanitizer needs a DOM, which the tests don't have; it isn't what's tested here.
vi.mock("dompurify", () => ({ default: { sanitize: (html: string) => html } }));
const { renderMarkdown } = await import("$lib/markdown");

/** The languages the issue names, with code that has a keyword or a string to colour. */
const SAMPLES: Record<string, string> = {
  rust: 'fn main() {\n    let s = "hi"; // note\n}',
  js: 'const a = "x"; // note\nfunction f() { return 1; }',
  python: 'def f(x):\n    return "s"  # note',
  sql: "SELECT name FROM notes WHERE id = 1 -- note",
  json: '{ "a": [1, true, null], "b": "s" }',
  bash: 'if [ -f "$1" ]; then echo "yes"; fi # note',
};

const text = (html: string) => html.replace(/<[^>]*>/g, "").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&amp;/g, "&");

describe("the language of a fence", () => {
  it("is the first word of the info string, by name or alias, in any case", () => {
    expect(codeLanguage("rust")?.name).toBe("Rust");
    expect(codeLanguage("js")?.name).toBe("JavaScript");
    expect(codeLanguage("JS title=x")?.name).toBe("JavaScript");
    expect(codeLanguage("  Python  ")?.name).toBe("Python");
    expect(codeLanguage("bash")?.name).toBe("Shell");
    expect(codeLanguage("c++")?.name).toBe("C++");
  });

  it("is none for an unknown or missing language, mermaid and markdown", () => {
    for (const info of ["", "  ", "nolang", "mermaid", "markdown", "not js", "javascripty"]) expect(codeLanguage(info), info).toBeNull();
  });
});

describe("code in the preview", () => {
  it("is plain until the grammar has loaded, then highlighted", async () => {
    const src = "```toml\nkey = \"value\"\n```";
    let loaded = 0;
    const stop = onGrammar(() => loaded++);
    expect(renderMarkdown(src)).toBe('<pre><code class="language-toml">key = &quot;value&quot;\n</code></pre>\n');
    await vi.waitFor(() => expect(loaded).toBe(1));
    stop();
    const html = renderMarkdown(src);
    expect(html).toContain('<span class="tok-string">&quot;value&quot;</span>');
    expect(html.startsWith('<pre><code class="language-toml">')).toBe(true);
  });

  it("highlights each language the issue names, with classes and the code unchanged", async () => {
    for (const [lang, code] of Object.entries(SAMPLES)) {
      expect(await loadGrammar(lang), lang).toBe(true);
      const html = highlightFence(code, lang);
      expect(html, lang).toMatch(/<span class="tok-(keyword|string|propertyName)[^"]*">/);
      expect(html, lang).not.toMatch(/style=/);
      expect(text(html), lang).toBe(code);
    }
  });

  it("escapes the code", async () => {
    await loadGrammar("js");
    const html = highlightFence('a < b && "<script>" > c', "js");
    expect(html).not.toContain("<script>");
    expect(text(html)).toBe('a < b && "<script>" > c');
  });

  it("leaves an unknown language, no language and very long code plain", async () => {
    expect(renderMarkdown("```nolang\na < b\n```")).toBe('<pre><code class="language-nolang">a &lt; b\n</code></pre>\n');
    expect(renderMarkdown("```\nlet a\n```")).toBe("<pre><code>let a\n</code></pre>\n");
    expect(renderMarkdown("    let a")).toBe("<pre><code>let a\n</code></pre>\n");
    await loadGrammar("js");
    expect(highlightFence("let a;\n".repeat(HIGHLIGHT_LIMIT / 7 + 1), "js")).toBe("");
  });

  it("keeps a mermaid fence a diagram placeholder", () => {
    expect(renderMarkdown("```mermaid\nflowchart TD\n```")).toContain('<div class="mermaid-block">');
  });

  it("gives the same result from the cache", async () => {
    await loadGrammar("rust");
    const first = highlightFence(SAMPLES.rust, "rust");
    expect(highlightFence(SAMPLES.rust, "rust")).toBe(first);
    // Enough other code to push it out of the cache: it is parsed again, to the same result.
    const filler = "let a = 1;\n".repeat(4000);
    for (let i = 0; i < 40; i++) highlightFence(`${filler}let b = ${i};`, "rust");
    expect(highlightFence(SAMPLES.rust, "rust")).toBe(first);
  });
});

describe("code in the editor", () => {
  /** The classes the editor's highlighter gives `doc`, as `text:classes` for each token. */
  function tokens(doc: string) {
    const state = EditorState.create({ doc, extensions: [liveLanguage()] });
    const tree = ensureSyntaxTree(state, doc.length, 5000)!;
    const out: string[] = [];
    highlightTree(tree, classHighlighter, (from, to, classes) => out.push(`${doc.slice(from, to)}:${classes}`));
    return { out, tree, state };
  }

  it("parses a fence in its language once the grammar has loaded, with the preview's classes", async () => {
    await loadGrammar("js");
    const { out } = tokens('Text\n\n```js\nconst a = "x";\n```\n');
    expect(out).toContain("const:tok-keyword");
    expect(out).toContain('"x":tok-string');
    expect(highlightFence('const a = "x";\n', "js")).toContain('<span class="tok-keyword">const</span>');
  });

  it("leaves an unknown language and a mermaid fence unparsed", () => {
    for (const lang of ["nolang", "mermaid", ""]) {
      const { out } = tokens("```" + lang + '\nconst a = "x";\n```\n');
      expect(out.some((t) => t.startsWith("const:")), lang).toBe(false);
    }
  });
});
