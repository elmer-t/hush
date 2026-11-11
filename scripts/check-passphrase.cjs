// Browser check for the passphrase strength meter on create vault and change passphrase.
// Needs the Vite dev server and Playwright:
//   npx vite dev --port 1420 & node scripts/check-passphrase.cjs [screenshot dir] [base url]
// Create vault runs on the app page with mocked commands; Settings is mounted alone through
// src/lib/checks/settings-harness.svelte.ts. Exits 1 when a check fails.
const { chromium } = require("playwright");
const SHOTS = process.argv[2];
const URL = process.argv[3] || "http://127.0.0.1:1420/";

function mocks() {
  const replies = {
    vault_status: null, recent_vaults: [], "plugin:dialog|save": "/home/me/Notes.hush",
    auto_lock_minutes: 10, default_author: "", trash_days: 30, confirm_trash: true, recent_opened_count: 10,
    index_all_cores: false, model_downloads: [], index_status: { state: "idle", done: 0, total: 0 },
    model_status: { model: "e5-small", readyModel: "e5-small", load: { state: "ready" }, reduced: null },
    tag_list: [], tag_training: [], folders: [],
    gpu_status: { adapters: [], best: null, enabled: false, model: "e5-small", device: "cpu", reason: null, gpuModel: null },
    suggestion_threshold: { model: "e5-small", threshold: null, default: 0.8 },
  };
  replies.model_overview = { status: replies.model_status, model: "e5-small", notes: 0, defaultRate: 1.4, models: [] };
  window.__TAURI_INTERNALS__ = {
    invoke: async (cmd) => (cmd.startsWith("plugin:event|") ? 1 : cmd in replies ? structuredClone(replies[cmd]) : null),
    transformCallback: () => 0,
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
}

async function main() {
  const browser = await chromium.launch();
  const fails = [];
  const check = (name, ok) => { console.log(`${ok ? "ok  " : "FAIL"} ${name}`); if (!ok) fails.push(name); };
  for (const scheme of ["light", "dark"]) {
    const context = await browser.newContext({ colorScheme: scheme, viewport: { width: 760, height: 980 } });
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.addInitScript(mocks);
    await page.goto(URL);
    await page.getByRole("button", { name: "Create new…" }).click();
    const field = page.locator("#new-passphrase");
    await field.waitFor();
    const strength = page.locator("#new-passphrase-strength");
    check(`${scheme} create: guidance shown before typing`, /four or more random words/.test(await strength.textContent()));
    await field.fill("abc");
    check(`${scheme} create: too short`, /Too short/.test(await strength.textContent()));
    await field.fill("Password1");
    check(`${scheme} create: common password warning`, /most common passwords/.test(await strength.textContent()));
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/create-weak-${scheme}.png` });
    await field.fill("velvet orbit pickle tundra");
    check(`${scheme} create: four random words strong`, /Strong/.test(await strength.textContent()));
    check(`${scheme} create: no warning when strong`, (await strength.locator(".warning").count()) === 0);
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/create-strong-${scheme}.png` });

    await page.evaluate(() => import("/src/lib/checks/settings-harness.svelte.ts"));
    errors.length = 0;
    await page.evaluate(() => window.__mountSettings(720));
    const next = page.locator('input[aria-describedby="new-passphrase-strength"]');
    await next.waitFor();
    await next.fill("qwertyuiop");
    const change = page.locator("#new-passphrase-strength");
    check(`${scheme} change: keyboard pattern warning`, /Keyboard patterns/.test(await change.textContent()));
    await next.scrollIntoViewIfNeeded();
    const form = page.locator("form", { has: next });
    if (SHOTS) await form.screenshot({ path: `${SHOTS}/change-weak-${scheme}.png` });
    await next.fill("kq7#mLp2");
    check(`${scheme} change: fair, no warning`, /Fair/.test(await change.textContent()) && (await change.locator(".warning").count()) === 0);
    if (SHOTS) await form.screenshot({ path: `${SHOTS}/change-fair-${scheme}.png` });
    check(`${scheme}: no page errors (${errors.join("; ")})`, errors.length === 0);
    await context.close();
  }
  await browser.close();
  process.exit(fails.length ? 1 : 0);
}
main();
