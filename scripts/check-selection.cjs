// Browser check for selected text (#245): the editor's selection has the app's own colour
// (`--text-selection`) with and without focus, in Source and Live and both themes, not
// CodeMirror's lavender; a field outside the editor selects in the same colour; the text on
// the selection reaches 4.5:1; a find match in the selection keeps its own colours.
// Needs `npm run dev` (Vite on :1420, or HUSH_DEV_URL) and Playwright:
//   NODE_PATH=$(npm root -g) node scripts/check-selection.cjs [screenshot dir]
// Mounts the editor through src/lib/checks/selection-harness.svelte.ts, and exits 1 when a check fails.
const { chromium } = require("playwright");
const URL = process.env.HUSH_DEV_URL || process.env.DEV_URL || "http://127.0.0.1:1420/";
const SHOTS = process.argv[2];
const DOC = [
  "---", "title: Selection", "---", "",
  "# A heading", "",
  "Plain text with a [link](https://example.com), `code` and a needle in it.", "",
  "> [!tip] A tip", "> Its text.", "",
  "> [!danger] A danger", "> Its text.", "",
  "```js", "const answer = 42; // a comment", "function name() { return \"string\"; }", "```", "",
  "- a list item", "",
].join("\n");

const lum = ([r, g, b]) => {
  const [x, y, z] = [r, g, b].map((c) => ((c /= 255) <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * x + 0.7152 * y + 0.0722 * z;
};
const contrast = (a, b) => { const [hi, lo] = [lum(a), lum(b)].sort((p, q) => q - p); return (hi + 0.05) / (lo + 0.05); };
const rgb = (s) => (s.match(/[\d.]+/g) || []).slice(0, 3).map(Number);

/** Runs in the page: the colours as the browser resolved them. */
function state() {
  const probe = document.createElement("div");
  probe.style.backgroundColor = "var(--text-selection)";
  document.body.append(probe);
  const token = getComputedStyle(probe).backgroundColor;
  probe.remove();
  const editor = document.querySelector(".cm-editor");
  const backgrounds = [...editor.querySelectorAll(".cm-selectionBackground")].map((el) => getComputedStyle(el).backgroundColor);
  // Every piece of text in the editor, by the colour it is drawn in.
  const colours = new Set();
  const walker = document.createTreeWalker(editor.querySelector(".cm-content"), NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (!n.textContent.trim() || n.parentElement.closest(".cm-searchMatch")) continue;
    colours.add(getComputedStyle(n.parentElement).color);
  }
  const match = editor.querySelector(".cm-searchMatch");
  return {
    token,
    backgrounds: [...new Set(backgrounds)],
    pieces: backgrounds.length,
    focused: editor.classList.contains("cm-focused"),
    colours: [...colours],
    native: getComputedStyle(editor.querySelector(".cm-line"), "::selection").backgroundColor,
    field: getComputedStyle(document.getElementById("field"), "::selection").backgroundColor,
    match: match ? { background: getComputedStyle(match).backgroundColor, color: getComputedStyle(match).color } : null,
  };
}

async function main() {
  const browser = await chromium.launch();
  const fails = [];
  const check = (name, ok, detail = "") => { console.log(`${ok ? "ok  " : "FAIL"} ${name}${ok || !detail ? "" : `  (${detail})`}`); if (!ok) fails.push(name); };
  for (const scheme of ["light", "dark"]) {
    for (const mode of ["source", "live"]) {
      const tag = `${scheme} ${mode}`;
      const page = await (await browser.newContext({ colorScheme: scheme, viewport: { width: 900, height: 620 }, deviceScaleFactor: SHOTS ? 2 : 1 })).newPage();
      const errors = [];
      page.on("pageerror", (e) => errors.push(String(e)));
      await page.addInitScript(() => {
        window.__TAURI_INTERNALS__ = { transformCallback: () => 0, invoke: async () => { throw new Error("mock"); } };
      });
      await page.goto(URL);
      await page.waitForTimeout(1500);
      await page.evaluate(() => import("/src/lib/checks/selection-harness.svelte.ts"));
      await page.evaluate(([doc, m]) => (window.h = window.__mountEditor(doc, m)), [DOC, mode]);
      await page.waitForSelector(".cm-content");
      // From here on: the app's own start fails against the mock, which is not what is checked.
      errors.length = 0;
      await page.waitForTimeout(400);

      await page.click(".cm-content");
      await page.keyboard.press("Control+a");
      await page.waitForTimeout(150);
      let s = await page.evaluate(state);
      const token = rgb(s.token);
      check(`${tag}: focused, the selection is drawn (${s.pieces} pieces)`, s.focused && s.pieces > 0);
      check(`${tag}: focused, the selection has the app's colour ${s.token}`, s.backgrounds.length === 1 && s.backgrounds[0] === s.token, s.backgrounds.join(" / "));
      check(`${tag}: it is not CodeMirror's lavender`, !s.backgrounds.includes("rgb(215, 212, 240)"));
      const worst = s.colours.map((c) => ({ c, ratio: contrast(rgb(c), token) })).sort((a, b) => a.ratio - b.ratio)[0];
      check(`${tag}: all ${s.colours.length} text colours shown reach 4.5:1 on it (lowest ${worst.ratio.toFixed(2)} for ${worst.c})`, worst.ratio >= 4.5);
      if (SHOTS) await page.screenshot({ path: `${SHOTS}/selection-${scheme}-${mode}.png` });

      // A find match inside the selection keeps its own background and text.
      await page.evaluate(() => window.h.find("needle"));
      await page.waitForTimeout(150);
      await page.click(".cm-content");
      await page.keyboard.press("Control+a");
      await page.waitForTimeout(150);
      s = await page.evaluate(state);
      const pair = s.match && contrast(rgb(s.match.background), rgb(s.match.color));
      check(`${tag}: a find match in the selection keeps its own colours (${pair && pair.toFixed(1)}:1)`, !!s.match && s.match.background !== s.token && pair >= 4.5, JSON.stringify(s.match));
      await page.evaluate(() => window.h.find(""));

      // Without focus: the same colour.
      await page.click(".cm-content");
      await page.keyboard.press("Control+a");
      await page.focus("#field");
      await page.waitForTimeout(150);
      s = await page.evaluate(state);
      check(`${tag}: not focused, the selection keeps the app's colour`, !s.focused && s.pieces > 0 && s.backgrounds.length === 1 && s.backgrounds[0] === s.token, s.backgrounds.join(" / "));

      // A field outside the editor.
      check(`${tag}: a field outside the editor selects in the same colour`, s.field === s.token, s.field);
      check(`${tag}: no page errors`, errors.length === 0, errors.join(" | "));
      await page.close();
    }
  }
  await browser.close();
  console.log(fails.length ? `\n${fails.length} failed` : "\nall passed");
  process.exit(fails.length ? 1 : 0);
}

main().catch((e) => { console.error(e); process.exit(1); });
