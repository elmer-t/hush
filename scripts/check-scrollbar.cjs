// Browser check for the editor's own scrollbar (#243): in a long wrapped note that was never
// scrolled, the handle stays under the mouse during a drag and the bottom of the track is
// the end of the note; the handle follows wheel, keys and find; a short note has no handle.
// Source and Live, both themes.
// Needs `npm run dev` (Vite on :1420, or HUSH_DEV_URL) and Playwright:
//   NODE_PATH=$(npm root -g) node scripts/check-scrollbar.cjs [screenshot dir]
// Mounts one editor through src/lib/checks/scrollbar-harness.svelte.ts, and exits 1 when a check fails.
const { chromium } = require("playwright");
const URL = process.env.HUSH_DEV_URL || process.env.DEV_URL || "http://127.0.0.1:1420/";
const SHOTS = process.argv[2];
const PARAGRAPHS = 400;

/** Paragraphs of the same length, so paragraph `n` of `PARAGRAPHS` starts about `n / PARAGRAPHS` down the note. */
function note(paragraphs) {
  const words = "the of and a to in is you that it he was for on are as with his they at be this have from or one had by word but not what all were we when your can said there use an each which she do how their if will up other about".split(" ");
  let seed = 11;
  const rnd = () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 2 ** 32);
  const out = [];
  for (let p = 0; p < paragraphs; p++) {
    let text = `P${String(p).padStart(3, "0")}`;
    while (text.length < 600) text += ` ${words[(rnd() * words.length) | 0]}`;
    out.push(text.slice(0, 600) + ".", "");
  }
  out.push("END-OF-NOTE");
  return out.join("\n");
}

/** Runs in the page: the scroller, the handle and what is shown. */
function state() {
  const scroller = document.querySelector(".cm-scroller");
  const bar = document.querySelector(".editor > .bar");
  const handle = bar.querySelector(".handle");
  const box = scroller.getBoundingClientRect();
  const first = [...scroller.querySelectorAll(".cm-line")].find((l) => l.textContent && l.getBoundingClientRect().bottom > box.top + 1);
  const rect = (el) => { const r = el.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height }; };
  const lines = [...scroller.querySelectorAll(".cm-line")];
  const last = lines[lines.length - 1].getBoundingClientRect();
  return {
    scrollTop: scroller.scrollTop,
    range: scroller.scrollHeight - scroller.clientHeight,
    scrollHeight: scroller.scrollHeight,
    barShown: getComputedStyle(bar).display !== "none",
    bar: rect(bar),
    handle: handle ? rect(handle) : null,
    first: first?.textContent.slice(0, 4) ?? "",
    endShown: lines[lines.length - 1].textContent === "END-OF-NOTE" && last.bottom <= box.bottom + 1 && last.top >= box.top,
    native: getComputedStyle(scroller).scrollbarWidth,
    padding: getComputedStyle(scroller).paddingRight,
  };
}

async function main() {
  const browser = await chromium.launch();
  const fails = [];
  const check = (name, ok, detail = "") => { console.log(`${ok ? "ok  " : "FAIL"} ${name}${ok || !detail ? "" : `  (${detail})`}`); if (!ok) fails.push(name); };
  const long = note(PARAGRAPHS);
  for (const scheme of ["light", "dark"]) {
    for (const mode of ["source", "live"]) {
      const tag = `${scheme} ${mode}`;
      const page = await (await browser.newContext({ colorScheme: scheme, viewport: { width: 800, height: 700 } })).newPage();
      const errors = [];
      page.on("pageerror", (e) => errors.push(e.message));
      await page.addInitScript(() => {
        window.__TAURI_INTERNALS__ = { invoke: async () => null, transformCallback: () => 0 };
        window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
      });
      await page.goto(URL);
      await page.waitForTimeout(800);
      await page.evaluate(() => import("/src/lib/checks/scrollbar-harness.svelte.ts"));
      errors.length = 0;
      const frames = (n = 3) => page.evaluate(async (n) => { for (let i = 0; i < n; i++) await new Promise((r) => requestAnimationFrame(r)); }, n);
      const mount = async (doc) => {
        await page.evaluate(([doc, mode]) => (window.__editor = window.__mountEditor(doc, mode)), [doc, mode]);
        await page.locator(".cm-content .cm-line").first().waitFor();
        await frames(4);
      };
      const look = () => page.evaluate(state);
      /** Where the handle should be for the scroll position: the scrolled share of the free track. */
      const follows = (s) => Math.abs(s.handle.y - s.bar.y - (s.scrollTop / s.range) * (s.bar.height - s.handle.height)) <= 1.5;

      // A note that fits.
      await mount("# Short\n\nOne line.");
      let s = await look();
      check(`${tag}: a short note shows no handle and leaves no room for one`, !s.barShown && !s.handle && s.padding === "0px", JSON.stringify(s));
      check(`${tag}: the native scrollbar is hidden`, s.native === "none");

      // A long note, never scrolled: one drag from top to bottom.
      await mount(long);
      const fresh = await look();
      check(`${tag}: a long note shows the handle at the top`, fresh.barShown && fresh.handle && Math.abs(fresh.handle.y - fresh.bar.y) < 1 && fresh.padding === "14px", JSON.stringify(fresh));
      if (SHOTS) await page.screenshot({ path: `${SHOTS}/scrollbar-${scheme}-${mode}.png` });
      const x = fresh.handle.x + fresh.handle.width / 2;
      const grab = 10;
      await page.mouse.move(x, fresh.handle.y + grab);
      await page.mouse.down();
      const bottom = fresh.bar.y + fresh.bar.height;
      const travel = bottom - fresh.handle.height - fresh.bar.y;
      let worst = 0;
      let behind = 0;
      const STEPS = 24;
      for (let i = 1; i <= STEPS; i++) {
        const y = fresh.handle.y + grab + (travel * i) / STEPS;
        await page.mouse.move(x, y);
        await frames(3);
        s = await look();
        worst = Math.max(worst, Math.abs(s.handle.y + grab - y));
        // What the native handle would do: sit at the scrolled share of a range that keeps growing.
        behind = Math.max(behind, Math.abs(s.scrollTop / s.range - i / STEPS));
      }
      check(`${tag}: the handle stays under the mouse through the whole drag (worst ${worst.toFixed(1)} px)`, worst <= 2);
      check(`${tag}: the note is at the dragged fraction at every step (worst ${(behind * 100).toFixed(2)}%)`, behind <= 0.005);
      // Past the end of the track, as a hand does.
      await page.mouse.move(x, bottom + 60);
      await frames(4);
      await page.mouse.up();
      await frames(4);
      s = await look();
      check(`${tag}: released at the bottom, the end of the note is shown`, s.endShown && s.range - s.scrollTop <= 1, `scrollTop ${s.scrollTop} of ${s.range}`);
      check(`${tag}: the handle is at the bottom of the track`, Math.abs(s.handle.y + s.handle.height - bottom) <= 1.5);
      check(`${tag}: the note grew while it was dragged through (${fresh.scrollHeight} to ${s.scrollHeight} px), which is what made the native handle lag`, s.scrollHeight > fresh.scrollHeight * 1.03);

      // And back up in one motion.
      await page.mouse.move(x, s.handle.y + grab);
      await page.mouse.down();
      await page.mouse.move(x, fresh.bar.y - 40, { steps: 12 });
      await page.mouse.up();
      await frames(4);
      s = await look();
      check(`${tag}: dragged back up, the note is at its start`, s.scrollTop === 0 && s.first === "P000", JSON.stringify({ top: s.scrollTop, first: s.first }));

      // A fresh note again: the handle held at a fraction shows the text at that fraction.
      await mount(long);
      s = await look();
      await page.mouse.move(x, s.handle.y + grab);
      await page.mouse.down();
      for (const fraction of [0.25, 0.5, 0.75]) {
        const free = s.bar.height - s.handle.height;
        await page.mouse.move(x, s.bar.y + grab + free * fraction, { steps: 4 });
        await frames(6);
        s = await look();
        const paragraph = Number(s.first.slice(1));
        check(`${tag}: handle at ${fraction * 100}%: the first line shown is paragraph ${paragraph} of ${PARAGRAPHS}`, Math.abs(paragraph / PARAGRAPHS - fraction) <= 0.03);
      }
      await page.mouse.up();
      await frames(3);

      // The handle follows scrolling from any other source.
      const middle = { x: fresh.bar.x - 300, y: fresh.bar.y + 300 };
      await page.mouse.move(middle.x, middle.y);
      await page.mouse.wheel(0, 2400);
      await page.waitForTimeout(400);
      let before = s;
      s = await look();
      check(`${tag}: the wheel scrolls the note and the handle follows`, s.scrollTop > before.scrollTop + 1000 && follows(s), `${before.scrollTop} to ${s.scrollTop}`);
      await page.mouse.click(middle.x, middle.y);
      before = await look();
      await page.keyboard.press("PageDown");
      await page.waitForTimeout(300);
      s = await look();
      check(`${tag}: PageDown moves the note and the handle together`, s.scrollTop > before.scrollTop + 200 && follows(s));
      await page.keyboard.press("PageUp");
      await page.keyboard.press("PageUp");
      await page.waitForTimeout(300);
      s = await look();
      check(`${tag}: PageUp too`, s.scrollTop < before.scrollTop && follows(s));
      await page.keyboard.press("Control+End");
      await page.waitForTimeout(400);
      s = await look();
      check(`${tag}: Ctrl+End: the end of the note, handle at the bottom`, s.endShown && follows(s) && Math.abs(s.handle.y + s.handle.height - bottom) <= 1.5);
      await page.keyboard.press("Control+Home");
      await page.waitForTimeout(400);
      s = await look();
      // The editor stops when the first line is in view, a few pixels below the note's top padding.
      check(`${tag}: Ctrl+Home: the start, handle at the top`, s.scrollTop < 40 && s.first === "P000" && follows(s) && Math.abs(s.handle.y - s.bar.y) < 1, JSON.stringify({ top: s.scrollTop, handle: s.handle.y, bar: s.bar.y }));
      await page.evaluate(() => window.__editor.find("END-OF-NOTE"));
      await page.waitForTimeout(400);
      s = await look();
      check(`${tag}: a find match at the end scrolls there and the handle follows`, s.scrollTop > s.range * 0.9 && follows(s), `${s.scrollTop} of ${s.range}`);
      await page.evaluate(() => window.__editor.find("P200"));
      await page.waitForTimeout(400);
      s = await look();
      check(`${tag}: and to a match in the middle`, s.scrollTop > s.range * 0.3 && s.scrollTop < s.range * 0.7 && follows(s), `${s.scrollTop} of ${s.range}`);

      // A click on the track pages.
      before = s;
      await page.mouse.click(x, s.handle.y + s.handle.height + 40);
      await frames(4);
      s = await look();
      check(`${tag}: a click on the track below the handle pages down`, Math.abs(s.scrollTop - before.scrollTop - 600 * 0.875) <= 2 && follows(s), `${s.scrollTop - before.scrollTop} px`);
      await page.mouse.click(x, s.handle.y - 40);
      await frames(4);
      const up = await look();
      check(`${tag}: and above it pages up`, Math.abs(s.scrollTop - up.scrollTop - 600 * 0.875) <= 2 && follows(up), `${s.scrollTop - up.scrollTop} px`);

      // Typing still works with the bar there, and a note that shrinks loses the handle.
      await page.mouse.click(middle.x, middle.y);
      await page.keyboard.press("Control+a");
      await page.keyboard.type("short again");
      await frames(6);
      s = await look();
      check(`${tag}: a note that comes to fit loses the handle`, !s.barShown && s.padding === "0px");

      check(`${tag}: no page errors (${errors.join("; ")})`, errors.length === 0);
      await page.context().close();
    }
  }
  await browser.close();
  console.log(fails.length ? `\n${fails.length} failed` : "\nall passed");
  process.exit(fails.length ? 1 : 0);
}

main().catch((e) => { console.error(e); process.exit(1); });
