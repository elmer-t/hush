// Browser check for search by meaning (#146). Needs `npm run dev` (Vite on :1420, or set
// HUSH_URL) and Playwright: `node scripts/check-meaning-search.cjs [screenshot dir]`. Runs
// the whole app with mocked commands at the minimum sidebar width; exits 1 when a check fails.
const { chromium } = require("playwright");
const URL = process.env.HUSH_URL || "http://127.0.0.1:1420/";
const SHOTS = process.argv[2];

function mocks(reduced) {
  const note = (id, folder, title) => ({
    id, folder, title, author: null, topic: null, rating: null, readStatus: "unread", createdAt: 1.7e12, updatedAt: 1.7e12,
  });
  const words = { note: note(1, "journal", "Feeling stuck notes"), snippet: "I am \u0002stuck\u0003 on the parser." };
  const both = { note: note(2, "journal", "Stuck again"), snippet: "\u0002Stuck\u0003 for a week now." };
  const meaning = { note: note(3, "journal/2019", "Going nowhere"), snippet: "Another week of going nowhere on the project." };
  window.__calls = [];
  const replies = {
    vault_status: { path: "/vaults/test.hush", name: "test" },
    folders: ["journal", "journal/2019", "work"],
    folder_sizes: [], folder_notes: [], recent_opened_notes: [], model_downloads: [],
    sidebar_view: "tree", sidebar_folder: null, sidebar_search_scope: "folder", sidebar_search_mode: "words",
    search_facets: { tags: [], topics: [], authors: [], attributeKeys: [] },
    index_status: { state: "indexing", done: 412000, complete: 300000, total: 1000000, etaSecs: null },
    model_status: reduced
      ? { model: "bge", readyModel: null, load: { state: "missing", file: "bge.onnx" }, reduced: { reason: "missing", message: "model file bge.onnx is missing" } }
      : { model: "bge", readyModel: "bge", load: { state: "ready" }, reduced: null },
    meaning_model_loaded: false,
  };
  window.__TAURI_INTERNALS__ = {
    transformCallback: () => 0,
    invoke: async (cmd, args) => {
      if (cmd.startsWith("plugin:event|")) return 1;
      window.__calls.push({ cmd, args: JSON.parse(JSON.stringify(args ?? null)) });
      if (cmd === "search_notes") {
        const q = args.query;
        if (q.mode === "meaning") {
          // The first meaning search waits for the model to load.
          await new Promise((r) => setTimeout(r, 400));
          replies.meaning_model_loaded = true;
          return {
            hits: [
              { ...both, why: { words: true, cosine: 0.81 } },
              { ...words, why: { words: true, cosine: null } },
              { ...meaning, why: { words: false, cosine: 0.7342 } },
            ],
            newestFirst: false, byMeaning: true, top: null, wordsOnly: null, cancelled: false,
          };
        }
        const hits = q.text.trim() ? [words, both] : [];
        return { hits, newestFirst: false, byMeaning: false, top: null, wordsOnly: null, cancelled: false };
      }
      return cmd in replies ? structuredClone(replies[cmd]) : null;
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  try {
    localStorage.setItem("hush.layout.leftWidth", "200");
  } catch {}
}

async function main() {
  const browser = await chromium.launch();
  const fails = [];
  const check = (name, ok) => { console.log(`${ok ? "ok  " : "FAIL"} ${name}`); if (!ok) fails.push(name); };
  const errors = [];
  const open = async (reduced) => {
    const page = await (await browser.newContext({ viewport: { width: 1100, height: 700 }, deviceScaleFactor: 2 })).newPage();
    page.on("pageerror", (e) => errors.push(e.message));
    await page.addInitScript(mocks, reduced);
    await page.goto(URL);
    await page.waitForSelector(".sidebar input[type=search]");
    await page.waitForTimeout(800);
    return page;
  };

  const page = await open(false);
  const box = page.locator(".sidebar input[type=search]");
  const lastSearch = () => page.evaluate(() => window.__calls.filter((c) => c.cmd === "search_notes").at(-1)?.args.query);
  const summary = () => page.locator(".sidebar .summary p").innerText();
  const sidebar = page.locator(".sidebar");
  const meaningButton = page.getByRole("group", { name: "Search mode" }).getByRole("button", { name: "Meaning" });

  await box.fill("feeling stuck");
  await page.waitForTimeout(600);
  check("words is the default mode", (await lastSearch()).mode === "words");
  check(`words summary unchanged (${await summary()})`, /^2 results in whole vault · best match first$/.test(await summary()));
  check("no markers in words mode", (await sidebar.locator(".why").count()) === 0);

  await meaningButton.click();
  await page.waitForTimeout(100);
  check("switching searches at once with mode meaning", (await lastSearch()).mode === "meaning");
  check(`loading model is said while the first search waits (${await summary()})`, (await summary()).includes("Loading model…"));
  await page.waitForTimeout(600);
  check(`summary says by meaning (${await summary()})`, (await summary()).startsWith("3 results in whole vault · by meaning"));
  check("Loading model… is gone", !(await summary()).includes("Loading model…"));
  check(
    `coverage while indexing (${await summary()})`,
    (await summary()).includes("meaning covers 412,000 of 1,000,000 notes (first part only for 112,000)"),
  );
  const marks = await sidebar.locator(".why").allInnerTexts();
  check(`markers words/meaning/both (${marks})`, marks.join(",") === "both,words,meaning");
  check("cosine on hover", (await sidebar.locator(".why.meaning").getAttribute("title")) === "Found by meaning, cosine 0.734");
  const snippet = await sidebar.locator(".item", { hasText: "Going nowhere" }).locator(".snippet").innerText();
  check("vector-only hit shows the body start", snippet.startsWith("Another week of going nowhere"));
  const stored = await page.evaluate(() => window.__calls.filter((c) => c.cmd === "set_sidebar_search_mode").at(-1)?.args);
  check("mode stored", stored?.mode === "meaning");
  if (SHOTS) await sidebar.screenshot({ path: `${SHOTS}/meaning-mode.png` });

  await box.focus();
  await page.keyboard.press("Alt+M");
  await page.waitForTimeout(100);
  check("Alt+M switches back to words", (await lastSearch()).mode === "words");
  check("coverage gone in words mode", !(await summary()).includes("meaning covers"));
  await page.keyboard.press("Alt+M");
  await page.waitForTimeout(700);

  // Graph and Discover keep the words query; their label says so.
  const discoverSource = await page.evaluate(() => window.__calls.some((c) => c.cmd === "search_notes"));
  check("searched", discoverSource);

  const off = await open(true);
  await off.locator(".sidebar input[type=search]").fill("feeling stuck");
  await off.waitForTimeout(600);
  const offButton = off.getByRole("group", { name: "Search mode" }).getByRole("button", { name: "Meaning" });
  check("Meaning is disabled in the reduced state", await offButton.isDisabled());
  check("with the reason", (await offButton.getAttribute("title")) === "Search by meaning is off: model file bge.onnx is missing");
  check("words search still runs", (await off.evaluate(() => window.__calls.filter((c) => c.cmd === "search_notes").at(-1)?.args.query.mode)) === "words");
  if (SHOTS) await off.locator(".sidebar").screenshot({ path: `${SHOTS}/meaning-off.png` });

  check(`no page errors (${errors.join("; ")})`, errors.length === 0);
  await browser.close();
  process.exit(fails.length ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
