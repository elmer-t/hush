// Browser check for reopening a note where it was left (#242): the line at the top and the
// cursor come back after going to another note, to Settings, through a lock and unlock,
// and after a mode switch, in Source and in Live. Also: the setting off, history off, a
// note that got shorter, a new note, and that nothing is put in the webview's storage.
// Needs `npm run dev` (Vite on :1420, or HUSH_DEV_URL) and Playwright:
//   NODE_PATH=$(npm root -g) node scripts/check-note-position.cjs
// Runs the app's own page on mocked commands, and exits 1 when a check fails.
const { chromium } = require("playwright");
const URL = process.env.HUSH_DEV_URL || process.env.DEV_URL || "http://127.0.0.1:1420/";

/**
 * Runs in the page before the app. The mocked vault outlives a reload in `window.name`, as
 * the real one does in its file; it follows the rules of `db/positions.rs`.
 */
function backend(mode) {
  if (!localStorage.getItem("hush.layout.editorMode")) localStorage.setItem("hush.layout.editorMode", JSON.stringify(mode));
  const fresh = { locked: false, restore: true, count: 10, recent: [], positions: {}, last: null, short: false };
  const vault = () => ({ ...fresh, ...JSON.parse(window.name || "{}") });
  const save = (change) => (window.name = JSON.stringify({ ...vault(), ...change }));
  const words = "the of and a to in is you that it he was for on are as with his they at be this have from or one had by word but not what all were we when your can said there use an each which she do how their if will up other about out many then them these so some her would make like him into time has look two more write go see number no way could people my than first water been call who oil its now find long down day did get come made may part".split(" ");
  let seed = 7;
  const rnd = () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 2 ** 32);
  const sentence = (n) => Array.from({ length: n }, () => words[(rnd() * words.length) | 0]).join(" ");
  // Long wrapped paragraphs, each starting with its number; headings, lists and tables in between.
  function body(id, paragraphs) {
    seed = id;
    const out = ["---", "tags: [journal]", "author: Someone", "---", ""];
    for (let p = 0; p < paragraphs; p++) {
      if (p % 10 === 0) out.push(`## Section ${p / 10}`, "");
      if (p % 25 === 12) out.push("| a | b | c |", "|---|---|---|", ...Array.from({ length: 5 }, () => `| ${sentence(2)} | ${sentence(2)} | ${sentence(2)} |`), "");
      if (p % 15 === 7) out.push(...Array.from({ length: 4 }, () => `- ${sentence(9)}`), "");
      out.push(`N${id}P${String(p).padStart(3, "0")} ${sentence(40 + ((rnd() * 80) | 0))}.`, "");
    }
    return out.join("\n");
  }
  const titles = { 1: "Long one", 2: "Short", 3: "Long two" };
  const summary = (id) => ({ id, folder: "docs", title: titles[id] ?? `Untitled ${id}`, author: null, topic: null, rating: null, readStatus: "unread", createdAt: 1.6e12, updatedAt: 1.7e12 });
  const meta = { author: "Someone", topic: null, rating: null, tags: ["journal"], frontmatterTags: ["journal"], attributes: [] };
  const text = (id) => (id === 2 ? body(2, 2) : id > 3 ? "" : body(id, vault().short ? 12 : 300));
  const opened = (id) => {
    const v = vault();
    const position = v.restore ? (v.positions[id] ?? null) : null;
    return { note: { ...summary(id), body: text(id), tags: meta.tags }, meta, position };
  };
  const record = (id) => {
    const v = vault();
    if (v.count === 0) return;
    const recent = [id, ...v.recent.filter((i) => i !== id)].slice(0, v.count);
    const positions = Object.fromEntries(Object.entries(v.positions).filter(([i]) => recent.includes(Number(i))));
    save({ recent, positions, last: id });
  };
  const commands = {
    vault_status: () => (vault().locked ? null : { name: "work", path: "C:\\vaults\\work.hush", icon: null }),
    recent_vaults: () => [{ name: "work", path: "C:\\vaults\\work.hush", exists: true }],
    folders: () => ["docs"], folder_sizes: () => [{ path: "docs", bytes: 1000 }],
    folder_notes: ({ folder }) => (folder === "docs" ? [1, 2, 3].map((id) => ({ ...summary(id), size: 1000 })) : []),
    sidebar_view: () => "tree", sidebar_folder: () => null, sidebar_search_scope: () => "folder", sidebar_search_mode: () => "words",
    confirm_trash: () => true, graph_scope: () => null, recent_opened_collapsed: () => false, auto_lock_minutes: () => 10,
    trash_count: () => 0, collapsed_sections: () => [], tag_suggestions: () => [], related_notes: () => [], backlinks: () => [], outgoing_links: () => [],
    index_status: () => ({ state: "idle" }), model_status: () => ({ model: "m", readyModel: "m", load: { state: "ready" }, reduced: null }),
    search_facets: () => ({ tags: [], authors: [], topics: [], attributes: [] }), touch: () => null,
    last_opened_note: () => (vault().last && vault().count > 0 ? opened(vault().last) : null),
    recent_opened_notes: () => vault().recent.map(summary),
    recent_opened_count: () => vault().count,
    set_recent_opened_count: ({ count }) => (save({ count, recent: vault().recent.slice(0, count), ...(count === 0 ? { positions: {}, last: null } : {}) }), null),
    open_note: ({ id, record: rec }) => { const note = opened(id); if (rec !== false) record(id); return note; },
    create_note: () => { record(4); return opened(4); },
    set_note_position: ({ id, position }) => {
      window.__positionWrites.push({ id, ...position });
      const v = vault();
      if (v.restore && v.count > 0 && v.recent.includes(id)) save({ positions: { ...v.positions, [id]: position } });
      return null;
    },
    restore_position: () => vault().restore,
    set_restore_position: ({ on }) => (save({ restore: on, ...(on ? {} : { positions: {} }) }), null),
    vault_icon: () => null,
    default_author: () => "", trash_days: () => 30, index_all_cores: () => false, memory_security: () => ({ on: false, active: false }),
    gpu_status: () => ({ adapters: [], best: null, enabled: true, model: "m", device: "cpu", reason: null, gpuModel: "m" }),
    tag_list: () => [], tag_training: () => [], model_downloads: () => [], suggestion_threshold: () => ({ model: "m", threshold: null, default: 0.8 }),
    vault_space: () => ({ fileBytes: 1e6, reclaimableBytes: 0, trashNotes: 0, trashTextBytes: 0, neededBytes: 1e6, incremental: true, worthCompacting: false }),
  };
  window.__positionWrites = [];
  window.__vault = vault;
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

/** What the user sees of the position: the first line shown, and where the cursor is in it. */
function seen() {
  const scroller = document.querySelector(".cm-scroller");
  if (!scroller) return null;
  // The first row of text shown: its line, and how far into the line it starts. Not
  // scrollTop: a fresh editor estimates the heights above differently.
  const box = scroller.getBoundingClientRect();
  const inLine = (node, offset) => {
    let line = node;
    while (line && !(line.nodeType === 1 && line.classList.contains("cm-line"))) line = line.parentNode;
    if (!line) return "+0";
    const range = document.createRange();
    range.setStart(line, 0);
    range.setEnd(node, offset);
    return `${line.textContent.slice(0, 8)}+${range.toString().length}`;
  };
  const lineLeft = scroller.querySelector(".cm-line").getBoundingClientRect().left;
  const style = getComputedStyle(scroller.querySelector(".cm-line"));
  const caret = document.caretRangeFromPoint(lineLeft + parseFloat(style.paddingLeft) + 3, box.top + 8);
  const top = caret ? inLine(caret.startContainer, caret.startOffset) : "";
  const selection = getSelection();
  return {
    top,
    cursor: selection.focusNode ? inLine(selection.focusNode, selection.focusOffset) : "",
    selected: selection.toString().length,
    scrollTop: Math.round(scroller.scrollTop),
    focus: document.activeElement?.closest(".cm-content") ? "editor" : document.activeElement?.className || document.activeElement?.tagName,
  };
}

async function main() {
  const browser = await chromium.launch();
  const fails = [];
  const check = (name, ok, detail = "") => { console.log(`${ok ? "ok  " : "FAIL"} ${name}${ok || !detail ? "" : `  (${detail})`}`); if (!ok) fails.push(name); };
  for (const mode of ["source", "live"]) {
    const page = await (await browser.newContext({ viewport: { width: 1400, height: 900 } })).newPage();
    const errors = [];
    page.on("pageerror", (e) => { errors.push(e.message); if (process.env.STACK) console.log(e.stack); });
    await page.addInitScript(backend, mode);
    const settle = () => page.waitForTimeout(350);
    const look = () => page.evaluate(seen);
    const open = async (title) => {
      await page.locator(".row.note", { hasText: title }).first().click();
      await page.waitForFunction((t) => document.querySelector(".note input.title")?.value === t && document.querySelector(".cm-content .cm-line"), title);
      await settle();
    };
    const start = async () => {
      await page.goto(URL);
      await page.locator(".row.folder[title=docs]").waitFor();
      if (!(await page.locator(".row.note").count())) await page.locator(".row.folder[title=docs]").click();
      await page.locator(".row.note").first().waitFor();
      await settle();
    };
    // The row that was cut by the top edge comes back whole, so the probe may see the row
    // after it: the same paragraph, at most one row (about 100 characters) apart.
    const sameTop = (a, b) => a.top.slice(0, 8) === b.top.slice(0, 8) && Math.abs(Number(a.top.split("+")[1]) - Number(b.top.split("+")[1])) <= 120;
    const same = (a, b) => sameTop(a, b) && a.cursor === b.cursor && a.selected === b.selected;
    const show = (s) => JSON.stringify(s);
    /** Scrolls the editor to `fraction` of the note and clicks in the paragraph in the middle of it. */
    const place = async (fraction) => {
      await page.evaluate((f) => { const s = document.querySelector(".cm-scroller"); s.scrollTop = (s.scrollHeight - s.clientHeight) * f; }, fraction);
      await settle();
      const box = await page.locator(".cm-scroller").boundingBox();
      await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
      await settle();
      return look();
    };
    const vault = () => page.evaluate(() => window.__vault());
    const writes = () => page.evaluate(() => window.__positionWrites);

    await start();
    await open("Long one");
    const first = await look();
    check(`${mode}: a note never opened starts at the top`, first.scrollTop === 0 && first.focus === "editor", show(first));

    // Another note and back.
    const left = await place(0.5);
    check(`${mode}: the test note is scrolled well down (${left.top}, scrollTop ${left.scrollTop})`, left.scrollTop > 5000 && /^N1P\d+ .\+[1-9]/.test(left.top));
    await open("Short");
    check(`${mode}: leaving the note stores its position in the vault at once`, (await writes()).some((w) => w.id === 1 && w.top > 1000));
    await open("Long one");
    let back = await look();
    check(`${mode}: another note and back: same line at the top, same cursor`, same(left, back), `${show(left)} / ${show(back)}`);
    check(`${mode}: the editor has the focus, as before`, back.focus === "editor", back.focus);

    // Reopening by itself must not move the position: five more round trips.
    for (let i = 0; i < 5; i++) {
      await open("Short");
      await open("Long one");
    }
    const sixth = await look();
    check(`${mode}: five more round trips without a move: the same row, no drift`, sixth.top === back.top && sixth.cursor === back.cursor, `${show(back)} / ${show(sixth)}`);

    // A selection, then Settings and back (the editor is rebuilt, no open_note call).
    await page.keyboard.down("Shift");
    for (let i = 0; i < 12; i++) await page.keyboard.press("ArrowRight");
    await page.keyboard.up("Shift");
    await settle();
    const selected = await look();
    check(`${mode}: a selection of 12 characters`, selected.selected === 12, show(selected));
    await page.click('button[aria-label="Settings"]');
    await page.locator("#settings-general").waitFor();
    check(`${mode}: Settings has "Reopen notes where you left off", on`, await page.getByLabel("Reopen notes where you left off").isChecked());
    await page.click('button[aria-label="Settings"]');
    await page.locator(".cm-content .cm-line").first().waitFor();
    await settle();
    back = await look();
    check(`${mode}: Settings and back: same line, cursor and selection`, same(selected, back), `${show(selected)} / ${show(back)}`);

    // The debounced write: move, don't leave the note, wait.
    const moved = await place(0.8);
    const before = (await writes()).length;
    await page.waitForTimeout(1500);
    check(`${mode}: no vault write right after a move`, (await writes()).length === before);
    await page.waitForTimeout(2200);
    const after = await writes();
    check(`${mode}: one vault write a few seconds after the last move`, after.length === before + 1 && after.at(-1).id === 1, `${after.length - before} writes`);

    // Idle lock: the vault closes without warning, the page reloads. Unlock again.
    await page.evaluate(() => (window.name = JSON.stringify({ ...window.__vault(), locked: true })));
    await page.goto(`${URL}?locked=idle`);
    await page.locator(".vault-row").first().waitFor();
    const storage = await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]));
    check(`${mode}: nothing of a position is in localStorage or sessionStorage`, !/position|anchor|"top"|N1P/i.test(storage), storage);
    await page.evaluate(() => (window.name = JSON.stringify({ ...window.__vault(), locked: false })));
    await start();
    await page.locator(".cm-content .cm-line").first().waitFor();
    await settle();
    back = await look();
    check(`${mode}: after lock and unlock the last note opens where it was`, same(moved, back), `${show(moved)} / ${show(back)}`);

    // A mode switch in between: the position is a document offset, so the line is the same.
    const other = mode === "source" ? "Live" : "Source";
    const kept = await place(0.3);
    await open("Short");
    await page.getByRole("button", { name: other, exact: true }).click();
    await open("Long one");
    back = await look();
    check(`${mode}: reopened in ${other}: same line at the top`, back.top.slice(0, 8) === kept.top.slice(0, 8) && back.cursor.slice(0, 8) === kept.cursor.slice(0, 8), `${show(kept)} / ${show(back)}`);
    await page.getByRole("button", { name: mode === "source" ? "Source" : "Live", exact: true }).click();
    await settle();

    // A new note: the title has the focus and the editor is at the top.
    await page.getByRole("button", { name: "New note" }).first().click();
    await page.waitForFunction(() => document.activeElement?.matches("input.title"));
    check(`${mode}: a new note has the focus in its title and its editor at the top`, (await look()).scrollTop === 0);

    // The note got shorter than the stored offsets (an import, a metadata edit).
    await open("Long one");
    await place(0.9);
    await open("Short");
    await page.evaluate(() => (window.name = JSON.stringify({ ...window.__vault(), short: true })));
    await open("Long one");
    back = await look();
    check(`${mode}: a note shorter than its stored position opens without error`, errors.length === 0 && back.focus === "editor" && back.scrollTop > 0, `${show(back)} ${errors.join("; ")}`);
    await page.evaluate(() => (window.name = JSON.stringify({ ...window.__vault(), short: false })));

    // The setting off: notes open at the top and the vault holds no positions.
    await open("Long two");
    await place(0.5);
    await page.click('button[aria-label="Settings"]');
    await page.getByLabel("Reopen notes where you left off").uncheck();
    await settle();
    check(`${mode}: turning it off deletes the positions in the vault`, JSON.stringify((await vault()).positions) === "{}" && (await vault()).restore === false);
    await page.click('button[aria-label="Settings"]');
    await page.locator(".cm-content .cm-line").first().waitFor();
    await settle();
    back = await look();
    check(`${mode}: off: back from Settings the note is at the top`, back.scrollTop === 0, show(back));
    await place(0.5);
    await open("Short");
    await open("Long two");
    back = await look();
    check(`${mode}: off: a note opens at the top as before`, back.scrollTop === 0 && back.focus === "editor", show(back));
    await page.waitForTimeout(3300);
    check(`${mode}: off: nothing is written to the vault`, JSON.stringify((await vault()).positions) === "{}");
    await page.click('button[aria-label="Settings"]');
    await page.getByLabel("Reopen notes where you left off").check();
    await settle();

    // History off (count 0): kept for the session, not in the vault.
    await page.locator("#settings-general input[type=number]").fill("0");
    await page.locator("#settings-general form", { hasText: "Recently opened notes" }).getByRole("button", { name: "Save" }).click();
    await settle();
    await page.click('button[aria-label="Settings"]');
    await page.locator(".cm-content .cm-line").first().waitFor();
    const session = await place(0.4);
    await open("Short");
    await open("Long two");
    back = await look();
    check(`${mode}: history off: the position is still kept for the session`, same(session, back), `${show(session)} / ${show(back)}`);
    await page.waitForTimeout(3300);
    check(`${mode}: history off: no position is written to the vault`, JSON.stringify((await vault()).positions) === "{}", show((await vault()).positions));

    check(`${mode}: no page errors (${errors.join("; ")})`, errors.length === 0);
    await page.context().close();
  }
  await browser.close();
  console.log(fails.length ? `\n${fails.length} failed` : "\nall passed");
  process.exit(fails.length ? 1 : 0);
}

main().catch((e) => { console.error(e); process.exit(1); });
