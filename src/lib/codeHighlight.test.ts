import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
import { classHighlighter, highlightTree } from "@lezer/highlight";
import { codeLanguage, loadGrammar } from "$lib/codeHighlight";
import { liveLanguage } from "$lib/livePreview";

/** The languages the issue names, with code that has a keyword or a string to colour. */
const SAMPLES: Record<string, string> = {
  rust: 'fn main() {\n    let s = "hi"; // note\n}',
  js: 'const a = "x"; // note\nfunction f() { return 1; }',
  python: 'def f(x):\n    return "s"  # note',
  sql: "SELECT name FROM notes WHERE id = 1 -- note",
  json: '{ "a": [1, true, null], "b": "s" }',
  bash: 'if [ -f "$1" ]; then echo "yes"; fi # note',
};

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

describe("code in the editor", () => {
  /** The classes the editor's highlighter gives `doc`, as `text:classes` for each token. */
  function tokens(doc: string) {
    const state = EditorState.create({ doc, extensions: [liveLanguage()] });
    const tree = ensureSyntaxTree(state, doc.length, 5000)!;
    const out: string[] = [];
    highlightTree(tree, classHighlighter, (from, to, classes) => out.push(`${doc.slice(from, to)}:${classes}`));
    return { out, tree, state };
  }

  it("parses a fence in its language once the grammar has loaded", async () => {
    await loadGrammar("js");
    const { out } = tokens('Text\n\n```js\nconst a = "x";\n```\n');
    expect(out).toContain("const:tok-keyword");
    expect(out).toContain('"x":tok-string');
  });

  it("highlights each language the issue names", async () => {
    for (const [lang, code] of Object.entries(SAMPLES)) {
      expect(await loadGrammar(lang), lang).toBe(true);
      const { out } = tokens("```" + lang + "\n" + code + "\n```\n");
      expect(out.some((t) => /:tok-(keyword|string|propertyName)/.test(t)), lang).toBe(true);
    }
  });

  it("leaves an unknown language and a mermaid fence unparsed", () => {
    for (const lang of ["nolang", "mermaid", ""]) {
      const { out } = tokens("```" + lang + '\nconst a = "x";\n```\n');
      expect(out.some((t) => t.startsWith("const:")), lang).toBe(false);
    }
  });
});
