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

const MARKS = [
  "Plain and ==important== text, ==**bold** inside== and a==b==c.",
  "",
  "Not marks: a == b, ==open, `==code==`.",
  "",
  "- ==in a list==",
  "",
  "Find me: important.",
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

async function mounted(browser, scheme, body, find, height = 900) {
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
  await page.evaluate(([b, f]) => window.__mountMarkdown(b, f), [body, find]);
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

async function marks(browser, scheme) {
  const { ctx, page, errors } = await mounted(browser, scheme, MARKS, "important");
  const want = ["important", "bold inside", "b", "in a list"];
  const preview = await page.evaluate(() => [...document.querySelectorAll("#preview mark")].map((m) => m.textContent));
  // Off the cursor the `==` are hidden, so a Live mark's text is the highlighted text.
  const live = await page.evaluate(() => [...document.querySelectorAll("#live .cm-live-mark")].map((m) => m.textContent));
  check(`${scheme}: preview highlights`, preview.join("|") === want.join("|"), preview.join("|"));
  check(`${scheme}: Live shows the same highlights, markers hidden`, live.join("|") === want.join("|"), live.join("|"));
  const colour = (sel) => page.evaluate((sel) => { const s = getComputedStyle(document.querySelector(sel)); return `${s.backgroundColor} ${s.color}`; }, sel);
  const p = await colour("#preview mark");
  check(`${scheme}: same colours in the preview and in Live`, p === (await colour("#live .cm-live-mark")), `${p} / ${await colour("#live .cm-live-mark")}`);
  const find = await page.evaluate(() => {
    const probe = document.createElement("mark");
    probe.className = "hush-find";
    document.body.append(probe);
    const c = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return { colour: c, matches: CSS.highlights.get("hush-find")?.size ?? -1 };
  });
  check(`${scheme}: not the colour of a find match`, !p.startsWith(find.colour), `${p} / ${find.colour}`);
  check(`${scheme}: find-in-note finds text inside and outside a highlight, and leaves the marks`, find.matches === 2 && preview.length === 4, JSON.stringify(find));
  const text = await page.locator("#preview").textContent();
  check(`${scheme}: text that is not a highlight keeps its ==`, text.includes("a == b") && text.includes("==open") && text.includes("==code=="));
  // The cursor on a highlight shows its markers.
  await page.locator("#live .cm-live-mark").first().click();
  await page.waitForTimeout(200);
  check(`${scheme}: the markers show while the cursor is on the highlight`, (await page.locator("#live .cm-live-mark").first().textContent()) === "==important==");
  check(`${scheme}: no errors`, errors.length === 0, errors.join("; "));
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/marks-${scheme}.png` });
  await ctx.close();
}

async function scripts(browser, scheme) {
  const { ctx, page, errors } = await mounted(browser, scheme, SCRIPTS);
  const want = "sub:2|sup:2|sub:2";
  const preview = await page.evaluate(() => [...document.querySelectorAll("#preview sub, #preview sup")].map((e) => `${e.localName}:${e.textContent}`).join("|"));
  const live = await page.evaluate(() =>
    [...document.querySelectorAll("#live .cm-live-sub, #live .cm-live-sup")].map((e) => `${e.classList.contains("cm-live-sub") ? "sub" : "sup"}:${e.textContent}`).join("|"),
  );
  check(`${scheme}: preview subscript and superscript`, preview === want, preview);
  check(`${scheme}: Live shows the same, markers hidden`, live === want, live);
  const style = (sel) => page.evaluate((sel) => { const s = getComputedStyle(document.querySelector(sel)); return `${s.verticalAlign} ${parseFloat(s.fontSize) < parseFloat(getComputedStyle(document.querySelector(sel).parentElement).fontSize)}`; }, sel);
  check(`${scheme}: lowered and raised, smaller, in both views`, [await style("#preview sub"), await style("#live .cm-live-sub"), await style("#preview sup"), await style("#live .cm-live-sup")].join() === "sub true,sub true,super true,super true");
  const heights = await page.evaluate(() => ({
    preview: [...document.querySelectorAll("#preview p")].slice(0, 2).map((p) => p.getBoundingClientRect().height),
    live: [...document.querySelectorAll("#live .cm-line")].filter((l) => /Water|plain line/.test(l.textContent)).map((l) => l.getBoundingClientRect().height),
  }));
  check(`${scheme}: a line with scripts is as high as one without`, heights.preview[0] === heights.preview[1] && heights.live[0] === heights.live[1], JSON.stringify(heights));
  const strikes = await page.evaluate(() => [[...document.querySelectorAll("#preview s")].map((e) => e.textContent).join("|"), [...document.querySelectorAll("#live .cm-live-strike")].map((e) => e.textContent).join("|")]);
  check(`${scheme}: strikethrough keeps working in both views`, strikes[0] === "Struck H2O|plain strike" && strikes[1] === strikes[0], strikes.join(" / "));
  const text = await page.locator("#preview").textContent();
  check(`${scheme}: lone markers, code and a link stay as written`, text.includes("~5 minutes, 2^10, x^2^, https://example.com/~user/~files."), text);
  await page.locator("#live .cm-live-sub").first().click();
  await page.waitForTimeout(200);
  check(`${scheme}: the markers show while the cursor is on the subscript`, (await page.locator("#live .cm-live-sub").first().textContent()) === "~2~");
  check(`${scheme}: no errors`, errors.length === 0, errors.join("; "));
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/scripts-${scheme}.png` });
  await ctx.close();
}

async function callouts(browser, scheme, seen) {
  const { ctx, page, errors } = await mounted(browser, scheme, CALLOUTS, undefined, 2200); // tall: the editor only draws the lines in view
  const want = ["warning:Check the passphrase", "note:Note", "tip:Folded in Obsidian", "note:An unknown type", "important:Outer", "danger:Inner"];
  const preview = await page.evaluate(() =>
    [...document.querySelectorAll("#preview blockquote.note-callout")].map((q) => {
      const title = q.querySelector(":scope > .note-callout-title");
      const s = getComputedStyle(title);
      const icon = getComputedStyle(title, "::before");
      return {
        id: `${[...q.classList].find((c) => c.startsWith("note-callout-")).slice(13)}:${title.textContent}`,
        colour: s.color,
        bar: getComputedStyle(q).borderLeftColor,
        tint: getComputedStyle(q).backgroundColor,
        icon: icon.maskImage || icon.webkitMaskImage,
        iconWidth: parseFloat(icon.width),
      };
    }),
  );
  const live = await page.evaluate(() =>
    [...document.querySelectorAll("#live .cm-line.cm-live-callout-title")].map((l) => {
      const icon = getComputedStyle(l, "::before");
      return {
        id: `${[...l.classList].find((c) => /^cm-live-callout-(?!title$)/.test(c)).slice(16)}:${l.textContent}`,
        colour: getComputedStyle(l).color,
        background: getComputedStyle(l).backgroundImage,
        icon: icon.maskImage || icon.webkitMaskImage,
      };
    }),
  );
  check(`${scheme}: preview callouts with their titles`, preview.slice(0, 6).map((c) => c.id).join("|") === want.join("|"), preview.map((c) => c.id).join("|"));
  check(`${scheme}: Live shows the same callouts, markers hidden`, live.slice(0, 6).map((c) => c.id).join("|") === want.join("|"), live.map((c) => c.id).join("|"));
  check(`${scheme}: all sixteen callouts in both views`, preview.length === 16 && live.length === 16, `${preview.length} ${live.length}`);
  check(`${scheme}: same title colour and icon per callout in both views`, preview.every((c, i) => c.colour === live[i].colour && c.icon === live[i].icon), JSON.stringify([preview.map((c) => c.colour), live.map((c) => c.colour)]));
  check(`${scheme}: the bar has the title's colour and the box a tint`, preview.every((c) => c.bar === c.colour && c.tint !== "rgba(0, 0, 0, 0)" && c.tint !== c.colour), JSON.stringify(preview[0]));
  check(`${scheme}: Live lines carry the bar and the tint`, live.every((c) => c.background.includes("linear-gradient")));
  check(`${scheme}: every icon is drawn from a bundled data URI`, preview.every((c) => c.icon.startsWith('url("data:image/svg+xml') && c.iconWidth > 10), preview.map((c) => c.icon.slice(0, 30)).join());
  check(`${scheme}: fifteen kinds have fifteen icons`, new Set(preview.map((c) => c.icon)).size === 15, String(new Set(preview.map((c) => c.icon)).size));
  check(`${scheme}: kinds differ in colour`, new Set(preview.map((c) => c.colour)).size >= 5, [...new Set(preview.map((c) => c.colour))].join());
  const text = await page.locator("#preview").textContent();
  check(`${scheme}: no marker is shown as text in a callout`, !(await page.locator("#preview blockquote.note-callout").allTextContents()).some((t) => /\[!/.test(t)));
  check(`${scheme}: a marker that is not on the first line stays text`, text.includes("[!note] not a callout"));
  check(`${scheme}: an ordinary quote is not a callout`, (await page.locator("#preview blockquote:not(.note-callout)").count()) === 2);
  check(`${scheme}: no style attribute in the preview`, (await page.locator("#preview blockquote [style], #preview blockquote[style]").count()) === 0);
  // The cursor on the title line shows the marker.
  await page.locator("#live .cm-line.cm-live-callout-title").first().click();
  await page.waitForTimeout(200);
  check(`${scheme}: the marker shows while the cursor is on the title line`, (await page.locator("#live .cm-line.cm-live-callout-title").first().textContent()).includes("[!warning] Check the"));
  check(`${scheme}: no errors`, errors.length === 0, errors.join("; "));
  seen[scheme] = preview.map((c) => c.colour).join();
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
  check("light and dark use different colours", colours.preview.light !== colours.preview.dark);
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
