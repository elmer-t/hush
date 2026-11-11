// Browser check for the Settings layout: section menu, active section, the model in use,
// the third-party notices under About.
// Needs `npm run dev` (Vite on :1420) and Playwright: `node scripts/check-settings.cjs [screenshot dir]`.
// Mounts Settings alone through src/lib/checks/settings-harness.svelte.ts with mocked commands,
// and exits 1 when a check fails.
const { chromium } = require("playwright");
const URL = process.env.DEV_URL ?? "http://127.0.0.1:1420/";
const SHOTS = process.argv[2];

function mocks() {
  const model = (id, name, extra) => ({
    id, name, licence: "Apache-2.0", licenceUrl: "", dims: 384, maxTokens: 512, bundled: false, size: 90e6,
    host: "huggingface.co", availability: { state: "available", bytes: 90e6, downloaded: true }, rate: null, gpu: false, ...extra,
  });
  const status = { model: "e5-small", readyModel: "e5-small", load: { state: "ready" }, reduced: null };
  const replies = {
    auto_lock_minutes: 10, default_author: "Elmer", trash_days: 30, confirm_trash: true, recent_opened_count: 10,
    index_all_cores: false,
    gpu_status: { adapters: [], best: null, enabled: true, model: "e5-small", device: "cpu", reason: null, gpuModel: "e5-small-fp16" },
    model_status: status, index_status: { state: "idle" }, model_downloads: [],
    model_overview: {
      status, model: "e5-small", notes: 12000, defaultRate: 1.4,
      models: [
        model("e5-small", "multilingual-e5-small (int8)", { bundled: true, availability: { state: "available", bytes: 118e6, downloaded: false }, rate: 1.39 }),
        model("e5-small-fp16", "multilingual-e5-small (fp16, GPU)", { gpu: true }),
        model("bge-m3", "BGE-M3", { availability: { state: "missing", file: "model.onnx" }, size: 560e6, dims: 1024 }),
      ],
    },
    tag_list: [{ tag: "work", notes: 120, description: "Work notes" }, { tag: "journal", notes: 40, description: "" }],
    "plugin:app|version": "0.1.0",
    third_party_notices:
      "# Third-party notices\n\n## Summary\n\n| Licence | Rust crates | npm packages |\n|---|---:|---:|\n| MIT | 391 | 59 |\n| ISC | 23 | 20 |\n| MPL-2.0 | 5 | 0 |\n| OFL-1.1 | 0 | 3 |\n\n## Native libraries, fonts and models\n\n### SQLCipher (community edition)\n\n~~~~text\nCopyright (c) 2008-2020 Zetetic LLC\nAll rights reserved.\n~~~~\n".repeat(3),
    suggestion_threshold: { model: "e5-small", threshold: null, default: 0.8 }, tag_training: [], folders: [], recent_vaults: [],
  };
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
  for (const [scheme, width] of [["light", 1000], ["dark", 1000], ["light", 560]]) {
    const page = await (await browser.newContext({ colorScheme: scheme, viewport: { width: width + 40, height: 800 } })).newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.addInitScript(mocks);
    await page.goto(URL);
    await page.waitForTimeout(1000);
    await page.evaluate(() => import("/src/lib/checks/settings-harness.svelte.ts"));
    // Only errors from Settings count, not from the app page the harness replaces.
    errors.length = 0;
    await page.evaluate((w) => window.__mountSettings(w), width);
    await page.waitForSelector(".in-use");
    const tag = `${scheme} ${width}px`;
    const current = () => page.evaluate(() => document.querySelector('.menu a[aria-current="true"]')?.textContent.trim());
    check(`${tag}: seven menu entries`, (await page.locator(".menu a").count()) === 7);
    check(`${tag}: seven section headers`, (await page.locator(".sections > section > h2").count()) === 7);
    check(`${tag}: General active at the top`, /General/.test(await current()));
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/top-${scheme}-${width}.png` });
    await page.click('.menu a[href="#model-settings"]');
    await page.waitForTimeout(1200);
    const gap = await page.evaluate(() => document.getElementById("model-settings").getBoundingClientRect().top - document.getElementById("scroller").getBoundingClientRect().top);
    check(`${tag}: menu click scrolls to Model (top ${gap.toFixed(0)} px)`, gap >= 0 && gap < 160);
    check(`${tag}: Model active after the click`, /Model/.test(await current()));
    check(`${tag}: menu still visible`, await page.locator(".menu").isVisible() && (await page.locator(".menu").boundingBox()).y >= 0);
    check(`${tag}: one model row marked in use`, (await page.locator('.models li[aria-current="true"]').count()) === 1);
    check(`${tag}: in-use row is e5-small`, /e5-small \(int8\)/.test(await page.locator('.models li[aria-current="true"]').textContent()));
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/model-${scheme}-${width}.png` });
    await page.mouse.move(width / 2, 400);
    await page.mouse.wheel(0, 5000);
    await page.waitForTimeout(600);
    check(`${tag}: wheel to the bottom lights About`, /About/.test(await current()));
    check(`${tag}: About shows the version`, /Hush 0\.1\.0/.test(await page.locator("#settings-about h3").first().textContent()));
    check(`${tag}: notices hidden until asked`, (await page.locator("pre.notices").count()) === 0);
    await page.click("text=Show third-party notices");
    await page.waitForSelector("pre.notices");
    check(`${tag}: licence summary has four rows`, (await page.locator(".families tbody tr").count()) === 4);
    check(`${tag}: notices text shown`, /Zetetic LLC/.test(await page.locator("pre.notices").textContent()));
    check(`${tag}: button says Hide`, (await page.locator('#settings-about button[aria-expanded="true"]').count()) === 1);
    await page.evaluate(() => document.getElementById("settings-about").scrollIntoView());
    await page.waitForTimeout(300);
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/about-${scheme}-${width}.png` });
    await page.mouse.wheel(0, -100000);
    await page.waitForTimeout(600);
    check(`${tag}: wheel back up lights General`, /General/.test(await current()));
    check(`${tag}: no page errors (${errors.join("; ")})`, errors.length === 0);
  }
  await browser.close();
  process.exit(fails.length ? 1 : 0);
}
main();
