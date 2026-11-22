// Browser check for the vault icon (#241): the top bar with and without an icon, the
// picker in Settings (mouse and keyboard), and that nothing of the icon is on the unlock
// screen or in the webview's storage. Both themes.
// Needs `npm run dev` (Vite on :1420, or HUSH_DEV_URL) and Playwright:
//   NODE_PATH=$(npm root -g) node scripts/check-vault-icon.cjs [screenshot dir]
// Runs the app's own page on mocked commands, and exits 1 when a check fails.
const { chromium } = require("playwright");
const URL = process.env.HUSH_DEV_URL || process.env.DEV_URL || "http://127.0.0.1:1420/";
const SHOTS = process.argv[2];

/**
 * Runs in the page before the app. The mocked vault outlives a reload in `window.name`,
 * as the real one does in its file: `{ locked, icon }`. Nothing else is kept.
 */
function backend() {
  const vault = () => JSON.parse(window.name || '{"locked":false,"icon":null}');
  const save = (next) => (window.name = JSON.stringify(next));
  const info = () => ({ name: "work", path: "C:\\vaults\\work.hush", icon: vault().icon });
  const locked = () => {
    if (vault().locked) throw { kind: "locked", message: "vault is locked" };
  };
  const commands = {
    vault_status: () => (vault().locked ? null : info()),
    recent_vaults: () => [
      { name: "work", path: "C:\\vaults\\work.hush", exists: true },
      { name: "home", path: "C:\\vaults\\home.hush", exists: true },
    ],
    open_vault: () => (save({ ...vault(), locked: false }), info()),
    vault_icon: () => (locked(), vault().icon),
    set_vault_icon: ({ icon }) => (locked(), window.__iconWrites.push(icon), save({ ...vault(), icon }), null),
    folders: () => [], folder_sizes: () => [], folder_notes: () => [], sidebar_view: () => "tree", sidebar_folder: () => null,
    sidebar_search_scope: () => "folder", sidebar_search_mode: () => "words", confirm_trash: () => true,
    last_opened_note: () => null, graph_scope: () => null, recent_opened_notes: () => [], recent_opened_count: () => 10,
    recent_opened_collapsed: () => false, auto_lock_minutes: () => 10, trash_count: () => 0, collapsed_sections: () => [],
    index_status: () => ({ state: "idle" }), model_status: () => ({ model: "m", readyModel: "m", load: { state: "ready" }, reduced: null }),
    search_facets: () => ({ tags: [], authors: [], topics: [], attributes: [] }), touch: () => null,
    default_author: () => "", trash_days: () => 30, index_all_cores: () => false, memory_security: () => ({ on: false, active: false }),
    gpu_status: () => ({ adapters: [], best: null, enabled: true, model: "m", device: "cpu", reason: null, gpuModel: "m" }),
    tag_list: () => [], tag_training: () => [], model_downloads: () => [],
    suggestion_threshold: () => ({ model: "m", threshold: null, default: 0.8 }),
    vault_space: () => ({ fileBytes: 1e6, reclaimableBytes: 0, trashNotes: 0, trashTextBytes: 0, neededBytes: 1e6, incremental: true, worthCompacting: false }),
  };
  window.__iconWrites = [];
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

async function main() {
  const browser = await chromium.launch();
  const fails = [];
  const check = (name, ok) => { console.log(`${ok ? "ok  " : "FAIL"} ${name}`); if (!ok) fails.push(name); };
  for (const scheme of ["light", "dark"]) {
    const page = await (await browser.newContext({ colorScheme: scheme, viewport: { width: 1200, height: 900 } })).newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.addInitScript(backend);
    const shot = async (name, target = page) => { if (SHOTS) await target.screenshot({ path: `${SHOTS}/${name}-${scheme}.png` }); };
    const setVault = (vault) => page.evaluate((v) => (window.name = JSON.stringify(v)), vault);
    const bar = page.locator("header .vault");
    const drawn = bar.locator(".vault-icon");
    const mark = bar.locator("img.mark");
    const writes = () => page.evaluate(() => window.__iconWrites);
    const checkedIcon = () => page.locator('.icons [role="radio"][aria-checked="true"]').getAttribute("aria-label");
    /** The resolved value of a colour token, as the browser computes it. */
    const token = (id) => page.evaluate((id) => {
      const probe = document.body.appendChild(document.createElement("i"));
      probe.style.color = `var(--vault-${id})`;
      const color = getComputedStyle(probe).color;
      probe.remove();
      return color;
    }, id);
    const barColor = () => drawn.evaluate((el) => getComputedStyle(el).color);

    await page.goto(URL);
    await bar.waitFor();
    check(`${scheme}: no icon: the top bar shows the Hush mark`, (await mark.count()) === 1 && (await drawn.count()) === 0);
    await shot("bar-none", page.locator("header").first());

    await page.click('button[aria-label="Settings"]');
    const card = page.locator("#settings-general .card").first();
    await card.locator('[role="radiogroup"]').first().waitFor();
    check(`${scheme}: "Vault icon" is the first block of General, above Theme`,
      (await card.locator("h3").textContent()) === "Vault icon" &&
      (await page.locator("#settings-general .card").nth(1).locator("h3").textContent()) === "Theme");
    const icons = page.locator('.icons [role="radio"]');
    const swatches = page.locator('.colors [role="radio"]');
    check(`${scheme}: 27 icon choices and 8 colours (${await icons.count()}, ${await swatches.count()})`, (await icons.count()) === 27 && (await swatches.count()) === 8);
    check(`${scheme}: "No icon" is checked for a vault without one`, (await checkedIcon()) === "No icon");
    check(`${scheme}: colours are off without an icon`, await swatches.first().isDisabled());
    const names = await page.locator('.icons [role="radio"], .colors [role="radio"]').evaluateAll((els) => els.map((e) => e.getAttribute("aria-label")));
    check(`${scheme}: every choice has its own accessible name`, names.every(Boolean) && new Set(names).size === names.length);
    check(`${scheme}: one tab stop per group`, (await page.locator('.icons [tabindex="0"]').count()) === 1);
    await shot("picker-none", card);

    await page.getByRole("radio", { name: "Anchor" }).click();
    check(`${scheme}: a click shows the icon in the top bar at once`, (await drawn.count()) === 1 && (await mark.count()) === 0);
    check(`${scheme}: it is written to the vault with the first colour`, JSON.stringify(await writes()) === '[{"icon":"anchor","color":"blue"}]');
    check(`${scheme}: Anchor is checked`, (await checkedIcon()) === "Anchor");
    check(`${scheme}: the top bar icon is 28 px`, await drawn.evaluate((el) => el.offsetWidth === 28 && el.offsetHeight === 28));
    await page.getByRole("radio", { name: "Green" }).click();
    check(`${scheme}: a colour click recolours the top bar icon`, (await barColor()) === (await token("green")));
    check(`${scheme}: Green is checked`, (await page.locator('.colors [aria-checked="true"]').getAttribute("aria-label")) === "Green");
    await shot("picker-set", card);
    await shot("bar-set", page.locator("header").first());

    // Keyboard: the arrow keys move the choice within a group and apply it.
    await page.getByRole("radio", { name: "Anchor" }).focus();
    await page.keyboard.press("ArrowRight");
    check(`${scheme}: ArrowRight chooses the next icon`, (await checkedIcon()) === "Ship" && (await page.evaluate(() => document.activeElement.getAttribute("aria-label"))) === "Ship");
    check(`${scheme}: and writes it, keeping the colour`, JSON.stringify((await writes()).at(-1)) === '{"icon":"ship","color":"green"}');
    const place = () => page.evaluate(() => { const r = document.activeElement.getBoundingClientRect(); return { x: r.x, y: r.y }; });
    const before = await place();
    await page.keyboard.press("ArrowUp");
    const above = await place();
    check(`${scheme}: ArrowUp chooses the icon one row up`, above.x === before.x && above.y < before.y && (await checkedIcon()) !== "Ship");
    await page.keyboard.press("ArrowDown");
    check(`${scheme}: ArrowDown comes back`, (await checkedIcon()) === "Ship");
    await page.keyboard.press("Tab");
    check(`${scheme}: Tab goes to the checked colour`, (await page.evaluate(() => document.activeElement.getAttribute("aria-label"))) === "Green");
    await page.keyboard.press("ArrowLeft");
    check(`${scheme}: ArrowLeft chooses the colour before`, JSON.stringify((await writes()).at(-1)) === '{"icon":"ship","color":"amber"}' && (await barColor()) === (await token("amber")));
    await page.keyboard.press("Shift+Tab");
    await page.keyboard.press("Home");
    check(`${scheme}: Home chooses "No icon": the Hush mark is back`, (await checkedIcon()) === "No icon" && (await mark.count()) === 1 && (await drawn.count()) === 0 && (await writes()).at(-1) === null);
    await page.keyboard.press("End");
    check(`${scheme}: End chooses the last icon`, (await checkedIcon()) === "Music" && (await drawn.count()) === 1);

    const storage = await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]));
    check(`${scheme}: nothing of the icon is in localStorage or sessionStorage`, !/music|amber|vault_icon|vaultIcon/i.test(storage));

    // Lock: the page reloads on the unlock screen, as the app does on the locked event.
    const unlockScreen = async () => {
      await page.goto(`${URL}?locked=manual`);
      await page.locator(".vault-row").first().waitFor();
      return page.locator("main, body > div").first().innerHTML();
    };
    await setVault({ locked: true, icon: { icon: "music", color: "amber" } });
    const withIcon = await unlockScreen();
    check(`${scheme}: locked: no vault icon anywhere on the unlock screen`, (await page.locator(".vault-icon").count()) === 0);
    check(`${scheme}: locked: every vault row shows the lock icon`, await page.locator(".vault-row").evaluateAll((rows) => rows.length === 2 && rows.every((r) => r.querySelector("button.vault > svg.icon"))));
    await shot("unlock-with-icon");
    await setVault({ locked: true, icon: null });
    const without = await unlockScreen();
    check(`${scheme}: the unlock screen is the same for a vault with and without an icon`, withIcon === without);

    // Unlocked again (a restart looks the same to the page): the icon comes with the vault's info.
    await setVault({ locked: false, icon: { icon: "music", color: "amber" } });
    await page.goto(URL);
    await bar.waitFor();
    check(`${scheme}: after unlock the top bar shows the icon on its first render`, (await drawn.count()) === 1 && (await barColor()) === (await token("amber")));
    check(`${scheme}: without asking for it again`, (await writes()).length === 0);

    // A vault a newer Hush wrote to.
    await setVault({ locked: false, icon: { icon: "submarine", color: "amber" } });
    await page.goto(URL);
    await bar.waitFor();
    check(`${scheme}: an unknown icon draws the Hush mark`, (await mark.count()) === 1 && (await drawn.count()) === 0);
    await page.click('button[aria-label="Settings"]');
    await icons.first().waitFor();
    check(`${scheme}: and the picker checks nothing`, (await page.locator('.icons [aria-checked="true"]').count()) === 0 && (await page.locator('.icons [tabindex="0"]').count()) === 1);

    check(`${scheme}: no page errors (${errors.join("; ")})`, errors.length === 0);
  }
  await browser.close();
  console.log(fails.length ? `\n${fails.length} failed` : "\nall passed");
  process.exit(fails.length ? 1 : 0);
}

main().catch((e) => { console.error(e); process.exit(1); });
