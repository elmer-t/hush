// Browser check for leaving Settings, Import and Trash (#251): the view closes at once,
// also while the vault is slow to hand over the open note, and the editor follows when the
// note arrives. Covers the button clicked again, Esc, a click on the open note, and Back.
// Needs `npm run dev` (Vite on :1420, or HUSH_DEV_URL) and Playwright:
//   NODE_PATH=$(npm root -g) node scripts/check-close-views.cjs
// Runs the app's own page on mocked commands, and exits 1 when a check fails.
const { chromium } = require("playwright");
const URL = process.env.HUSH_DEV_URL || process.env.DEV_URL || "http://127.0.0.1:1420/";
// How long the mocked vault takes to read the open note again, as when a long query holds it.
const SLOW_MS = 5000;

/** Runs in the page before the app. `window.__slow` delays reading a note again by that many ms. */
function backend() {
  localStorage.setItem("hush.layout.editorMode", JSON.stringify("source"));
  const titles = { 1: "Alpha", 2: "Beta" };
  const bodies = { 1: Array.from({ length: 400 }, (_, i) => `Alpha line ${i}`).join("\n\n"), 2: "Beta body" };
  const positions = {};
  const recent = [];
  const summary = (id) => ({ id, folder: "docs", title: titles[id], author: null, topic: null, rating: null, readStatus: "unread", createdAt: 1.6e12, updatedAt: 1.7e12 });
  const meta = { author: null, topic: null, rating: null, tags: [], frontmatterTags: [], attributes: [] };
  const opened = (id) => ({ note: { ...summary(id), body: bodies[id], tags: [] }, meta, position: positions[id] ?? null });
  const later = (value, ms) => (ms ? new Promise((done) => setTimeout(() => done(value), ms)) : value);
  const commands = {
    vault_status: () => ({ name: "work", path: "C:\\vaults\\work.hush", icon: null }),
    recent_vaults: () => [], folders: () => ["docs"], folder_sizes: () => [{ path: "docs", bytes: 1000 }],
    folder_notes: ({ folder }) => (folder === "docs" ? [1, 2].map((id) => ({ ...summary(id), size: 1000 })) : []),
    sidebar_view: () => "tree", sidebar_folder: () => null, sidebar_search_scope: () => "folder", sidebar_search_mode: () => "words",
    confirm_trash: () => true, graph_scope: () => null, recent_opened_collapsed: () => false, auto_lock_minutes: () => 10,
    trash_count: () => 0, trash_entries: () => [], collapsed_sections: () => [], tag_suggestions: () => [], related_notes: () => [], backlinks: () => [], outgoing_links: () => [],
    index_status: () => ({ state: "idle" }), model_status: () => ({ model: "m", readyModel: "m", load: { state: "ready" }, reduced: null }),
    search_facets: () => ({ tags: [], authors: [], topics: [], attributes: [] }), touch: () => null,
    last_opened_note: () => null, recent_opened_notes: () => recent.map(summary), recent_opened_count: () => 10,
    // Reading the open note again (`record: false`) is the call that waited for the vault.
    open_note: ({ id, record }) => {
      window.__opens.push({ id, record: record !== false });
      if (record !== false) recent.splice(0, recent.length, id, ...recent.filter((i) => i !== id));
      return later(opened(id), record === false ? window.__slow : 0);
    },
    save_note: ({ id, body }) => { window.__saves.push({ id, body }); bodies[id] = body; return { summary: summary(id), meta }; },
    set_note_position: ({ id, position }) => { positions[id] = position; return null; },
    restore_position: () => true, vault_icon: () => null, import_rules: () => [], import_target_notes: () => 0,
    default_author: () => "", trash_days: () => 30, index_all_cores: () => false, memory_security: () => ({ on: false, active: false }),
    gpu_status: () => ({ adapters: [], best: null, enabled: true, model: "m", device: "cpu", reason: null, gpuModel: "m" }),
    tag_list: () => [], tag_training: () => [], model_downloads: () => [], suggestion_threshold: () => ({ model: "m", threshold: null, default: 0.8 }),
    vault_space: () => ({ fileBytes: 1e6, reclaimableBytes: 0, trashNotes: 0, trashTextBytes: 0, neededBytes: 1e6, incremental: true, worthCompacting: false }),
  };
  window.__slow = 0;
  window.__opens = [];
  window.__saves = [];
  window.__TAURI_INTERNALS__ = {
    transformCallback: () => 0,
    invoke: async (cmd, args) => {
      if (cmd.startsWith("plugin:")) return cmd === "plugin:app|version" ? "0.4.0" : 0;
      if (!(cmd in commands)) throw new Error(`no mock for ${cmd}`);
      return commands[cmd](args ?? {});
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
}

/** What the main area shows. */
function seen() {
  const pages = [...document.querySelectorAll("main > .page")];
  return {
    settings: !!document.querySelector("#settings-general"),
    import: pages.some((p) => !p.hidden && !p.querySelector("#settings-general")),
    trash: !!document.querySelector("main .trash, main [aria-labelledby=trash-title]") || /Trash is empty|Nothing in the trash/i.test(document.querySelector("main")?.textContent ?? ""),
    waiting: !!document.querySelector("main .empty [role=status]"),
    editor: !!document.querySelector("main .cm-content"),
    title: document.querySelector(".note input.title")?.value ?? null,
    text: document.querySelector("main .cm-content")?.textContent ?? "",
    scrollTop: Math.round(document.querySelector("main .cm-scroller")?.scrollTop ?? 0),
    banner: document.querySelector("main .banner")?.textContent ?? null,
  };
}

async function main() {
  const browser = await chromium.launch();
  const fails = [];
  const check = (name, ok, detail = "") => { console.log(`${ok ? "ok  " : "FAIL"} ${name}${ok || !detail ? "" : `  (${detail})`}`); if (!ok) fails.push(name); };
  const page = await (await browser.newContext({ viewport: { width: 1400, height: 900 } })).newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.addInitScript(backend);
  await page.goto(URL);
  await page.locator(".row.folder[title=docs]").waitFor();
  if (!(await page.locator(".row.note").count())) await page.locator(".row.folder[title=docs]").click();

  const look = () => page.evaluate(seen);
  const show = (s) => JSON.stringify({ ...s, text: s.text.slice(0, 30) });
  const slow = (ms) => page.evaluate((v) => (window.__slow = v), ms);
  const button = { settings: page.locator('header button[aria-label="Settings"]'), import: page.locator("header button", { hasText: "Import" }), trash: page.locator(".left button", { hasText: "Trash" }) };
  const openView = async (name) => {
    await button[name].click();
    await page.waitForFunction((n) => !document.querySelector("main .cm-content") && (n !== "settings" || document.querySelector("#settings-general")), name);
    const s = await look();
    check(`${name} opens`, s[name] && !s.editor, show(s));
  };
  /** Right after `leave`: the view is gone and the note is on its way; then the editor arrives. */
  const leaves = async (name, how, leave, delay) => {
    await slow(delay);
    const start = Date.now();
    await leave();
    await page.waitForFunction((n) => (n === "settings" ? !document.querySelector("#settings-general") : true), name, { timeout: 1000 }).catch(() => {});
    const at = await look();
    const took = Date.now() - start;
    check(`${name}, ${how}: the view is left at once (${took} ms, note takes ${delay} ms)`, !at[name] && took < 500 && (delay === 0 || !at.editor), show(at));
    if (delay) check(`${name}, ${how}: the editor waits for the note`, at.waiting && !at.editor, show(at));
    await page.locator("main .cm-content").waitFor({ timeout: delay + 3000 });
    const done = await look();
    check(`${name}, ${how}: the note is back`, done.editor && done.title === "Alpha" && !done.waiting && !done.banner, show(done));
    await slow(0);
    return done;
  };

  await page.locator(".row.note", { hasText: "Beta" }).first().click();
  await page.locator(".row.note", { hasText: "Alpha" }).first().click();
  await page.waitForFunction(() => document.querySelector(".note input.title")?.value === "Alpha");

  // An edit, and a place in the note, before leaving it.
  await page.locator("main .cm-content").click();
  await page.keyboard.press("Control+Home");
  await page.keyboard.type("EDITED-ONE ");
  await page.evaluate(() => { const s = document.querySelector("main .cm-scroller"); s.scrollTop = 4000; });
  await page.waitForTimeout(400);
  const before = await look();

  // The button clicked again, with the vault slow: the acceptance case.
  await openView("settings");
  let back = await leaves("settings", "button again", () => button.settings.click(), SLOW_MS);
  check("the edit made before Settings is in the note", (await page.evaluate(() => window.__saves.at(-1)?.body ?? "")).startsWith("EDITED-ONE "));
  check("the editor has the saved text, not the text from when the note was opened", await page.evaluate(() => document.querySelector("main .cm-content").cmView?.view.state.doc.toString().startsWith("EDITED-ONE ") ?? null) !== false);
  check(`the note is where it was left (scrollTop ${before.scrollTop} / ${back.scrollTop})`, Math.abs(back.scrollTop - before.scrollTop) < 200 && back.scrollTop > 1000);

  // The first autosave after coming back must not write an older body.
  await page.keyboard.press("Control+End");
  await page.keyboard.type(" EDITED-TWO");
  await page.waitForTimeout(900);
  const last = await page.evaluate(() => window.__saves.at(-1)?.body ?? "");
  check("the next autosave keeps the earlier edit", last.startsWith("EDITED-ONE ") && last.endsWith(" EDITED-TWO"), `${last.slice(0, 20)} … ${last.slice(-20)}`);

  const fast = 1500;
  await openView("import");
  await leaves("import", "button again", () => button.import.click(), fast);

  for (const name of ["settings", "import", "trash"]) {
    await openView(name);
    await leaves(name, "Esc", () => page.keyboard.press("Escape"), fast);
  }

  // A click on the open note, in Recently opened and in the tree.
  await openView("settings");
  const opensBefore = await page.evaluate(() => window.__opens.length);
  await leaves("settings", "click on the open note in Recently opened", () => page.locator(".left").getByText("Alpha").first().click(), fast);
  const opens = await page.evaluate((n) => window.__opens.slice(n), opensBefore);
  check("the open note is read again, not opened anew", opens.length === 1 && opens[0].record === false, JSON.stringify(opens));
  await openView("import");
  await leaves("import", "click on the open note in the tree", () => page.locator(".row.note", { hasText: "Alpha" }).first().click(), fast);

  // Back (Alt+Left) from Settings.
  await openView("settings");
  await leaves("settings", "Alt+Left", () => page.keyboard.press("Alt+ArrowLeft"), fast);
  await page.keyboard.press("Alt+ArrowRight");
  await page.locator("#settings-general").waitFor({ timeout: 2000 }).catch(() => {});
  check("Alt+Right goes forward to Settings again", (await look()).settings);
  await leaves("settings", "button again, vault fast", () => button.settings.click(), 0);

  // Another note clicked while the open one is still on its way: that note wins.
  await openView("settings");
  await slow(fast);
  await button.settings.click();
  await page.locator(".row.note", { hasText: "Beta" }).first().click();
  await page.waitForFunction(() => document.querySelector(".note input.title")?.value === "Beta", null, { timeout: 2000 }).catch(() => {});
  await page.waitForTimeout(fast + 400);
  const beta = await look();
  check("another note opened while waiting stays open when the first one arrives", beta.title === "Beta" && beta.editor && !beta.waiting, show(beta));
  await slow(0);

  // Settings opened again while the note is on its way: Settings stays.
  await openView("settings");
  await slow(fast);
  await button.settings.click();
  await page.waitForTimeout(100);
  await button.settings.click();
  await page.waitForTimeout(fast + 400);
  const again = await look();
  check("Settings opened again while waiting stays open", again.settings && !again.editor, show(again));
  await slow(0);

  // Esc keeps its meaning where something else uses it.
  await page.locator("#settings-general input").first().focus();
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  check("Esc in a Settings field doesn't close Settings", (await look()).settings);
  await page.locator(".left input").first().focus();
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  check("Esc in the sidebar's search box doesn't close Settings", (await look()).settings);
  await page.locator("#settings-general h2").first().click();
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => !document.querySelector("#settings-general"), null, { timeout: 1000 }).catch(() => {});
  check("Esc after a click in Settings closes it", !(await look()).settings);

  // Esc in the note view does nothing to the view.
  await page.locator("main .cm-content").waitFor();
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  check("Esc in the note view leaves the note open", (await look()).editor);

  check(`no page errors (${errors.join("; ")})`, errors.length === 0);
  await browser.close();
  console.log(fails.length ? `\n${fails.length} failed` : "\nall passed");
  process.exit(fails.length ? 1 : 0);
}

main().catch((e) => { console.error(e); process.exit(1); });
