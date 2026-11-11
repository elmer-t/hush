// Browser check for the search scope (#140). Needs `npm run dev` (Vite on :1420, or set
// HUSH_URL) and Playwright: `node scripts/check-search-scope.cjs [screenshot dir]`. Runs the
// whole app with mocked commands at the minimum sidebar width; exits 1 when a check fails.
const { chromium } = require("playwright");
const URL = process.env.HUSH_URL || "http://127.0.0.1:1420/";
const SHOTS = process.argv[2];

function mocks(opts = {}) {
  const note = (id, folder, title) => ({
    id, folder, title, author: null, topic: null, rating: null, readStatus: "unread", createdAt: 1.7e12, updatedAt: 1.7e12,
  });
  const all = [note(1, "Projects/Work", "Plan for Q3"), note(2, "Archive", "Old plan"), note(3, "", "Plan ideas")];
  window.__calls = [];
  const replies = {
    vault_status: { path: "/vaults/test.hush", name: "test" },
    folders: ["Archive", "Projects", "Projects/Work"],
    folder_sizes: [], folder_notes: [], recent_opened_notes: [], model_downloads: [],
    sidebar_view: "tree", sidebar_folder: opts.folder === undefined ? "Projects/Work" : opts.folder, sidebar_search_scope: "folder",
    search_facets: { tags: [], topics: [], authors: [], attributeKeys: [] },
    index_status: { state: "idle", done: 0, total: 0 },
    model_status: { model: "e5-small", readyModel: "e5-small", load: { state: "ready" }, reduced: null },
  };
  window.__TAURI_INTERNALS__ = {
    transformCallback: () => 0,
    invoke: async (cmd, args) => {
      if (cmd.startsWith("plugin:event|")) return 1;
      window.__calls.push({ cmd, args: JSON.parse(JSON.stringify(args ?? null)) });
      if (cmd === "search_notes") {
        const q = args.query;
        const words = q.text.trim().toLowerCase();
        const hits = all
          .filter((n) => q.folder === null || n.folder === q.folder || n.folder.startsWith(`${q.folder}/`))
          .filter((n) => !words || n.title.toLowerCase().includes(words))
          .map((n) => ({ note: n, snippet: null }));
        return { hits, newestFirst: false, cancelled: false };
      }
      return cmd in replies ? structuredClone(replies[cmd]) : null;
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  try {
    localStorage.setItem("hush.layout.leftWidth", String(opts.width ?? 200));
  } catch {}
}

/**
 * Where the search controls sit (#157): top to bottom the search box, the Words/Meaning and
 * This folder/Whole vault toggles, the results line, then Tree/Recent/Suggestions + Filters.
 */
function searchLayout() {
  const box = (sel) => { const e = document.querySelector(sel); if (!e) return null; const r = e.getBoundingClientRect(); return { top: r.top, bottom: r.bottom, left: r.left, right: r.right }; };
  const unsplit = (sel) => { const tops = [...document.querySelectorAll(`${sel} button`)].map((b) => Math.round(b.getBoundingClientRect().top)); return tops.length > 0 && new Set(tops).size === 1; };
  const status = document.querySelector(".sidebar .summary");
  const over = (sel) => { const e = document.querySelector(sel); return e ? e.scrollWidth - e.clientWidth : 0; };
  return {
    search: box(".sidebar label.search"), mode: box(".sidebar .mode-switch"), scope: box(".sidebar .scope-switch"),
    summary: box(".sidebar .summary"), bar: box(".sidebar .bar"),
    modeUnsplit: unsplit(".sidebar .mode-switch"), scopeUnsplit: unsplit(".sidebar .scope-switch"),
    statusRole: status?.getAttribute("role") ?? null, statusButtons: status ? status.querySelectorAll("button").length : -1,
    // The sidebar, and the rows #157 moved, inside the content box of .top. (The Tree/Recent/
    // Suggestions control is wider than 200 px on its own; that row is out of scope here.)
    overflow: Math.max(
      over(".sidebar"),
      ...[".mode-switch", ".scope-switch", ".summary p"].map((sel) => {
        const e = document.querySelector(`.sidebar ${sel}`);
        const top = document.querySelector(".sidebar .top");
        if (!e) return 0;
        return Math.ceil(e.getBoundingClientRect().right - (top.getBoundingClientRect().left + top.clientWidth - 12));
      }),
    ),
  };
}

function checkLayout(check, l, label, { folder, oneRow }) {
  const toggles = folder ? Math.max(l.mode.bottom, l.scope.bottom) : l.mode.bottom;
  check(`${label}: search box above the toggles`, l.search.bottom <= l.mode.top);
  check(`${label}: toggles above the results line`, toggles <= l.summary.top);
  check(`${label}: results line above Tree/Recent/Suggestions`, l.summary.bottom <= l.bar.top);
  check(`${label}: results line is a status without buttons`, l.statusRole === "status" && l.statusButtons === 0);
  check(`${label}: Words/Meaning on one line`, l.modeUnsplit);
  if (folder) {
    check(`${label}: This folder/Whole vault on one line`, l.scopeUnsplit);
    if (oneRow) check(`${label}: both toggles on one row, Words/Meaning first`, Math.abs(l.scope.top - l.mode.top) < 1 && l.scope.left > l.mode.right);
    else check(`${label}: scope toggle on its own line, left-aligned under Words/Meaning`, l.scope.top >= l.mode.bottom && Math.abs(l.scope.left - l.mode.left) < 1);
  } else {
    check(`${label}: no scope toggle without a folder`, l.scope === null);
  }
  check(`${label}: no horizontal overflow (${l.overflow})`, l.overflow <= 0);
}

async function main() {
  const browser = await chromium.launch();
  const fails = [];
  const check = (name, ok) => { console.log(`${ok ? "ok  " : "FAIL"} ${name}`); if (!ok) fails.push(name); };
  const page = await (await browser.newContext({ viewport: { width: 1100, height: 700 }, deviceScaleFactor: 2 })).newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.addInitScript(mocks);
  await page.goto(URL);
  await page.waitForSelector(".sidebar input[type=search]");
  await page.waitForTimeout(800);
  const box = page.locator(".sidebar input[type=search]");
  const lastSearch = () => page.evaluate(() => window.__calls.filter((c) => c.cmd === "search_notes").at(-1)?.args.query);
  const summary = () => page.locator(".sidebar .summary p").innerText();
  const heading = page.locator(".sidebar .explorer").getByText("in Projects/Work");
  const sidebar = page.locator(".sidebar");

  check("idle placeholder names the folder", (await box.getAttribute("placeholder")) === "Search Projects/Work");
  check("tree heading shows the folder", await heading.isVisible());

  await box.fill("plan");
  await page.waitForTimeout(600);
  check("query limited to the folder", (await lastSearch()).folder === "Projects/Work");
  check(`summary names the folder (${await summary()})`, /^1 result in Projects\/Work · best match first$/.test(await summary()));
  checkLayout(check, await page.evaluate(searchLayout), "200 px, folder", { folder: true, oneRow: false });
  if (SHOTS) await sidebar.screenshot({ path: `${SHOTS}/folder-scope.png` });

  await page.getByRole("button", { name: "Whole vault" }).click();
  await page.waitForTimeout(100);
  check("Whole vault searches at once with folder null", (await lastSearch()).folder === null);
  check(`summary says whole vault (${await summary()})`, /^3 results in whole vault · best match first$/.test(await summary()));
  const stored = await page.evaluate(() => window.__calls.find((c) => c.cmd === "set_sidebar_search_scope")?.args);
  check("scope stored as vault", stored?.scope === "vault");
  check("selected folder not cleared", !(await page.evaluate(() => window.__calls.some((c) => c.cmd === "set_sidebar_folder" && c.args.folder === null))));
  if (SHOTS) await sidebar.screenshot({ path: `${SHOTS}/whole-vault.png` });

  await page.getByRole("button", { name: "This folder" }).click();
  await box.fill("nothing like this");
  await page.waitForTimeout(600);
  const empty = await page.locator(".sidebar .empty").innerText();
  check(`empty state names the folder (${empty})`, empty === "No matching notes in Projects/Work.");
  if (SHOTS) await sidebar.screenshot({ path: `${SHOTS}/empty-folder.png` });
  await page.getByRole("button", { name: "Search the whole vault" }).click();
  await page.waitForTimeout(100);
  check("empty-state action searches the whole vault", (await lastSearch()).folder === null);

  await box.fill("plan");
  await box.press("Alt+f");
  await page.waitForTimeout(100);
  check("Alt+F switches back to the folder", (await lastSearch()).folder === "Projects/Work");
  check("text not changed by Alt+F", (await box.inputValue()) === "plan");

  await box.fill("");
  await page.waitForTimeout(300);
  check("tree heading still shows the folder", await heading.isVisible());
  const overflow = await page.evaluate(() => { const s = document.querySelector(".sidebar"); return s.scrollWidth - s.clientWidth; });
  check(`no horizontal overflow at minimum width (${overflow})`, overflow <= 0);
  check(`no page errors (${errors.join("; ")})`, errors.length === 0);

  // The order and wrapping at the minimum, default and maximum sidebar widths, with and
  // without a selected folder; with a screenshot directory, in light and dark too.
  for (const scheme of SHOTS ? ["light", "dark"] : ["light"]) {
    for (const width of [200, 280, 480]) {
      for (const folder of ["Projects/Work", null]) {
        const ctx = await browser.newContext({ viewport: { width: 1100, height: 700 }, deviceScaleFactor: 2, colorScheme: scheme });
        const p = await ctx.newPage();
        p.on("pageerror", (e) => errors.push(e.message));
        await p.addInitScript(mocks, { width, folder });
        await p.goto(URL);
        await p.waitForSelector(".sidebar input[type=search]");
        await p.waitForTimeout(500);
        await p.locator(".sidebar input[type=search]").fill("plan");
        await p.waitForTimeout(600);
        const label = `${width} px, ${folder ? "folder" : "no folder"}, ${scheme}`;
        checkLayout(check, await p.evaluate(searchLayout), label, { folder: folder !== null, oneRow: width >= 480 });
        if (SHOTS) await p.locator(".sidebar").screenshot({ path: `${SHOTS}/search-${width}-${folder ? "folder" : "vault"}-${scheme}.png` });
        await ctx.close();
      }
    }
  }
  check(`no page errors after the width checks (${errors.join("; ")})`, errors.length === 0);
  await browser.close();
  process.exit(fails.length ? 1 : 0);
}

main().catch((e) => { console.error(e); process.exit(1); });
