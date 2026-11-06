// Browser check for the link bar (#131). Needs `npm run dev` (Vite on :1420) and Playwright:
// `node scripts/check-link-bar.cjs [screenshot dir]`. Mounts the preview alone through
// src/lib/checks/mermaid-harness.svelte.ts with `resolve_link` mocked; exits 1 when a check fails.
const { chromium } = require("playwright");
const URL = "http://127.0.0.1:1420/";
const SHOTS = process.argv[2];
const body = [
  "# Links",
  "",
  "[the docs](https://example.com/docs?x=1) and https://bare.example.org/page",
  "",
  "[https://mybank.com](https://evil.example/login) and [mail](mailto:someone@example.com)",
  "",
  "[[Ideas]] and [[Twin]] and [[Nowhere]] and [top](#top)",
  "",
  "[long](https://docs.example.com/" + "segment/".repeat(40) + "page?id=42)",
  "",
  "[look-alike](https://аpple.com/caf%C3%A9)",
].join("\n");

async function main() {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1000, height: 800 } });
  const errors = [];
  // The page boot calls other commands, which the mock refuses with "mock".
  page.on("pageerror", (e) => { if (e.message !== "mock") errors.push(e.message); });
  await page.addInitScript(() => {
    const notes = { Ideas: [{ id: 1, folder: "Projects/Hush", title: "Ideas" }], Twin: [{ id: 2 }, { id: 3 }] };
    window.__TAURI_INTERNALS__ = {
      transformCallback: () => 0,
      invoke: async (cmd, args) => {
        if (cmd !== "resolve_link") throw new Error("mock");
        const n = notes[args.target] ?? [];
        const resolution = n.length === 1 ? { kind: "unique", ids: n[0].id } : n.length ? { kind: "ambiguous", ids: n.map((x) => x.id) } : { kind: "missing" };
        return { target: args.target, resolution, notes: n };
      },
    };
  });
  await page.goto(URL);
  await page.waitForTimeout(1500);
  await page.evaluate(() => import("/src/lib/checks/mermaid-harness.svelte.ts"));
  await page.evaluate((b) => (window.h = window.__mountPreview(b)), body);
  await page.waitForSelector(".preview a");

  const bar = () => page.evaluate(() => document.querySelector(".link-bar")?.textContent.trim() ?? null);
  const results = [];
  const check = (name, got, ok) => results.push({ name, got, ok: ok(got) });
  const hover = async (sel, shot) => {
    await page.locator(".preview a", { hasText: sel }).first().hover();
    await page.waitForTimeout(120);
    if (SHOTS && shot) await page.screenshot({ path: `${SHOTS}/${shot}.png` });
    return bar();
  };

  check("external", await hover("the docs", "external"), (s) => s === "https://example.com/docs?x=1");
  check("linkified", await hover("bare.example.org"), (s) => s === "https://bare.example.org/page");
  check("mismatch", await hover("mybank.com", "mismatch"), (s) => s.includes("Link text says mybank.com") && s.includes("https://evil.example/login"));
  check("mailto", await hover("mail"), (s) => s === "mailto:someone@example.com");
  check("wiki unique", await hover("Ideas", "wikilink"), (s) => s === "Note: Projects/Hush/Ideas");
  check("wiki ambiguous", await hover("Twin"), (s) => s === "2 notes match Twin");
  check("wiki missing", await hover("Nowhere"), (s) => s === "No note Nowhere (click to create)");
  check("anchor", await hover("top"), (s) => s === "#top (won't open)");
  check("long", await hover("long", "long"), (s) => s.startsWith("https://docs.example.com/") && s.includes("…") && s.endsWith("page?id=42"));
  check("punycode", await hover("look-alike"), (s) => s === "https://xn--pple-43d.com/café");
  check("bar width ≤ half window", await page.evaluate(() => document.querySelector(".link-bar").getBoundingClientRect().width), (w) => w <= 500);
  await page.mouse.move(900, 100);
  await page.waitForTimeout(120);
  check("hidden after leaving", await bar(), (s) => s === null);
  await page.locator(".preview a", { hasText: "the docs" }).focus();
  check("keyboard focus", await bar(), (s) => s === "https://example.com/docs?x=1");
  await page.keyboard.press("Tab");
  check("focus moves", await bar(), (s) => s === "https://bare.example.org/page");
  // A link under the bar's own corner moves the bar to the right.
  await page.mouse.move(900, 100);
  const right = await page.evaluate(async () => {
    const a = document.querySelector(".preview a");
    a.dispatchEvent(new MouseEvent("mouseover", { bubbles: true, clientX: 10, clientY: innerHeight - 5 }));
    await new Promise((r) => setTimeout(r, 50));
    const r = document.querySelector(".link-bar").getBoundingClientRect();
    return Math.round(innerWidth - r.right);
  });
  check("flips right under the pointer", right, (gap) => gap === 0);
  await page.evaluate(() => window.h.set("# Changed\n\n[x](https://x.example)"));
  await page.waitForTimeout(120);
  check("hidden after re-render", await bar(), (s) => s === null);

  for (const r of results) console.log(`${r.ok ? "ok  " : "FAIL"} ${r.name}: ${JSON.stringify(r.got)}`);
  if (errors.length) console.log("page errors:", errors);
  await browser.close();
  process.exit(results.every((r) => r.ok) && !errors.length ? 0 : 1);
}
main();
