// Browser check for the note tree, which draws only the rows in view. Needs `npm run dev`
// (Vite on :1420) and Playwright:
//   NODE_PATH=$(npm root -g) node scripts/check-tree.cjs [screenshot dir]
// Mounts the whole workspace through src/lib/checks/open-note-harness.svelte.ts on the mocked
// vault of scripts/check-open-note.cjs (10,000 notes, 2,500 per folder); exits 1 when a check fails.
const { chromium } = require("playwright");
const { backend } = require("./check-open-note.cjs");
const URL = "http://127.0.0.1:1420/";
const SHOTS = process.argv[2];
const PITCH = 31;

async function main() {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  const errors = [];
  page.on("pageerror", (e) => { if (e.message !== "mock") errors.push(e.message); });
  await page.addInitScript(backend, { notes: 10000, backendMs: 0, mode: "live", perFolder: 2500 });
  await page.goto(URL);
  await page.waitForTimeout(1500);
  await page.evaluate(() => import("/src/lib/checks/open-note-harness.svelte.ts"));
  await page.evaluate(() => window.__mountWorkspace());
  await page.waitForSelector(".row.folder");

  const results = [];
  const check = (name, got, ok) => results.push({ name, got, ok: ok(got) });
  const tree = page.locator('[role="tree"]');
  const state = () =>
    page.evaluate(() => {
      const tree = document.querySelector('[role="tree"]');
      const box = tree.getBoundingClientRect();
      const rows = [...tree.querySelectorAll(".row")];
      const inView = rows.filter((r) => { const b = r.getBoundingClientRect(); return b.bottom > box.top && b.top < box.bottom; });
      const active = document.activeElement?.closest?.(".row");
      const selected = tree.querySelector(".row.selected");
      const visible = (el) => { if (!el) return false; const b = el.getBoundingClientRect(); return b.top >= box.top - 1 && b.bottom <= box.bottom + 1; };
      return {
        drawn: rows.length,
        total: Math.round((tree.querySelector(".sizer").offsetHeight + 1) / 31),
        scrollTop: tree.scrollTop,
        // No gaps: the rows in view cover it from top to bottom.
        covered: inView.length >= Math.floor(Math.min(box.height, tree.querySelector(".sizer").offsetHeight + 1) / 31),
        tops: inView.map((r) => Math.round(r.getBoundingClientRect().top - box.top + tree.scrollTop)).sort((a, b) => a - b),
        focused: active?.querySelector(".name")?.textContent ?? null,
        focusedVisible: visible(active),
        selected: selected?.querySelector(".name")?.textContent ?? null,
        selectedVisible: visible(selected),
        title: document.querySelector(".note input.title")?.value ?? null,
      };
    });
  const evenlySpaced = (tops) => tops.every((t, i) => t % PITCH === 0 && (i === 0 || t - tops[i - 1] === PITCH));
  const shot = async (name) => { if (SHOTS) await page.screenshot({ path: `${SHOTS}/tree-${name}.png` }); };

  // Two folders of 2,500 notes each, every page loaded.
  for (const kind of ["small", "medium"]) {
    await page.locator(`.row.folder[title="${kind}"]`).click();
    await page.waitForTimeout(150);
    await page.locator(`.row.folder[title="${await page.evaluate((k) => window.__folder(k), kind)}"]`).click();
    await page.waitForTimeout(300);
  }
  let s = await state();
  check("a page of 500 per folder is loaded", s.total, (n) => n > 1000 && n < 1100);
  check("only the rows in view are drawn", s.drawn, (n) => n > 20 && n < 80);
  check("rows in view have no gaps and sit 31px apart", [s.covered, evenlySpaced(s.tops)], ([a, b]) => a && b);
  await shot("top");

  // Show more is at the end of its folder: scroll there, click, and the folder grows.
  for (let i = 0; i < 12; i++) {
    const more = await page.evaluate(async () => {
      const tree = document.querySelector('[role="tree"]');
      for (let top = 0; top <= tree.scrollHeight; top += tree.clientHeight / 2) {
        tree.scrollTop = top;
        await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
        const more = tree.querySelector(".row.more");
        if (more) return more.click(), true;
      }
      return false;
    });
    if (!more) break;
    await page.waitForTimeout(250);
  }
  s = await state();
  check("Show more loads every page", s.total, (n) => n === 5000 + 6);
  check("still only the rows in view are drawn", s.drawn, (n) => n > 20 && n < 80);

  // Scrolling by wheel draws the rows there.
  await page.evaluate(() => (document.querySelector('[role="tree"]').scrollTop = 0));
  await tree.hover();
  await page.mouse.wheel(0, 40_000);
  await page.waitForTimeout(200);
  s = await state();
  check("wheel scroll: rows drawn at the new place", [s.scrollTop > 30_000, s.covered, evenlySpaced(s.tops)], (v) => v.every(Boolean));
  await shot("scrolled");

  // Click a note there: it opens and is selected.
  const row = page.locator(".row.note").nth(20);
  const title = await row.locator(".name").textContent();
  await row.click();
  await page.waitForFunction((t) => document.querySelector(".note input.title")?.value === t, title);
  s = await state();
  check("click opens the note and selects its row", [s.title, s.selected], ([a, b]) => a === title && b === title);

  // Keyboard: the arrows move the focus row by row and scroll with it.
  await row.focus();
  for (let i = 0; i < 60; i++) await page.keyboard.press("ArrowDown");
  await page.waitForTimeout(150);
  s = await state();
  check("ArrowDown x60: focus is on a row in view", [s.focused !== title && s.focused !== null, s.focusedVisible, s.covered], (v) => v.every(Boolean));
  const afterArrows = s.focused;

  // The focused row stays drawn, and focused, when it is scrolled far out of view.
  await tree.hover();
  await page.mouse.wheel(0, -60_000);
  await page.waitForTimeout(200);
  s = await state();
  check("wheel away: focus stays on its row", [s.focused, s.scrollTop], ([f, top]) => f === afterArrows && top < 31_000);
  await page.keyboard.press("ArrowUp");
  await page.waitForTimeout(200);
  s = await state();
  check("ArrowUp after scrolling away: back at the focused row's neighbour", [s.focused !== null && s.focused !== afterArrows, s.focusedVisible], (v) => v.every(Boolean));

  await page.keyboard.press("End");
  await page.waitForTimeout(200);
  s = await state();
  check("End: the last row, in view", [s.focusedVisible, Math.max(...s.tops) === (s.total - 1) * PITCH], (v) => v.every(Boolean));
  await page.keyboard.press("Home");
  await page.waitForTimeout(200);
  s = await state();
  check("Home: the first row, in view", [s.focusedVisible, s.scrollTop], ([v, top]) => v && top === 0);

  // Opening a note another way (the Recently opened list) scrolls the tree to it.
  await page.evaluate(() => (document.querySelector('[role="tree"]').scrollTop = 0));
  await page.waitForTimeout(100);
  const other = page.locator(".row.note").first();
  const otherTitle = await other.locator(".name").textContent();
  await other.click();
  await page.waitForFunction((t) => document.querySelector(".note input.title")?.value === t, otherTitle);
  await page.locator("section.recent").getByText(title, { exact: true }).first().click();
  await page.waitForFunction((t) => document.querySelector(".note input.title")?.value === t, title);
  await page.waitForTimeout(200);
  s = await state();
  check("opened from Recently opened: the tree shows its row", [s.selected, s.selectedVisible], ([a, b]) => a === title && b);
  await shot("revealed");

  // A context menu opens on a row.
  await page.locator('[role="tree"] .row.note.selected').click({ button: "right" });
  await page.waitForTimeout(150);
  check("right-click: the note menu", await page.locator('[role="menu"]').count(), (n) => n >= 1);
  await page.keyboard.press("Escape");

  // Collapsing folders while scrolled to the end leaves no empty tree: ArrowLeft goes to
  // the parent folder, which is far above and not drawn until then, and then closes it.
  await page.evaluate(() => (document.querySelector('[role="tree"]').scrollTop = 1e7));
  await page.waitForTimeout(150);
  await page.locator(".row.note").last().focus();
  for (let i = 0; i < 4; i++) {
    await page.keyboard.press("ArrowLeft");
    await page.waitForTimeout(150);
  }
  s = await state();
  check("ArrowLeft x4 from the last note: its folders are closed, focus on the top folder", [s.focused, s.focusedVisible, s.total], ([f, v, t]) => f === "small" && v && t === 2505);
  await page.keyboard.press("Home");
  await page.waitForTimeout(150);
  await page.locator('.row.folder[title="medium"]').click();
  await page.waitForTimeout(200);
  s = await state();
  check("collapsed: the four folders show", [s.total, s.drawn, s.covered], ([t, d, c]) => t === 4 && d === 4 && c);
  check("no page errors", errors, (e) => e.length === 0);

  await browser.close();
  let failed = 0;
  for (const r of results) {
    if (!r.ok) failed++;
    console.log(`${r.ok ? "ok  " : "FAIL"} ${r.name}${r.ok ? "" : `: ${JSON.stringify(r.got)}`}`);
  }
  process.exit(failed ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
