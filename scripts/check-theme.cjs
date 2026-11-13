// Browser check for the theme setting (#173): Settings > General > Theme switches the whole
// page, diagrams in Live and the graph, follows the OS on System, persists, and is
// applied before the first paint. Needs `npm run dev` (Vite on :1420, or HUSH_DEV_URL) and
// Playwright: `node scripts/check-theme.cjs [screenshot dir]`. Exits 1 when a check fails.
const { chromium } = require("playwright");
const URL = process.env.HUSH_DEV_URL || process.env.DEV_URL || "http://127.0.0.1:1420/";
const SHOTS = process.argv[2];
const DIAGRAM = "```mermaid\nflowchart TD\n  A[Start] --> B[Done]\n```\n";

function mocks() {
  const graph = {
    nodes: [
      { key: "n1", kind: "note", id: 1, label: "Centre note", folder: "", topic: null, tags: [], depth: 0 },
      { key: "n2", kind: "note", id: 2, label: "Linked note", folder: "a", topic: null, tags: [], depth: 1 },
      { key: "n3", kind: "note", id: 3, label: "Other note", folder: "b", topic: null, tags: [], depth: 1 },
    ],
    edges: [
      { source: "n1", target: "n2", kind: "link", score: null },
      { source: "n1", target: "n3", kind: "link", score: null },
    ],
    total: 3, capped: false, meaningUnavailable: false,
  };
  const replies = {
    auto_lock_minutes: 10, default_author: "", trash_days: 30, confirm_trash: true, recent_opened_count: 10,
    index_all_cores: false, memory_security: { on: false, active: false }, graph, graph_scope: null,
    "plugin:app|version": "0.1.0", model_downloads: [], tag_list: [], folders: [], recent_vaults: [],
    third_party_notices: "", suggestion_threshold: { model: "e5-small", threshold: null, default: 0.8 }, tag_training: [],
    gpu_status: { adapters: [], best: null, enabled: true, model: "e5-small", device: "cpu", reason: null, gpuModel: "e5-small" },
    index_status: { state: "idle" },
    model_status: { model: "e5-small", readyModel: "e5-small", load: { state: "ready" }, reduced: null },
    model_overview: { status: { model: "e5-small", readyModel: "e5-small", load: { state: "ready" }, reduced: null }, model: "e5-small", notes: 3, defaultRate: 1, models: [] },
    vault_space: { fileBytes: 1e6, reclaimableBytes: 0, trashNotes: 0, trashTextBytes: 0, neededBytes: 1e6, incremental: true, worthCompacting: false },
  };
  // Labels on every node, so the graph's label colour can be read off its canvas.
  localStorage.setItem("hush.graph.display", JSON.stringify({ labelSize: 0 }));
  window.__TAURI_INTERNALS__ = {
    invoke: async (cmd) => (cmd.startsWith("plugin:event|") ? 1 : cmd in replies ? structuredClone(replies[cmd]) : null),
    transformCallback: () => 0,
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  // When the theme was applied and when the first paint happened, to check for a flash.
  new PerformanceObserver((list) => {
    for (const e of list.getEntries()) if (e.name === "first-paint") window.__firstPaint = e.startTime;
  }).observe({ type: "paint", buffered: true });
  new MutationObserver(() => {
    if (window.__themedAt === undefined && document.documentElement.dataset.theme) window.__themedAt = performance.now();
  }).observe(document, { attributes: true, subtree: true, attributeFilter: ["data-theme"] });
}

const bg = (page) => page.evaluate(() => getComputedStyle(document.body).backgroundColor);
const LIGHT_BG = "rgb(246, 246, 243)";
const DARK_BG = "rgb(17, 19, 18)";

async function main() {
  const browser = await chromium.launch();
  const fails = [];
  const check = (name, ok, detail = "") => {
    console.log(`${ok ? "ok  " : "FAIL"} ${name}${ok || !detail ? "" : `: ${detail}`}`);
    if (!ok) fails.push(name);
  };

  // 1. Start-up: the stored choice wins over the OS, before the first paint.
  for (const [os, stored, want] of [["light", null, LIGHT_BG], ["dark", null, DARK_BG], ["light", "dark", DARK_BG], ["dark", "light", LIGHT_BG], ["dark", "{broken", DARK_BG]]) {
    const ctx = await browser.newContext({ colorScheme: os });
    const page = await ctx.newPage();
    await page.addInitScript(mocks);
    if (stored) await page.addInitScript((v) => localStorage.setItem("hush.theme", v.startsWith("{") ? v : JSON.stringify(v)), stored);
    await page.goto(URL);
    await page.waitForTimeout(800);
    const t = await page.evaluate(() => ({ themed: window.__themedAt, paint: window.__firstPaint }));
    const tag = `start, OS ${os}, stored ${stored ?? "nothing"}`;
    check(`${tag}: background`, (await bg(page)) === want, await bg(page));
    check(`${tag}: theme applied before first paint`, t.themed !== undefined && (t.paint === undefined || t.themed <= t.paint), JSON.stringify(t));
    await ctx.close();
  }

  // 2. Switching in Settings, with Live and the graph on screen.
  const ctx = await browser.newContext({ colorScheme: "light", viewport: { width: 1400, height: 900 } });
  const page = await ctx.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.addInitScript(mocks);
  await page.goto(URL);
  await page.waitForTimeout(800);
  await page.evaluate(() => import("/src/lib/checks/theme-harness.svelte.ts"));
  errors.length = 0;
  await page.evaluate((b) => window.__mountAll(b), DIAGRAM);
  const settled = () => page.waitForFunction(() => document.querySelectorAll('.mermaid-block[data-state="done"]').length >= 1 && !document.querySelector('.mermaid-block[data-state="pending"]'), null, { timeout: 30000 });
  await settled().catch(() => console.log("not settled; errors so far:", errors.join("; ")));
  await page.waitForTimeout(1500); // the graph layout
  const group = page.getByRole("group", { name: "Theme" });
  const pressed = () => group.locator('button[aria-pressed="true"]').textContent();
  check("Theme control has System, Light, Dark", (await group.locator("button").allTextContents()).join() === "System,Light,Dark");
  check("System selected on a fresh install", (await pressed()) === "System");

  // Diagram colours: mermaid writes the text colour into each SVG's <style>.
  const diagramText = () => page.evaluate(() => [...document.querySelectorAll("#live .mermaid-block svg")].map((s) => /#e7e9e5/i.test(s.innerHTML) ? "dark" : /#1a1b19/i.test(s.innerHTML) ? "light" : "?"));
  // Graph labels: sigma draws them on a 2D canvas in the label colour.
  const labelLuma = () => page.evaluate(() => {
    const c = document.querySelector("#graph canvas.sigma-labels");
    const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
    let max = 0, min = 255;
    for (let i = 0; i < d.length; i += 4) if (d[i + 3] > 200) { const l = (d[i] + d[i + 1] + d[i + 2]) / 3; max = Math.max(max, l); min = Math.min(min, l); }
    return { max, min };
  });
  const graphScheme = async () => { const { max, min } = await labelLuma(); return max > 180 ? "dark" : min < 80 ? "light" : "?"; };
  check("light: diagrams light", (await diagramText()).every((s) => s === "light"), (await diagramText()).join());
  check("light: graph labels dark text", (await graphScheme()) === "light", JSON.stringify(await labelLuma()));
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/switch-light.png` });

  const svgIds = () => page.evaluate(() => [...document.querySelectorAll(".mermaid-block svg")].map((s) => s.id).join());
  let before = await svgIds();
  await group.getByRole("button", { name: "Dark" }).click();
  await page.waitForFunction((b) => [...document.querySelectorAll(".mermaid-block svg")].map((s) => s.id).join() !== b && !document.querySelector('.mermaid-block[data-state="pending"]'), before, { timeout: 30000 });
  await page.waitForTimeout(300);
  check("Dark: page dark at once", (await bg(page)) === DARK_BG, await bg(page));
  check("Dark: pressed", (await pressed()) === "Dark");
  check("Dark: stored", (await page.evaluate(() => localStorage.getItem("hush.theme"))) === '"dark"');
  check("Dark: the Live diagram re-rendered dark", (await diagramText()).every((s) => s === "dark"), (await diagramText()).join());
  check("Dark: graph labels redrawn light", (await graphScheme()) === "dark", JSON.stringify(await labelLuma()));
  check("Dark: native controls dark", (await page.evaluate(() => getComputedStyle(document.documentElement).colorScheme)) === "dark");
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/switch-dark.png` });

  // An OS switch does nothing while Dark is chosen.
  await page.emulateMedia({ colorScheme: "dark" });
  await page.emulateMedia({ colorScheme: "light" });
  await page.waitForTimeout(300);
  check("Dark: OS switch to light ignored", (await bg(page)) === DARK_BG);

  // Back to System with the OS light, then an OS switch to dark is followed live.
  before = await svgIds();
  await group.getByRole("button", { name: "System" }).click();
  await page.waitForFunction((b) => [...document.querySelectorAll(".mermaid-block svg")].map((s) => s.id).join() !== b && !document.querySelector('.mermaid-block[data-state="pending"]'), before, { timeout: 30000 });
  check("System: follows the light OS", (await bg(page)) === LIGHT_BG);
  check("System: diagrams light", (await diagramText()).every((s) => s === "light"), (await diagramText()).join());
  check("System: stored", (await page.evaluate(() => localStorage.getItem("hush.theme"))) === '"system"');
  before = await svgIds();
  await page.emulateMedia({ colorScheme: "dark" });
  await page.waitForFunction((b) => [...document.querySelectorAll(".mermaid-block svg")].map((s) => s.id).join() !== b && !document.querySelector('.mermaid-block[data-state="pending"]'), before, { timeout: 30000 });
  await page.waitForTimeout(300);
  check("System: OS switch to dark followed live", (await bg(page)) === DARK_BG);
  check("System: diagrams follow the OS", (await diagramText()).every((s) => s === "dark"), (await diagramText()).join());
  check("System: graph follows the OS", (await graphScheme()) === "dark", JSON.stringify(await labelLuma()));

  // Light over a dark OS.
  await group.getByRole("button", { name: "Light" }).click();
  await page.waitForTimeout(300);
  check("Light: overrides the dark OS", (await bg(page)) === LIGHT_BG);
  check("Light: native controls light", (await page.evaluate(() => getComputedStyle(document.documentElement).colorScheme)) === "light");

  // Persists across a restart.
  await page.reload();
  await page.waitForTimeout(800);
  check("Light: kept after a restart, over the dark OS", (await bg(page)) === LIGHT_BG);
  check(`no page errors (${errors.join("; ")})`, errors.length === 0);

  await browser.close();
  if (fails.length) {
    console.log(`\n${fails.length} check(s) failed.`);
    process.exit(1);
  }
  console.log("\nAll checks passed.");
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
