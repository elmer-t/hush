// How long switching notes takes in the frontend (the whole workspace, on a mocked
// backend that answers at once, or after `--backend-ms` each). Needs `npm run dev`
// (Vite on :1420) and Playwright:
//   NODE_PATH=$(npm root -g) node scripts/check-open-note.cjs [options]
// For notes of four sizes (2 kB to 400 kB) it clicks through notes in the tree and reports, per switch:
//   shown: click until the new note's text is in the editor and painted,
//   settled: click until the details panel has its answers too and the page is idle.
// Options:
//   --notes N         notes in the vault (default and most 10,000)
//   --per-folder N    notes per folder (default 100); the tree loads 500 of a folder at a time
//   --load-all        load every page of each opened folder and keep the folders open, so
//                     the tree grows: with --per-folder 2500 to 2,500, 5,000, 7,500 and 10,000 rows
//   --mode live|source
//   --cpu N           slow the CPU N times, as a slower machine
//   --backend-ms N    each command answers after N ms
//   --switches N      switches measured per size (default 12)
//   --session N       instead: N switches in a row, the first 20 against the last 20
//   --profile         where the time of a switch goes; with --stacks, the call stacks too
const { chromium } = require("playwright");
const URL = "http://127.0.0.1:1420/";
const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  return i < 0 ? fallback : process.argv[i + 1];
};
const NOTES = Number(arg("notes", 10000));
const BACKEND_MS = Number(arg("backend-ms", 0));
const MODE = arg("mode", "live");
const CPU = Number(arg("cpu", 1));
const SWITCHES = Number(arg("switches", 12));
// Notes per folder; with `--load-all` every page of the opened folders is loaded (Show more).
const PER_FOLDER = Number(arg("per-folder", 100));
const LOAD_ALL = process.argv.includes("--load-all");
if (NOTES > 10000) throw new Error("at most 10,000 notes");

/** Runs in the page before the app: a vault of `notes` notes and the commands the workspace calls. */
function backend({ notes, backendMs, mode, perFolder }) {
  localStorage.setItem("hush.layout.editorMode", JSON.stringify(mode));
  let seed = 42;
  const rnd = () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 2 ** 32);
  const words = "the of and a to in is you that it he was for on are as with his they at be this have from or one had by word but not what all were we when your can said there use an each which she do how their if will up other about out many then them these so some her would make like him into time has look two more write go see number no way could people my than first water been call who oil its now find long down day did get come made may part".split(" ");
  const sentence = (n) => Array.from({ length: n }, () => words[(rnd() * words.length) | 0]).join(" ");
  // Sizes of a notes archive: most notes short, some long, a few very long.
  const SIZES = { small: 2_000, medium: 20_000, large: 100_000, huge: 400_000 };
  const kinds = Object.keys(SIZES);
  const folders = [];
  for (let i = 0; i < Math.max(4, notes / perFolder); i++) folders.push(`${kinds[i % kinds.length]}/folder ${i}`);
  const summaries = [];
  for (let id = 1; id <= notes; id++) {
    const folder = folders[id % folders.length];
    summaries.push({ id, folder, title: `Note ${String(id).padStart(5, "0")} ${sentence(2)}`, author: null, topic: null, rating: null, readStatus: "unread", createdAt: 1.6e12 + id * 1e6, updatedAt: 1.7e12 + id * 1e6 });
  }
  const bodies = new Map();
  function body(id) {
    if (bodies.has(id)) return bodies.get(id);
    const size = SIZES[summaries[id - 1].folder.split("/")[0]];
    seed = id;
    const out = ["---", "tags: [journal, bench]", "author: Someone", "---", ""];
    let length = 0;
    for (let block = 0; length < size; block++) {
      const part = [];
      switch (block % 8) {
        case 0: part.push(`## ${sentence(4)}`); break;
        case 3: for (let i = 0; i < 5; i++) part.push(`- ${sentence(8)} **${sentence(2)}** [[Note ${String(1 + ((rnd() * notes) | 0)).padStart(5, "0")}]]`); break;
        case 5: part.push(`> ${sentence(20)} *${sentence(3)}* [a link](https://example.com/${block})`); break;
        case 6: if (block % 40 === 6) part.push("| a | b | c |", "|---|---|---|", ...Array.from({ length: 6 }, () => `| ${sentence(2)} | ${sentence(2)} | ${sentence(2)} |`)); else part.push(sentence(90)); break;
        case 7: if (block % 80 === 7) part.push("```js", ...Array.from({ length: 12 }, (_, i) => `const v${i} = call(${i}, "${sentence(2)}");`), "```"); else part.push(sentence(70)); break;
        default: part.push(`${sentence(60)} ==${sentence(2)}== ${sentence(40)} \`${sentence(1)}\` ${sentence(30)}.`);
      }
      const text = part.join("\n");
      out.push(text, "");
      length += text.length + 2;
    }
    const text = out.join("\n");
    bodies.set(id, text);
    return text;
  }
  const meta = { author: "Someone", topic: null, rating: null, tags: ["journal", "bench"], frontmatterTags: ["journal", "bench"], attributes: [] };
  const recent = [];
  const pick = (id, n) => Array.from({ length: n }, (_, i) => summaries[(id * 7 + i * 131) % notes]);
  const reasons = { linksTo: false, linkedFrom: true, bothLinkTo: [], bothLinkedFrom: [], via: [], sharedTags: ["journal"], sameFolder: true, sameAuthor: null, sameTopic: null };
  const commands = {
    vault_status: () => ({ name: "bench", path: "/bench.hush" }),
    folders: () => folders,
    folder_sizes: () => folders.map((path) => ({ path, bytes: 1000 })),
    folder_notes: ({ folder, offset, limit }) => summaries.filter((s) => s.folder === folder).slice(offset, offset + limit).map((s) => ({ ...s, size: 1000 })),
    sidebar_view: () => "tree",
    sidebar_folder: () => null,
    sidebar_search_scope: () => "folder",
    sidebar_search_mode: () => "words",
    confirm_trash: () => true,
    last_opened_note: () => null,
    graph_scope: () => null,
    recent_opened_notes: () => recent.slice(0, 10).map((id) => summaries[id - 1]),
    recent_opened_count: () => 10,
    recent_opened_collapsed: () => false,
    open_note: ({ id }) => {
      const at = recent.indexOf(id);
      if (at >= 0) recent.splice(at, 1);
      recent.unshift(id);
      return { note: { ...summaries[id - 1], body: body(id), tags: meta.tags }, meta };
    },
    auto_lock_minutes: () => 10,
    trash_count: () => 0,
    collapsed_sections: () => [],
    tag_suggestions: () => [{ tag: "idea", score: 0.71, method: "description" }],
    related_notes: ({ id, limit }) => pick(id, limit).map((note, i) => ({ note, score: 0.9 - i * 0.02, reasons })),
    backlinks: ({ id }) => pick(id + 1, 12).map((note) => ({ note, ambiguous: false, snippet: sentence(20) })),
    outgoing_links: ({ id }) => pick(id + 2, 12).map((note) => ({ target: note.title, resolution: { kind: "unique", ids: note.id }, notes: [note] })),
    index_status: () => ({ state: "idle" }),
    model_status: () => ({ model: "m", readyModel: "m", load: { state: "ready" }, reduced: null }),
    search_facets: () => ({ tags: [], authors: [], topics: [], attributes: [] }),
    touch: () => null,
  };
  /** The titles of the first notes of `kind`'s first folder, in the tree's order. */
  window.__folder = (kind) => folders.find((f) => f.startsWith(`${kind}/`));
  window.__titles = (kind) => summaries.filter((n) => n.folder === window.__folder(kind)).map((n) => n.title).sort().slice(0, 40);
  window.__calls = [];
  window.__pending = 0;
  window.__TAURI_INTERNALS__ = {
    transformCallback: () => 0,
    invoke: async (cmd, args) => {
      if (cmd.startsWith("plugin:")) return 0;
      window.__calls.push({ cmd, at: performance.now() });
      window.__pending++;
      try {
        if (backendMs > 0) await new Promise((r) => setTimeout(r, backendMs));
        if (!(cmd in commands)) throw new Error("mock");
        return commands[cmd](args ?? {});
      } finally {
        window.__pending--;
      }
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  window.__longTasks = [];
  new PerformanceObserver((list) => {
    for (const e of list.getEntries()) window.__longTasks.push({ at: e.startTime, ms: e.duration });
  }).observe({ entryTypes: ["longtask"] });
}

/** Clicks the tree row of the note and waits; resolves to the times since the click. */
async function openNote(page, title) {
  return page.evaluate(async (title) => {
    const frame = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
    const find = () => [...document.querySelectorAll(".row.note")].find((r) => r.querySelector(".name").textContent === title);
    // The tree may draw only the rows in view: scroll through it until the row is there.
    const tree = document.querySelector('[role="tree"]');
    let row = find();
    for (let top = 0; !row && top <= tree.scrollHeight; top += tree.clientHeight / 2) {
      tree.scrollTop = top;
      await frame();
      row = find();
    }
    if (!row) throw new Error(`no row ${title}`);
    await frame();
    window.__longTasks.length = 0;
    const start = performance.now();
    row.click();
    while (document.querySelector(".note input.title")?.value !== title || !document.querySelector(".cm-content .cm-line")) await frame();
    await frame();
    const shown = performance.now() - start;
    // Settled: no command under way and two frames in a row without one starting.
    for (let quiet = 0; quiet < 3; ) {
      const calls = window.__calls.length;
      await frame();
      quiet = window.__pending === 0 && window.__calls.length === calls ? quiet + 1 : 0;
    }
    const settled = performance.now() - start;
    const blocked = window.__longTasks.filter((t) => t.at >= start).reduce((sum, t) => sum + t.ms, 0);
    return { shown, settled, blocked };
  }, title);
}

async function main() {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
  page.on("pageerror", (e) => { if (e.message !== "mock") console.log("page error:", e.message); });
  await page.addInitScript(backend, { notes: NOTES, backendMs: BACKEND_MS, mode: MODE, perFolder: PER_FOLDER });
  await page.goto(URL);
  await page.waitForTimeout(1500);
  await page.evaluate(() => import("/src/lib/checks/open-note-harness.svelte.ts"));
  await page.evaluate(() => window.__mountWorkspace());
  await page.waitForSelector(".row.folder");
  if (CPU > 1) {
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Emulation.setCPUThrottlingRate", { rate: CPU });
  }
  console.log(`${NOTES} notes, ${MODE} mode, backend ${BACKEND_MS} ms per command, CPU x${CPU} slower`);
  const median = (v) => v.slice().sort((a, b) => a - b)[v.length >> 1];
  const max = (v) => Math.max(...v);
  // `--session N`: N switches between small notes in one folder, as a long session has,
  // reporting the first and the last 20, with the style rules the page holds.
  const SESSION = Number(arg("session", 0));
  if (SESSION > 0) {
    await page.locator(`.row.folder[title="small"]`).click();
    await page.waitForTimeout(200);
    await page.locator(`.row.folder[title="${await page.evaluate(() => window.__folder("small"))}"]`).click();
    await page.waitForSelector(`.row.note[aria-level="3"]`);
    const titles = await page.evaluate(() => window.__titles("small"));
    const rules = () => page.evaluate(() => [...document.styleSheets, ...document.adoptedStyleSheets].reduce((n, s) => n + s.cssRules.length, 0));
    const shown = [];
    const rulesBefore = await rules();
    for (let i = 0; i < SESSION + 2; i++) {
      const r = await openNote(page, titles[i % titles.length]);
      if (i >= 2) shown.push(r.shown);
    }
    console.log(`session of ${SESSION} switches: first 20 shown in median ${median(shown.slice(0, 20)).toFixed(0)} ms, last 20 in median ${median(shown.slice(-20)).toFixed(0)} ms; style rules ${rulesBefore} before, ${await rules()} after`);
    await browser.close();
    return;
  }
  for (const kind of ["small", "medium", "large", "huge"]) {
    // Expand the kind's folder and its first subfolder.
    await page.locator(`.row.folder[title="${kind}"]`).click();
    await page.waitForTimeout(200);
    const folder = await page.evaluate((kind) => window.__folder(kind), kind);
    await page.locator(`.row.folder[title="${folder}"]`).click();
    await page.waitForSelector(`.row.note[title^="${folder}/"]`);
    // Every page of every open folder. The tree may draw only the rows in view, so each
    // Show more row is looked for by scrolling through it.
    while (LOAD_ALL) {
      const clicked = await page.evaluate(async () => {
        const tree = document.querySelector('[role="tree"]');
        for (let top = 0; top <= tree.scrollHeight; top += tree.clientHeight / 2) {
          tree.scrollTop = top;
          await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
          const more = document.querySelector(".row.more");
          if (more) return more.click(), true;
        }
        return false;
      });
      if (!clicked) break;
      await page.waitForTimeout(300);
    }
    await page.evaluate(() => (document.querySelector('[role="tree"]').scrollTop = 0));
    const titles = await page.evaluate((kind) => window.__titles(kind), kind);
    const runs = [];
    const profiling = process.argv.includes("--profile") && (!LOAD_ALL || kind === "huge");
    const cdp = profiling ? await page.context().newCDPSession(page) : null;
    const metrics = async () => Object.fromEntries((await cdp.send("Performance.getMetrics")).metrics.map((m) => [m.name, m.value]));
    let before;
    if (cdp) {
      await cdp.send("Performance.enable");
      await cdp.send("Profiler.enable");
      await cdp.send("Profiler.setSamplingInterval", { interval: 200 });
      for (let i = 0; i < 2; i++) await openNote(page, titles[i]);
      before = await metrics();
      await cdp.send("Profiler.start");
    }
    for (let i = 0; i < SWITCHES + 2; i++) {
      const r = await openNote(page, titles[i % titles.length]);
      // The first two warm up the fonts, the grammars and the JIT.
      if (i >= 2) runs.push(r);
      await page.waitForTimeout(150);
    }
    if (cdp) {
      const { profile } = await cdp.send("Profiler.stop");
      const after = await metrics();
      const n = SWITCHES + 2;
      const per = (k) => (((after[k] - before[k]) * 1000) / n).toFixed(1);
      console.log(`  per switch: script ${per("ScriptDuration")} ms, style ${per("RecalcStyleDuration")} ms, layout ${per("LayoutDuration")} ms, task ${per("TaskDuration")} ms`);
      const self = new Map();
      const dt = profile.timeDeltas;
      const byId = new Map(profile.nodes.map((nd) => [nd.id, nd]));
      profile.samples.forEach((id, i) => {
        const f = byId.get(id).callFrame;
        const key = `${f.functionName || "(anonymous)"} ${f.url.split("/").slice(-2).join("/").split("?")[0]}:${f.lineNumber + 1}`;
        self.set(key, (self.get(key) ?? 0) + dt[i] / 1000);
      });
      if (process.argv.includes("--stacks")) {
        const parent = new Map();
        for (const nd of profile.nodes) for (const c of nd.children ?? []) parent.set(c, nd.id);
        const name = (id) => { const f = byId.get(id).callFrame; return `${f.functionName || "(anon)"}@${f.url.split("/").pop().split("?")[0]}:${f.lineNumber + 1}`; };
        const stacks = new Map();
        profile.samples.forEach((id, i) => {
          if (byId.get(id).callFrame.functionName === "(idle)") return;
          const chain = [];
          for (let at = id; at !== undefined && chain.length < 14; at = parent.get(at)) chain.push(name(at));
          const key = chain.join(" < ");
          stacks.set(key, (stacks.get(key) ?? 0) + dt[i] / 1000);
        });
        for (const [k, v] of [...stacks].sort((a, b) => b[1] - a[1]).slice(0, 14)) console.log(`    ${(v / n).toFixed(1).padStart(7)} ms  ${k}`);
      }
      for (const [k, v] of [...self].sort((a, b) => b[1] - a[1]).slice(0, 22)) console.log(`    ${(v / n).toFixed(1).padStart(7)} ms  ${k}`);
    }
    const rows = await page.evaluate(() => document.querySelectorAll('[role="tree"] .row').length);
    const loaded = await page.evaluate(() => Math.round(document.querySelector('[role="tree"]').scrollHeight / 31));
    const r = { shown: median(runs.map((r) => r.shown)), shownMax: max(runs.map((r) => r.shown)), settled: median(runs.map((r) => r.settled)), blocked: median(runs.map((r) => r.blocked)) };
    console.log(`${kind.padEnd(7)} shown: median ${r.shown.toFixed(0)} ms, worst ${r.shownMax.toFixed(0)} ms; settled: median ${r.settled.toFixed(0)} ms; main thread blocked: median ${r.blocked.toFixed(0)} ms (${loaded} tree rows, ${rows} drawn)`);
    // Collapse again, so every kind is measured with the same tree.
    if (!LOAD_ALL) await page.locator(`.row.folder[title="${kind}"]`).click();
    await page.waitForTimeout(200);
  }
  await browser.close();
}

// scripts/check-tree.cjs uses the same mocked vault.
module.exports = { backend };

if (require.main === module) {
  main().catch((e) => {
    console.error(e);
    process.exit(1);
  });
}
