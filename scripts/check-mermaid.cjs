// Browser check for Mermaid diagrams in the Live editor (#89, #136). Needs `npm run dev` (Vite
// on :1420) and Playwright: `node scripts/check-mermaid.cjs [screenshot dir]`. It mounts the
// Live editor alone through src/lib/checks/mermaid-harness.svelte.ts, under the app's CSP, and
// exits 1 when a check fails.
const { chromium } = require("playwright");
const crypto = require("crypto");
const URL = "http://127.0.0.1:1420/";
const SHOTS = process.argv[2];
// Tauri adds hashes for the page's inline boot script; the dev server doesn't, so this does.
const CSP = (hashes) => `default-src 'self'; script-src 'self' ${hashes}; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; connect-src ipc: http://ipc.localhost ws://127.0.0.1:1420 ws://localhost:1420; object-src 'none'; base-uri 'none'; form-action 'none'; frame-src 'none'`;
const fence = (s) => "```mermaid\n" + s + "\n```\n";
// The cursor starts on the first line; a fence there would be shown as source, for editing.
const INTRO = "Intro\n\n";
// Tall enough to have every diagram of a small note in view: the editor draws nothing else.
const TALL = 6000;
const four = [
  "flowchart TD\n  A[Start] --> B{Ok?}\n  B -->|yes| C[Done]\n  B -->|no| D[Retry] --> A",
  "sequenceDiagram\n  Alice->>Bob: Hello\n  Bob-->>Alice: Hi\n  Note over Alice,Bob: a note",
  "classDiagram\n  class Animal {\n    +String name\n    +eat()\n  }\n  Animal <|-- Dog",
  "gantt\n  title Plan\n  dateFormat YYYY-MM-DD\n  section A\n  Task one :a1, 2026-01-01, 3d\n  Task two :after a1, 2d",
];
async function main() {
  const browser = await chromium.launch();
  const out = {};
  for (const scheme of ["light", "dark"]) {
    const ctx = await browser.newContext({ colorScheme: scheme, viewport: { width: 1000, height: 800 } });
    const page = await ctx.newPage();
    const requests = [];
    const errors = [];
    page.on("request", (r) => requests.push(r.url()));
    page.on("console", (m) => { if (m.type() === "error" || /Content Security Policy|Refused/.test(m.text())) errors.push(m.text()); });
    page.on("pageerror", (e) => errors.push("pageerror " + e.message));
    page.on("dialog", (d) => { errors.push("DIALOG " + d.message()); d.dismiss(); });
    await page.route(URL, async (route) => {
      const res = await route.fetch();
      const html = await res.text();
      const hashes = [...html.matchAll(/<script(?![^>]*\bsrc=)[^>]*>([\s\S]*?)<\/script>/g)]
        .map((m) => `'sha256-${crypto.createHash("sha256").update(m[1]).digest("base64")}'`)
        .join(" ");
      await route.fulfill({ response: res, body: html, headers: { ...res.headers(), "content-security-policy": CSP(hashes) } });
    });
    await page.addInitScript(() => {
      window.__violations = [];
      document.addEventListener("securitypolicyviolation", (e) => window.__violations.push(e.violatedDirective + " " + e.blockedURI));
      window.__TAURI_INTERNALS__ = { invoke: async () => { throw new Error("mock"); }, transformCallback: () => 0 };
    });
    await page.goto(URL);
    await page.waitForTimeout(1500);
    await page.evaluate(() => import("/src/lib/checks/mermaid-harness.svelte.ts"));
    const mark = requests.length;
    // 1. no diagrams: mermaid not requested
    await page.evaluate(() => (window.h = window.__mountLive("# Plain\n\nNo diagrams here.\n\n```js\nlet a = 1;\n```\n")));
    await page.waitForTimeout(1000);
    out[scheme + ":plain mermaid requests"] = requests.slice(mark).filter((u) => /mermaid/.test(u)).length;
    // 2. four diagrams
    await page.setViewportSize({ width: 1000, height: TALL });
    await page.evaluate(([b, h]) => { window.h.unmount(); window.h = window.__mountLive(b, h); }, [INTRO + four.map(fence).join("\n"), TALL]);
        await page.waitForFunction(() => document.querySelectorAll('.mermaid-block[data-state="pending"]').length === 0, null, { timeout: 30000 });
    out[scheme + ":four"] = await page.evaluate(() => [...document.querySelectorAll(".mermaid-block")].map((b) => b.dataset.state + ":" + (b.querySelector("svg")?.getAttribute("aria-roledescription") ?? b.textContent.slice(0, 80))));
    out[scheme + ":labels"] = await page.evaluate(() => ["Start", "Ok?", "Alice", "Animal", "Task one"].every((t) => document.getElementById("scroller").querySelector(".mermaid-block svg") && document.getElementById("scroller").textContent.includes(t)));
    out[scheme + ":mermaid requests now"] = requests.slice(mark).filter((u) => /mermaid/.test(u)).length;
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/four-${scheme}.png`, fullPage: true });
    // 3. one broken of three
    await page.evaluate(([b, h]) => { window.h.unmount(); window.h = window.__mountLive(b, h); }, [INTRO + fence(four[0]) + fence("flowchart TD\n  A --> --> ]]") + fence(four[1]) + "\nOutro\n", TALL]);
    await page.waitForFunction(() => document.querySelectorAll('.mermaid-block[data-state="pending"]').length === 0, null, { timeout: 30000 });
    out[scheme + ":broken"] = await page.evaluate(() => [...document.querySelectorAll(".mermaid-block")].map((b) => b.dataset.state));
    out[scheme + ":body strays"] = await page.evaluate(() => [...document.body.children].filter((c) => c.id !== "scroller").map((c) => c.tagName + "#" + c.id));
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/broken-${scheme}.png`, fullPage: true });
    if (scheme === "light") {
      // 4. hostile
      const hostile = [
        'flowchart TD\n  A["<img src=x onerror=window.__pwned=1>"] --> B',
        'flowchart TD\n  A --> B\n  click A "javascript:window.__pwned=2"\n  click B href "javascript:window.__pwned=3"',
        '%%{init: {"securityLevel":"loose", "theme":"dark", "maxTextSize": 99999999}}%%\nflowchart TD\n  A["<b onmouseover=window.__pwned=4>x</b>"] --> B\n  click A call alert(5)',
        'flowchart TD\n  A["<script>window.__pwned=6</script>"] --> B',
        'sequenceDiagram\n  Alice->>Bob: <img src=x onerror=window.__pwned=7>',
        'flowchart TD\n  A --> B\n  click A "https://example.com" "tip"',
      ];
      await page.evaluate(([b, h]) => { window.h.unmount(); window.h = window.__mountLive(b, h); }, [INTRO + hostile.map(fence).join("\n"), TALL]);
      await page.waitForFunction(() => document.querySelectorAll('.mermaid-block[data-state="pending"]').length === 0, null, { timeout: 30000 });
      await page.waitForTimeout(500);
      // click and hover everything
      for (const el of await page.$$(".mermaid-block .node, .mermaid-block a, .mermaid-block b")) { try { await el.hover({ timeout: 500 }); await el.click({ timeout: 500 }); } catch {} }
      await page.waitForTimeout(500);
      out["hostile"] = await page.evaluate(() => ({
        pwned: window.__pwned ?? null,
        states: [...document.querySelectorAll(".mermaid-block")].map((b) => b.dataset.state),
        onAttrs: [...document.querySelectorAll(".mermaid-block *")].flatMap((e) => [...e.attributes].filter((a) => /^on/i.test(a.name)).map((a) => e.tagName + " " + a.name)),
        jsUrls: [...document.querySelectorAll(".mermaid-block *")].flatMap((e) => [...e.attributes].filter((a) => /javascript:/i.test(a.value)).map((a) => e.tagName + " " + a.name)),
        scripts: document.querySelectorAll(".mermaid-block script").length,
        links: [...document.querySelectorAll(".mermaid-block a")].map((a) => a.getAttribute("href") ?? a.getAttribute("xlink:href")),
        darkThemeLeak: document.querySelectorAll(".mermaid-block svg")[2]?.querySelector("style")?.textContent.slice(0, 200),
      }));
      if (SHOTS) await page.screenshot({ path: `${SHOTS}/hostile.png`, fullPage: true });
      // 5. limits
      const many = "flowchart TD\n" + Array.from({ length: 1200 }, (_, i) => `  n${i} --> n${i + 1}`).join("\n");
      const huge = "flowchart TD\n" + "  A --> B\n".repeat(6000);
      await page.evaluate(([b, h]) => { window.h.unmount(); window.h = window.__mountLive(b, h); }, [INTRO + fence(many) + fence(huge), TALL]);
      const t0 = Date.now();
      await page.waitForFunction(() => document.querySelectorAll(".mermaid-block").length === 2 && document.querySelectorAll('.mermaid-block[data-state="pending"]').length === 0, null, { timeout: 60000, polling: 200 });
      out["limits"] = { ms: Date.now() - t0, errs: await page.evaluate(() => [...document.querySelectorAll(".mermaid-error p")].map((p) => p.textContent.slice(0, 120))) };
      // 6. 50 diagrams
      const fifty = Array.from({ length: 50 }, (_, i) => `## Section ${i}\n\nSome text ${i}.\n\n` + fence(four[i % 4].replace("Start", "Start " + i))).join("\n");
      const t1 = Date.now();
      await page.setViewportSize({ width: 1000, height: 800 });
      await page.evaluate((b) => { window.h.unmount(); window.h = window.__mountLive(b); }, fifty);
      const firstText = Date.now() - t1;
      await page.waitForFunction(() => document.querySelectorAll('.mermaid-block[data-state="done"]').length >= 1, null, { timeout: 30000 });
      const firstDiagram = Date.now() - t1;
      await page.waitForTimeout(1500);
      const doneAfterWait = await page.evaluate(() => document.querySelectorAll('.mermaid-block[data-state="done"]').length);
      // scroll through, measure longest frame gap
      const scroll = await page.evaluate(async () => {
        const s = document.querySelector(".cm-scroller"); let worst = 0; let last = performance.now();
        let run = true; (function f() { const n = performance.now(); worst = Math.max(worst, n - last); last = n; if (run) requestAnimationFrame(f); })();
        for (let y = 0; y < s.scrollHeight; y += 200) { s.scrollTop = y; await new Promise((r) => setTimeout(r, 50)); }
        run = false; return { worstFrameMs: Math.round(worst), height: s.scrollHeight };
      });
      await page.waitForFunction(() => document.querySelectorAll('.mermaid-block[data-state="pending"]').length === 0, null, { timeout: 60000 });
      out["fifty"] = { firstTextMs: firstText, firstDiagramMs: firstDiagram, doneAfter1500ms: doneAfterWait, scroll, allMs: Date.now() - t1 };
      // 7. typing outside a diagram: no re-render
      await page.evaluate((b) => { window.h.unmount(); window.h = window.__mountLive(b); }, "Text\n\n" + fence(four[0]));
      await page.waitForFunction(() => document.querySelector('.mermaid-block[data-state="done"]'));
      out["typing"] = await page.evaluate(async (base) => {
        const ids = []; 
        for (let i = 0; i < 5; i++) { window.h.set(base + "more ".repeat(i)); await new Promise((r) => setTimeout(r, 30)); ids.push(document.querySelector(".mermaid-block")?.dataset.state + ":" + document.querySelector(".mermaid-block svg")?.id); }
        // typing inside the diagram
        const states = [];
        let src = base;
        for (let i = 0; i < 5; i++) { src = src.replace("Retry", "Retry" + i); window.h.set(src); await new Promise((r) => setTimeout(r, 50)); states.push(document.querySelector(".mermaid-block")?.dataset.state + (document.querySelector(".mermaid-block")?.dataset.stale !== undefined ? "/stale:" + !!document.querySelector(".mermaid-block svg") : "")); }
        await new Promise((r) => setTimeout(r, 1500));
        states.push(document.querySelector(".mermaid-block")?.dataset.state + ":" + document.querySelector(".mermaid-block svg")?.id);
        return { outside: ids, inside: states };
      }, "Text\n\n" + fence(four[0]));
    }
    // 8. theme switch while open
    await page.evaluate((b) => { window.h.unmount(); window.h = window.__mountLive(b); }, INTRO + fence(four[0]));
    await page.waitForFunction(() => document.querySelector('.mermaid-block[data-state="done"]'));
    const before = await page.evaluate(() => document.querySelector(".mermaid-block svg").id);
    await page.emulateMedia({ colorScheme: scheme === "light" ? "dark" : "light" });
    await page.waitForTimeout(1500);
    out[scheme + ":theme switch"] = before + " -> " + (await page.evaluate(() => document.querySelector(".mermaid-block svg").id));
    out[scheme + ":violations"] = await page.evaluate(() => window.__violations);
    out[scheme + ":errors"] = errors.filter((e) => !/mock|WebSocket|websocket|Vite server/.test(e)).slice(0, 10);
    await ctx.close();
  }
  console.log(JSON.stringify(out, null, 1));
  await browser.close();
  const fails = [];
  const expect = (ok, what) => ok || fails.push(what);
  for (const scheme of ["light", "dark"]) {
    expect(out[`${scheme}:plain mermaid requests`] === 0, `${scheme}: a note without diagrams loaded mermaid`);
    expect(out[`${scheme}:four`].every((s) => s.startsWith("done:")), `${scheme}: not all four diagram types rendered`);
    expect(out[`${scheme}:labels`], `${scheme}: diagram labels missing`);
    expect(out[`${scheme}:broken`].join() === "done,error,done", `${scheme}: broken diagram not shown as one inline error`);
    expect(out[`${scheme}:body strays`].length === 0, `${scheme}: mermaid left nodes on <body>`);
    expect(!/ -> (.*)$/.exec(out[`${scheme}:theme switch`]) || out[`${scheme}:theme switch`].split(" -> ")[0] !== out[`${scheme}:theme switch`].split(" -> ")[1], `${scheme}: theme switch did not re-render`);
    expect(out[`${scheme}:violations`].length === 0, `${scheme}: CSP violations`);
  }
  const h = out.hostile;
  expect(h.pwned === null && h.onAttrs.length === 0 && h.jsUrls.length === 0 && h.scripts === 0, "hostile input survived");
  expect(/fill:#1a1b19/.test(h.darkThemeLeak), "an init directive changed the theme");
  expect(out.limits.errs.length === 2, "limits did not produce two inline errors");
  expect(new Set(out.typing.outside).size === 1, "typing outside a diagram re-rendered it");
  expect(out.typing.inside.slice(0, -1).every((s) => s === "pending/stale:true"), "the last rendering was not kept while editing");
  expect(out.fifty.firstTextMs < 500, "50-diagram note: text not shown at once");
  if (fails.length) {
    console.error("FAILED:\n- " + fails.join("\n- "));
    process.exit(1);
  }
  console.log("All checks passed.");
}
main().catch((e) => { console.error(e); process.exit(1); });
