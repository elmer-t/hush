// Browser check for the Markdown features the Live editor renders (and, for code, the Source
// editor). Needs `npm run dev` (Vite on :1420) and Playwright: `node scripts/check-markdown.cjs
// [screenshot dir]`. It mounts both editors on the same note through
// src/lib/checks/markdown-harness.svelte.ts, in the light and the dark theme, and exits 1 when
// a check fails.
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

const MARKS = [
  "Plain and ==important== text, ==**bold** inside== and a==b==c.",
  "",
  "Not marks: a == b, ==open, `==code==`.",
  "",
  "- ==in a list==",
  "",
  "Find me: important.",
  "",
  "| a | b |",
  "| - | - |",
  "| plain | ==in a table== |",
].join("\n");

const SCRIPTS = [
  "Water is H~2~O and E = mc^2^.",
  "",
  "A plain line of about the same length.",
  "",
  "~~Struck H~2~O~~ and ~~plain strike~~.",
  "",
  "Not scripts: ~5 minutes, 2^10, `x^2^`, https://example.com/~user/~files.",
].join("\n");

const CALLOUTS = [
  "> [!warning] Check the **passphrase**",
  "> There is no recovery.",
  "",
  "> [!note]",
  "> A note without a title.",
  "",
  "> [!tip]- Folded in Obsidian",
  "> Shown open here.",
  "",
  "> [!custom] An unknown type",
  "",
  "> [!important] Outer",
  "> Text.",
  ">",
  "> > [!danger] Inner",
  "> > Deep.",
  "",
  "> An ordinary quote.",
  "",
  "> Text first",
  "> [!note] not a callout",
  "",
  ...["abstract", "info", "todo", "success", "question", "caution", "failure", "bug", "example", "quote"].map((k) => `> [!${k}]\n`),
].join("\n");

let failed = 0;
function check(name, ok, detail = "") {
  if (!ok) failed++;
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${ok || !detail ? "" : `: ${detail}`}`);
}

async function mounted(browser, scheme, body, height = 900) {
  const ctx = await browser.newContext({ colorScheme: scheme, viewport: { width: 1400, height } });
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
  await page.evaluate((b) => (window.h = window.__mountMarkdown(b)), body);
  await page.waitForSelector("#live .cm-content");
  await page.waitForTimeout(300);
  return { ctx, page, errors, requests };
}

const liveText = (page) => page.locator("#live .cm-content").innerText();
/** The colour a CSS token of the app has now, as the browser reports colours. */
const tokenValue = (page, property, name) =>
  page.evaluate(
    ([property, name]) => {
      const probe = document.createElement("span");
      probe.style[property] = `var(${name})`;
      document.body.append(probe);
      const value = getComputedStyle(probe)[property];
      probe.remove();
      return value;
    },
    [property, name],
  );

async function tasks(browser, scheme) {
  const { ctx, page, errors } = await mounted(browser, scheme, TASKS);
  const boxes = () =>
    page.evaluate(() =>
      [...document.querySelectorAll("#live .cm-live-task input")].map((b) => ({
        checked: String(b.checked),
        width: b.getBoundingClientRect().width,
        line: b.closest(".cm-line").textContent,
      })),
    );
  const live = await boxes();
  const want = ["false", "true", "true", "false", "true", "false"];
  check(`${scheme}: Live checkboxes`, live.map((b) => b.checked).join() === want.join(), JSON.stringify(live));
  check(`${scheme}: boxes are drawn`, live.every((b) => b.width >= 12), live.map((b) => b.width).join());
  check(`${scheme}: markers are not shown as text`, !live.some((b) => /\[[ xX]\]/.test(b.line)), live.map((b) => b.line).join(" / "));
  // A box replaces the bullet; an ordered item keeps its number.
  check(`${scheme}: no bullet before a box, numbers stay`, live.map((b) => /^\s*\d\./.test(b.line)).join() === "false,false,false,false,true,true", live.map((b) => b.line).join(" / "));
  const text = await liveText(page);
  check(`${scheme}: text that is not a task keeps its brackets`, text.includes("a [ ] in the middle") && text.includes("[ ] in code") && text.includes("- [ ] in a code block"), text);
  check(`${scheme}: plain items keep their bullet`, (await page.locator("#live .cm-line", { hasText: "plain item" }).locator(".cm-live-bullet").count()) === 1);
  // Ticking a box changes the source, which the other boxes are read from again.
  await page.locator("#live .cm-live-task input").first().click();
  await page.waitForTimeout(200);
  check(`${scheme}: a click ticks the box`, (await boxes()).map((b) => b.checked).join() === ["true", ...want.slice(1)].join());
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
  const views = { live: "#live .cm-live-codeblock", source: "#source .cm-content" };
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
  check(`${scheme}: Live and Source use the same colours`, colours.live[scheme] === colours.source[scheme], JSON.stringify(colours));
  const body = await page.evaluate(() => getComputedStyle([...document.querySelectorAll("#live .cm-line.cm-live-codeblock")].find((l) => /const plain/.test(l.textContent))).color);
  check(`${scheme}: keywords and strings differ from plain code`, new Set([body, ...colours.live[scheme].split("|").slice(0, 5)]).size >= 3);
  const editorPlain = await page.evaluate(() =>
    ["#live", "#source"].map((r) => [...document.querySelectorAll(`${r} .cm-line`)].filter((l) => /const (plain|bare)/.test(l.textContent)).map((l) => l.querySelectorAll('[class*="tok-"]').length).join()),
  );
  check(`${scheme}: an unknown language and a fence without one stay plain in both editors`, editorPlain.join("/") === "0,0/0,0", editorPlain.join("/"));
  await page.waitForFunction(() => document.querySelector('#live .mermaid-block[data-state="done"]'), null, { timeout: 30000 }).catch(() => {});
  check(`${scheme}: a mermaid fence is still a diagram in Live`, (await page.locator("#live .mermaid-block svg").count()) === 1);
  check(`${scheme}: and source in Source`, (await page.locator("#source .mermaid-block").count()) === 0 && (await page.locator("#source .cm-content").innerText()).includes("flowchart TD"));
  const outside = requests.filter((u) => !u.startsWith(URL) && !u.startsWith("data:"));
  check(`${scheme}: nothing is requested from outside the app`, outside.length === 0, outside.join(" "));
  check(`${scheme}: no errors`, errors.length === 0, errors.join("; "));
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/code-${scheme}.png` });
  await ctx.close();
}

async function marks(browser, scheme) {
  const { ctx, page, errors } = await mounted(browser, scheme, MARKS);
  const want = ["important", "bold inside", "b", "in a list"];
  // Off the cursor the `==` are hidden, so a Live mark's text is the highlighted text.
  const live = () => page.evaluate(() => [...document.querySelectorAll("#live .cm-live-mark")].map((m) => m.textContent));
  check(`${scheme}: Live highlights, markers hidden`, (await live()).join("|") === want.join("|"), (await live()).join("|"));
  const colour = (sel) => page.evaluate((sel) => { const s = getComputedStyle(document.querySelector(sel)); return `${s.backgroundColor} ${s.color}`; }, sel);
  const highlight = `${await tokenValue(page, "backgroundColor", "--highlight")} ${await tokenValue(page, "color", "--highlight-text")}`;
  const mark = await colour("#live .cm-live-mark");
  check(`${scheme}: highlights have the highlight colours`, mark === highlight, `${mark} / ${highlight}`);
  // A table is rendered by markdown.ts, where a highlight is a <mark>.
  check(`${scheme}: a highlight in a table too`, (await page.locator("#live .cm-live-table mark").textContent()) === "in a table" && (await colour("#live .cm-live-table mark")) === highlight, await colour("#live .cm-live-table mark"));
  const text = await liveText(page);
  check(`${scheme}: text that is not a highlight keeps its ==`, text.includes("a == b") && text.includes("==open") && text.includes("==code=="), text);
  // The cursor on a highlight shows its markers.
  await page.locator("#live .cm-live-mark").first().click();
  await page.waitForTimeout(200);
  check(`${scheme}: the markers show while the cursor is on the highlight`, (await page.locator("#live .cm-live-mark").first().textContent()) === "==important==");
  // Find runs through the editor's search; a match inside a highlight keeps the highlight.
  await page.evaluate(() => window.h.find("important"));
  await page.waitForTimeout(200);
  const matches = await page.evaluate(() => [...document.querySelectorAll("#live .cm-searchMatch")].map((m) => ({ colour: getComputedStyle(m).backgroundColor, marked: m.closest(".cm-live-mark") !== null || m.querySelector(".cm-live-mark") !== null })));
  check(`${scheme}: find-in-note finds text inside and outside a highlight`, matches.map((m) => m.marked).join() === "true,false", JSON.stringify(matches));
  check(`${scheme}: a highlight is not the colour of a find match`, matches.every((m) => !mark.startsWith(m.colour)), `${mark} / ${JSON.stringify(matches)}`);
  check(`${scheme}: no errors`, errors.length === 0, errors.join("; "));
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/marks-${scheme}.png` });
  await ctx.close();
}

async function scripts(browser, scheme) {
  const { ctx, page, errors } = await mounted(browser, scheme, SCRIPTS);
  const want = "sub:2|sup:2|sub:2";
  const live = await page.evaluate(() =>
    [...document.querySelectorAll("#live .cm-live-sub, #live .cm-live-sup")].map((e) => `${e.classList.contains("cm-live-sub") ? "sub" : "sup"}:${e.textContent}`).join("|"),
  );
  check(`${scheme}: Live subscript and superscript, markers hidden`, live === want, live);
  const style = (sel) => page.evaluate((sel) => { const s = getComputedStyle(document.querySelector(sel)); return `${s.verticalAlign} ${parseFloat(s.fontSize) < parseFloat(getComputedStyle(document.querySelector(sel).parentElement).fontSize)}`; }, sel);
  check(`${scheme}: lowered and raised, smaller`, [await style("#live .cm-live-sub"), await style("#live .cm-live-sup")].join() === "sub true,super true");
  const heights = await page.evaluate(() => [...document.querySelectorAll("#live .cm-line")].filter((l) => /Water|plain line/.test(l.textContent)).map((l) => l.getBoundingClientRect().height));
  check(`${scheme}: a line with scripts is as high as one without`, heights.length === 2 && heights[0] === heights[1], JSON.stringify(heights));
  const strikes = await page.evaluate(() => [...document.querySelectorAll("#live .cm-live-strike")].map((e) => e.textContent).join("|"));
  check(`${scheme}: strikethrough keeps working`, strikes === "Struck H2O|plain strike", strikes);
  const text = await liveText(page);
  check(`${scheme}: lone markers, code and a link stay as written`, text.includes("~5 minutes, 2^10, x^2^, https://example.com/~user/~files."), text);
  await page.locator("#live .cm-live-sub").first().click();
  await page.waitForTimeout(200);
  check(`${scheme}: the markers show while the cursor is on the subscript`, (await page.locator("#live .cm-live-sub").first().textContent()) === "~2~");
  check(`${scheme}: no errors`, errors.length === 0, errors.join("; "));
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/scripts-${scheme}.png` });
  await ctx.close();
}

async function callouts(browser, scheme, seen) {
  const { ctx, page, errors } = await mounted(browser, scheme, CALLOUTS, 2200); // tall: the editor only draws the lines in view
  const want = ["warning:Check the passphrase", "note:Note", "tip:Folded in Obsidian", "note:An unknown type", "important:Outer", "danger:Inner"];
  const live = await page.evaluate(() =>
    [...document.querySelectorAll("#live .cm-line.cm-live-callout-title")].map((l) => {
      const icon = getComputedStyle(l, "::before");
      return {
        id: `${[...l.classList].find((c) => /^cm-live-callout-(?!title$)/.test(c)).slice(16)}:${l.textContent}`,
        colour: getComputedStyle(l).color,
        background: getComputedStyle(l).backgroundImage,
        icon: icon.maskImage || icon.webkitMaskImage,
        iconWidth: parseFloat(icon.width),
      };
    }),
  );
  check(`${scheme}: Live callouts with their titles, markers hidden`, live.slice(0, 6).map((c) => c.id).join("|") === want.join("|"), live.map((c) => c.id).join("|"));
  check(`${scheme}: all sixteen callouts`, live.length === 16, String(live.length));
  // The bar is drawn in the title's colour, and the tint is that colour mixed with transparent.
  check(`${scheme}: lines carry the bar in the title's colour and a tint`, live.every((c) => c.background.includes("linear-gradient") && c.background.includes(c.colour) && c.background.includes("/ 0.09)")), JSON.stringify(live[0]));
  check(`${scheme}: every icon is drawn from a bundled data URI`, live.every((c) => c.icon.startsWith('url("data:image/svg+xml') && c.iconWidth > 10), live.map((c) => `${c.icon.slice(0, 30)} ${c.iconWidth}`).join());
  check(`${scheme}: fifteen kinds have fifteen icons`, new Set(live.map((c) => c.icon)).size === 15, String(new Set(live.map((c) => c.icon)).size));
  check(`${scheme}: kinds differ in colour`, new Set(live.map((c) => c.colour)).size >= 5, [...new Set(live.map((c) => c.colour))].join());
  const text = await liveText(page);
  check(`${scheme}: no marker is shown as text in a callout`, !(await page.locator("#live .cm-line.cm-live-callout").allTextContents()).some((t) => /\[!/.test(t)));
  check(`${scheme}: a marker that is not on the first line stays text`, text.includes("[!note] not a callout"));
  const quotes = await page.locator("#live .cm-line.cm-live-quote:not(.cm-live-callout)").allTextContents();
  check(`${scheme}: an ordinary quote is not a callout`, quotes.join("|") === "An ordinary quote.|Text first|[!note] not a callout", quotes.join("|"));
  // The cursor on the title line shows the marker.
  await page.locator("#live .cm-line.cm-live-callout-title").first().click();
  await page.waitForTimeout(200);
  check(`${scheme}: the marker shows while the cursor is on the title line`, (await page.locator("#live .cm-line.cm-live-callout-title").first().textContent()).includes("[!warning] Check the"));
  check(`${scheme}: no errors`, errors.length === 0, errors.join("; "));
  seen[scheme] = live.map((c) => c.colour).join();
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/callouts-${scheme}.png`, fullPage: true });
  await ctx.close();
}

async function main() {
  const browser = await chromium.launch();
  const colours = {};
  const calloutColours = {};
  for (const scheme of ["light", "dark"]) {
    await tasks(browser, scheme);
    await code(browser, scheme, colours);
    await marks(browser, scheme);
    await scripts(browser, scheme);
    await callouts(browser, scheme, calloutColours);
  }
  check("light and dark use different colours", colours.live.light !== colours.live.dark);
  check("callouts have other colours in the dark theme", calloutColours.light !== calloutColours.dark);
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
