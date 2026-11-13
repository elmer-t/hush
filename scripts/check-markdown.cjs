// Browser check for Markdown features that the preview and the Live editor both render.
// Needs `npm run dev` (Vite on :1420) and Playwright: `node scripts/check-markdown.cjs
// [screenshot dir]`. It mounts both views on the same note through
// src/lib/checks/markdown-harness.svelte.ts, with the real sanitizer, in the light and the dark
// theme, and exits 1 when a check fails.
const { chromium } = require("playwright");
const URL = "http://127.0.0.1:1420/";
const SHOTS = process.argv[2];

const TASKS = [
  "- [ ] todo",
  "- [x] done with **bold**",
  "- plain item",
  "  - [X] nested done",
  "  - [ ] nested todo",
  "",
  "1. [x] first",
  "2. [ ] second",
  "3. plain",
  "",
  "- a [ ] in the middle",
  "- `[ ] in code`",
  "",
  "```",
  "- [ ] in a code block",
  "```",
].join("\n");

const fenced = (lang, code) => "```" + lang + "\n" + code + "\n```\n";
const CODE = [
  fenced("rust", 'fn main() {\n    let s = "hi"; // note\n}'),
  fenced("js", 'const a = "x"; // note'),
  fenced("python", 'def f(x):\n    return "s"  # note'),
  fenced("sql", "SELECT name FROM notes WHERE id = 1 -- note"),
  fenced("json", '{ "a": [1, true, null] }'),
  fenced("bash", 'if [ -f "$1" ]; then echo "yes"; fi # note'),
  fenced("nolang", "const plain = 1;"),
  fenced("", "const bare = 1;"),
  fenced("mermaid", "flowchart TD\n  A --> B"),
].join("\n");

let failed = 0;
function check(name, ok, detail = "") {
  if (!ok) failed++;
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${ok || !detail ? "" : `: ${detail}`}`);
}

async function mounted(browser, scheme, body) {
  const ctx = await browser.newContext({ colorScheme: scheme, viewport: { width: 1400, height: 900 } });
  const page = await ctx.newPage();
  const errors = [];
  const requests = [];
  page.on("request", (r) => requests.push(r.url()));
  // The app's own page fails its backend calls here ("mock"); only other errors count.
  const note = (text) => text !== "mock" && errors.push(text);
  page.on("pageerror", (e) => note(e.message));
  page.on("console", (m) => m.type() === "error" && note(m.text()));
  await page.addInitScript(() => {
    window.__TAURI_INTERNALS__ = { invoke: async () => { throw new Error("mock"); }, transformCallback: () => 0 };
  });
  await page.goto(URL);
  await page.waitForTimeout(800);
  await page.evaluate(() => import("/src/lib/checks/markdown-harness.svelte.ts"));
  errors.length = 0;
  await page.evaluate((b) => window.__mountMarkdown(b), body);
  await page.waitForSelector("#live .cm-content");
  await page.waitForTimeout(300);
  return { ctx, page, errors, requests };
}

async function tasks(browser, scheme) {
  const { ctx, page, errors } = await mounted(browser, scheme, TASKS);
  const preview = await page.evaluate(() =>
    [...document.querySelectorAll("#preview .task-box")].map((b) => ({
      checked: b.getAttribute("aria-checked"),
      role: b.getAttribute("role"),
      label: b.getAttribute("aria-label"),
      listStyle: getComputedStyle(b.closest("li")).listStyleType,
      width: b.getBoundingClientRect().width,
      background: getComputedStyle(b).backgroundColor,
      text: b.closest("li").firstChild.textContent + (b.parentElement.textContent ?? "").split("\n")[0],
    })),
  );
  const live = await page.evaluate(() => [...document.querySelectorAll("#live .cm-live-task input")].map((b) => String(b.checked)));
  const want = ["false", "true", "true", "false", "true", "false"];
  check(`${scheme}: preview checkboxes`, preview.map((b) => b.checked).join() === want.join(), JSON.stringify(preview));
  check(`${scheme}: Live shows the same checkboxes`, live.join() === want.join(), live.join());
  check(`${scheme}: boxes survive the sanitizer with role and label`, preview.every((b) => b.role === "checkbox" && /^(Done|Not done)$/.test(b.label)));
  // A box replaces the bullet; an ordered item keeps its number, as in Live.
  check(`${scheme}: no bullet before a box, numbers stay`, preview.map((b) => b.listStyle).join() === "none,none,none,none,decimal,decimal", preview.map((b) => b.listStyle).join());
  check(`${scheme}: boxes are drawn`, preview.every((b) => b.width >= 12), preview.map((b) => b.width).join());
  check(`${scheme}: done and not done look different`, preview[0].background !== preview[1].background, `${preview[0].background} ${preview[1].background}`);
  check(`${scheme}: no input element in the preview`, (await page.locator("#preview input").count()) === 0);
  check(`${scheme}: markers are not shown as text`, !(await page.locator("#preview li.task-item").allTextContents()).some((t) => /\[[ xX]\]/.test(t)));
  check(`${scheme}: text that is not a task keeps its brackets`, (await page.locator("#preview").textContent()).includes("a [ ] in the middle"));
  check(`${scheme}: plain items keep their bullet`, (await page.evaluate(() => getComputedStyle([...document.querySelectorAll("#preview li")].find((l) => l.textContent.startsWith("plain item"))).listStyleType)) === "disc");
  check(`${scheme}: no errors`, errors.length === 0, errors.join("; "));
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/tasks-${scheme}.png` });
  await ctx.close();
}

/** The colour of the first token with `cls` whose text is `text`, in the view `root`. */
const tokenColour = (page, root, cls, text) =>
  page.evaluate(
    ([root, cls, text]) => {
      const el = [...document.querySelectorAll(`${root} .${cls}`)].find((e) => e.textContent === text);
      return el ? getComputedStyle(el).color : null;
    },
    [root, cls, text],
  );

async function code(browser, scheme, colours) {
  const { ctx, page, errors, requests } = await mounted(browser, scheme, CODE);
  // Grammars load on demand: wait until the last language in the note is highlighted in each view.
  const views = { preview: "#preview pre", live: "#live .cm-live-codeblock", source: "#source .cm-content" };
  for (const [view, root] of Object.entries(views)) {
    await page
      .waitForFunction((root) => [...document.querySelectorAll(`${root} .tok-keyword`)].some((e) => e.textContent === "fi"), root, { timeout: 15000 })
      .catch(() => {});
    const tokens = [
      ["tok-keyword", "fn"],
      ["tok-keyword", "const"],
      ["tok-keyword", "def"],
      ["tok-keyword", "SELECT"],
      ["tok-string", '"hi"'],
      ["tok-comment", "// note"],
      ["tok-number", "1"],
      ["tok-keyword", "fi"],
    ];
    const got = [];
    for (const [cls, text] of tokens) got.push(await tokenColour(page, root, cls, text));
    check(`${scheme} ${view}: rust, js, python, sql, json and bash are highlighted`, got.every(Boolean), JSON.stringify(got));
    (colours[view] ??= {})[scheme] = got.join("|");
  }
  check(`${scheme}: the three views use the same colours`, colours.preview[scheme] === colours.live[scheme] && colours.live[scheme] === colours.source[scheme], JSON.stringify(colours));
  const body = await page.evaluate(() => getComputedStyle(document.querySelector("#preview pre code")).color);
  check(`${scheme}: keywords and strings differ from plain code`, new Set([body, ...colours.preview[scheme].split("|").slice(0, 5)]).size >= 3);
  const plain = await page.evaluate(() =>
    [...document.querySelectorAll("#preview pre code")].filter((c) => /plain|bare/.test(c.textContent)).map((c) => c.querySelectorAll("span").length),
  );
  check(`${scheme}: an unknown language and a fence without one stay plain in the preview`, plain.join() === "0,0", plain.join());
  const editorPlain = await page.evaluate(() =>
    ["#live", "#source"].map((r) => [...document.querySelectorAll(`${r} .cm-line`)].filter((l) => /const (plain|bare)/.test(l.textContent)).map((l) => l.querySelectorAll('[class*="tok-"]').length).join()),
  );
  check(`${scheme}: and in both editors`, editorPlain.join("/") === "0,0/0,0", editorPlain.join("/"));
  check(`${scheme}: no style attribute on a token`, (await page.locator("#preview pre [style]").count()) === 0);
  await page.waitForFunction(() => document.querySelector('#preview .mermaid-block[data-state="done"]'), null, { timeout: 30000 }).catch(() => {});
  check(`${scheme}: a mermaid fence is still a diagram`, (await page.locator("#preview .mermaid-block svg").count()) === 1);
  const outside = requests.filter((u) => !u.startsWith(URL) && !u.startsWith("data:"));
  check(`${scheme}: nothing is requested from outside the app`, outside.length === 0, outside.join(" "));
  check(`${scheme}: no errors`, errors.length === 0, errors.join("; "));
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/code-${scheme}.png` });
  await ctx.close();
}

async function main() {
  const browser = await chromium.launch();
  const colours = {};
  for (const scheme of ["light", "dark"]) {
    await tasks(browser, scheme);
    await code(browser, scheme, colours);
  }
  check("light and dark use different colours", colours.preview.light !== colours.preview.dark);
  await browser.close();
  if (failed) {
    console.log(`${failed} failed`);
    process.exit(1);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
