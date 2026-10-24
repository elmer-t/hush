#!/usr/bin/env node
// Drives a real Hush window over the Chrome DevTools Protocol (WebView2's remote debugging port).
// Every instance is isolated: own debug port, own WebView2 profile, own scratch vault.
// Run `node hush.mjs help` for the command list.

import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..");
const TARGET_DIR = path.join(REPO, "src-tauri", "target", "verify");
const EXE = path.join(TARGET_DIR, "debug", "hush.exe");
const ROOT = path.join(os.tmpdir(), "hush-verify");
// Hush's shared app config (recent vaults). Tauri resolves it from the Roaming AppData known folder,
// so it cannot be redirected per instance; `stop` removes the scratch vault from it again.
const USER_CONFIG = path.join(process.env.APPDATA ?? "", "com.redheadit.hush", "config.json");
const DEFAULT_PASSPHRASE = "verify-hush passphrase";

const HELP = `usage: node hush.mjs <command> [args] [--run NAME]   (default run: "default")

  build                       build hush.exe (debug, frontend embedded) into src-tauri/target/verify
  launch                      start an isolated instance; waits until the UI is usable
  doctor                      read-only health check of the instance
  seed [--passphrase P]       create the scratch vault, lock it, reload: unlock screen preselects it
  wait <target> [--timeout ms]
  click <target>              real mouse click at the element's centre
  type <target> <text>        click the element, then insert text (works in CodeMirror too)
  press <keys>                e.g. Enter, Escape, Control+N, Control+L, Control+K
  text [target]               print innerText (default: body)
  eval <js>                   evaluate in the page, print the JSON result
  shot <name>                 screenshot to evidence/<name>.png
  note <message>              append a line to evidence/steps.log
  stop                        forget scratch vault, kill this instance, delete instance data; keeps evidence

<target> is a CSS selector, or text=Label for the visible element whose text is exactly Label
(buttons and other controls win over plain text).`;

// ---------- args and run state ----------

const argv = process.argv.slice(2);
function option(name) {
  const i = argv.indexOf(`--${name}`);
  if (i < 0) return undefined;
  const [, value] = argv.splice(i, 2);
  return value;
}
const RUN = option("run") ?? "default";
const RUN_DIR = path.join(ROOT, RUN);
const INSTANCE = path.join(RUN_DIR, "instance");
const EVIDENCE = path.join(RUN_DIR, "evidence");
const STATE = path.join(RUN_DIR, "run.json");
const VAULT = path.join(INSTANCE, "verify.hush");

function readState() {
  if (!fs.existsSync(STATE)) fail(`no instance for run "${RUN}"; run \`launch\` first`);
  return JSON.parse(fs.readFileSync(STATE, "utf8"));
}

function alive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}

function log(line) {
  fs.mkdirSync(EVIDENCE, { recursive: true });
  fs.appendFileSync(path.join(EVIDENCE, "steps.log"), `${new Date().toISOString()} ${line}\n`);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------- CDP ----------

async function targets(port) {
  const res = await fetch(`http://127.0.0.1:${port}/json/list`);
  return (await res.json()).filter((t) => t.type === "page");
}

async function connect(port) {
  const [page] = await targets(port);
  if (!page) fail("no page target on the debug port");
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = () => reject(new Error("cannot open CDP websocket"));
  });
  let next = 0;
  const pending = new Map();
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    const p = pending.get(msg.id);
    if (!p) return;
    pending.delete(msg.id);
    msg.error ? p.reject(new Error(msg.error.message)) : p.resolve(msg.result);
  };
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = ++next;
      pending.set(id, { resolve, reject });
      ws.send(JSON.stringify({ id, method, params }));
    });
  // Await the close: exiting while a socket is closing trips a libuv assertion on Windows.
  const close = () =>
    new Promise((resolve) => {
      if (ws.readyState === WebSocket.CLOSED) return resolve();
      ws.onclose = () => resolve();
      ws.close();
    });
  return { send, close, url: page.url };
}

async function evaluate(cdp, expression) {
  const r = await cdp.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
  return r.result.value;
}

// Runs in the page: resolves a <target> to its visible element.
const FIND = `(target) => {
  const visible = (el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0 && getComputedStyle(el).visibility !== "hidden"; };
  if (!target.startsWith("text=")) return [...document.querySelectorAll(target)].find(visible) ?? null;
  const want = target.slice(5).trim();
  const hits = [...document.querySelectorAll("body *")].filter((el) => visible(el) && el.innerText?.trim().replace(/\\s+/g, " ") === want);
  const control = hits.find((el) => el.closest("button,[role=button],[role=tab],[role=menuitem],[role=radio],[role=treeitem],a,label"));
  const el = control ? control.closest("button,[role=button],[role=tab],[role=menuitem],[role=radio],[role=treeitem],a,label") : hits.at(-1);
  return el ?? null;
}`;

async function locate(cdp, target, timeout = 10_000) {
  const deadline = Date.now() + timeout;
  for (;;) {
    const box = await evaluate(
      cdp,
      `(() => { const el = (${FIND})(${JSON.stringify(target)}); if (!el) return null;
        el.scrollIntoView({ block: "center", inline: "center" });
        const r = el.getBoundingClientRect(); const x = r.x + r.width / 2, y = r.y + r.height / 2;
        const top = document.elementFromPoint(x, y);
        return { x, y, disabled: !!el.disabled, covered: !(top && (el === top || el.contains(top))) }; })()`,
    );
    if (box && !box.disabled && !box.covered) return box;
    if (Date.now() > deadline) {
      const why = !box ? "not found" : box.disabled ? "disabled" : "covered by another element";
      fail(`${target}: ${why} after ${timeout} ms`);
    }
    await sleep(150);
  }
}

async function mouseClick(cdp, { x, y }) {
  await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
  await cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
  await cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
}

const NAMED_KEYS = {
  Enter: [13, "Enter", "\r"],
  Escape: [27, "Escape"],
  Tab: [9, "Tab"],
  Backspace: [8, "Backspace"],
  Delete: [46, "Delete"],
  ArrowUp: [38, "ArrowUp"],
  ArrowDown: [40, "ArrowDown"],
  ArrowLeft: [37, "ArrowLeft"],
  ArrowRight: [39, "ArrowRight"],
  Home: [36, "Home"],
  End: [35, "End"],
  Space: [32, "Space", " "],
};
const MODIFIERS = { Alt: 1, Control: 2, Meta: 4, Shift: 8 };

async function press(cdp, combo) {
  const parts = combo.split("+");
  const name = parts.pop();
  let modifiers = 0;
  for (const m of parts) {
    if (!(m in MODIFIERS)) fail(`unknown modifier ${m}`);
    modifiers |= MODIFIERS[m];
  }
  let vk, code, key, text;
  if (NAMED_KEYS[name]) {
    [vk, code, text] = NAMED_KEYS[name];
    key = name === "Space" ? " " : name;
  } else if (/^[a-z0-9]$/i.test(name)) {
    const upper = name.toUpperCase();
    vk = upper.charCodeAt(0);
    code = /\d/.test(name) ? `Digit${name}` : `Key${upper}`;
    key = modifiers & MODIFIERS.Shift ? upper : name.toLowerCase();
    text = key;
  } else fail(`unknown key ${name}`);
  // Chords with Control/Alt/Meta type nothing.
  if (modifiers & ~MODIFIERS.Shift) text = undefined;
  const base = { modifiers, key, code, windowsVirtualKeyCode: vk };
  // CDP key events do not run editing shortcuts by themselves; name the command explicitly.
  const commands = modifiers === MODIFIERS.Control && key === "a" ? ["selectAll"] : undefined;
  await cdp.send("Input.dispatchKeyEvent", { type: text ? "keyDown" : "rawKeyDown", ...base, text, commands });
  await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
}

// Ready means usable: the unlock card or the workspace header is on screen.
async function waitUsable(cdp, timeout) {
  const deadline = Date.now() + timeout;
  for (;;) {
    const screen = await evaluate(cdp, SCREEN).catch(() => null);
    if (screen && screen.screen !== "loading") return screen;
    if (Date.now() > deadline) fail(`UI not usable after ${timeout} ms`);
    await sleep(250);
  }
}

const SCREEN = `(() => {
  const h2 = document.querySelector("main .card h2")?.textContent.trim();
  if (h2 === "Unlock a vault" || h2 === "Create a new vault") {
    const chosen = document.querySelector('[role=radio][aria-checked=true]')?.getAttribute("title") ?? null;
    return { screen: "unlock", heading: h2, selectedVault: chosen };
  }
  const vault = document.querySelector(".workspace header .vault")?.getAttribute("title");
  if (vault) return { screen: "workspace", vault };
  return { screen: "loading" };
})()`;

const invoke = (cmd, args) =>
  `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args ?? {})})`;

// ---------- commands ----------

async function freePort() {
  return new Promise((resolve) => {
    const srv = net.createServer();
    srv.listen(0, "127.0.0.1", () => {
      const { port } = srv.address();
      srv.close(() => resolve(port));
    });
  });
}

function newestSource() {
  let newest = 0;
  const walk = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else newest = Math.max(newest, fs.statSync(p).mtimeMs);
    }
  };
  for (const d of ["src", path.join("src-tauri", "src")]) walk(path.join(REPO, d));
  for (const f of ["src-tauri/tauri.conf.json", "src-tauri/Cargo.toml", "package.json"])
    newest = Math.max(newest, fs.statSync(path.join(REPO, f)).mtimeMs);
  return newest;
}

const commands = {
  help() {
    console.log(HELP);
  },

  build() {
    // OpenSSL's Configure fails under Git for Windows' perl (missing Locale::Maketext::Simple).
    const perl = execFileSync("where.exe", ["perl"], { encoding: "utf8" }).split(/\r?\n/)[0];
    if (/\\Git\\|\\usr\\bin\\/i.test(perl)) fail(`first perl on PATH is ${perl}; run from PowerShell with Strawberry Perl first (see README)`);
    // A separate target dir: a running `tauri dev` keeps target/debug/hush.exe locked.
    execFileSync("npm run tauri build -- --debug --no-bundle", {
      cwd: REPO,
      stdio: "inherit",
      shell: true,
      env: { ...process.env, CARGO_TARGET_DIR: TARGET_DIR },
    });
    console.log(`built ${EXE}`);
  },

  async launch() {
    if (fs.existsSync(STATE) && alive(readState().pid)) fail(`run "${RUN}" is already running; \`stop\` it or pick another --run`);
    if (!fs.existsSync(EXE)) fail(`${EXE} missing; run \`build\` first`);
    if (fs.statSync(EXE).mtimeMs < newestSource()) console.warn("warning: sources are newer than hush.exe; run `build` to test current code");
    fs.rmSync(INSTANCE, { recursive: true, force: true });
    fs.mkdirSync(path.join(INSTANCE, "webview"), { recursive: true });
    const port = await freePort();
    const out = fs.openSync(path.join(INSTANCE, "app.log"), "a");
    const child = spawn(EXE, [], {
      detached: true,
      stdio: ["ignore", out, out],
      env: {
        ...process.env,
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
        // Own WebView2 profile: instances sharing one profile with different browser args fail to start.
        WEBVIEW2_USER_DATA_FOLDER: path.join(INSTANCE, "webview"),
      },
    });
    child.unref();
    fs.writeFileSync(STATE, JSON.stringify({ pid: child.pid, port, startedAt: new Date().toISOString() }, null, 2));
    const deadline = Date.now() + 30_000;
    for (;;) {
      if (!alive(child.pid)) fail(`hush.exe exited during startup; see ${path.join(INSTANCE, "app.log")}`);
      try {
        if ((await targets(port)).length) break;
      } catch {}
      if (Date.now() > deadline) fail("debug port never answered");
      await sleep(250);
    }
    const cdp = await connect(port);
    const screen = await waitUsable(cdp, 20_000);
    await cdp.close();
    log(`launch pid=${child.pid} port=${port}`);
    console.log(JSON.stringify({ ready: true, pid: child.pid, port, ...screen }));
  },

  async doctor() {
    const s = readState();
    const checks = [];
    const ok = (name, pass, detail = "") => checks.push({ name, pass, detail });
    ok("process alive", alive(s.pid), `pid ${s.pid}`);
    let cdp;
    try {
      cdp = await connect(s.port);
      ok("debug port owned by this run", true, `port ${s.port}`);
    } catch (e) {
      ok("debug port owned by this run", false, e.message);
    }
    if (cdp) {
      // The embedded build is served from tauri.localhost; localhost:1420 means a dev build that needs Vite.
      ok("embedded frontend build", /tauri\.localhost/.test(cdp.url), cdp.url);
      const screen = await evaluate(cdp, SCREEN).catch((e) => ({ screen: `error: ${e.message}` }));
      const vault = screen.vault ?? screen.selectedVault;
      ok("UI usable", screen.screen === "unlock" || screen.screen === "workspace", JSON.stringify(screen));
      ok("only the scratch vault is in use", !vault || path.resolve(vault).toLowerCase() === VAULT.toLowerCase(), vault ?? "none selected");
      await cdp.close();
    }
    ok("isolated WebView2 profile", fs.existsSync(path.join(INSTANCE, "webview", "EBWebView")), path.join(INSTANCE, "webview"));
    ok("build is current", fs.existsSync(EXE) && fs.statSync(EXE).mtimeMs >= newestSource(), EXE);
    for (const c of checks) console.log(`${c.pass ? "ok  " : "FAIL"} ${c.name}${c.detail ? `  (${c.detail})` : ""}`);
    process.exitCode = checks.every((c) => c.pass) ? 0 : 1;
  },

  async seed() {
    const passphrase = option("passphrase") ?? DEFAULT_PASSPHRASE;
    const s = readState();
    if (fs.existsSync(VAULT)) fail(`${VAULT} already exists`);
    const cdp = await connect(s.port);
    // Scaffolding: "Create new…" goes through a native save dialog that CDP cannot reach.
    await evaluate(cdp, invoke("create_vault", { path: VAULT, passphrase }));
    await evaluate(cdp, invoke("lock_vault"));
    await evaluate(cdp, `location.replace("/")`).catch(() => {});
    await sleep(500);
    const page = await reconnect(s.port, cdp);
    const screen = await waitUsable(page, 10_000);
    await page.close();
    log(`seed vault=${VAULT}`);
    console.log(JSON.stringify({ vault: VAULT, passphrase, ...screen }));
  },

  async wait(target) {
    const timeout = Number(option("timeout") ?? 10_000);
    const cdp = await connect(readState().port);
    const deadline = Date.now() + timeout;
    while (!(await evaluate(cdp, `!!(${FIND})(${JSON.stringify(target)})`))) {
      if (Date.now() > deadline) fail(`${target}: not visible after ${timeout} ms`);
      await sleep(150);
    }
    log(`wait ${target}`);
    await cdp.close();
  },

  async click(target) {
    const cdp = await connect(readState().port);
    await mouseClick(cdp, await locate(cdp, target));
    log(`click ${target}`);
    await cdp.close();
  },

  async type(target, text) {
    const cdp = await connect(readState().port);
    const box = await locate(cdp, target);
    // Clicking a field that already has focus would drop a selection the app made (a new note's title).
    const focused = await evaluate(cdp, `(() => { const el = (${FIND})(${JSON.stringify(target)}); return !!el && el.contains(document.activeElement); })()`);
    if (!focused) await mouseClick(cdp, box);
    await cdp.send("Input.insertText", { text });
    log(`type ${target} ${JSON.stringify(target.includes("passphrase") ? "***" : text)}`);
    await cdp.close();
  },

  async press(keys) {
    const cdp = await connect(readState().port);
    await press(cdp, keys);
    log(`press ${keys}`);
    await cdp.close();
  },

  async text(target = "body") {
    const cdp = await connect(readState().port);
    const t = await evaluate(cdp, `(${FIND})(${JSON.stringify(target)})?.innerText ?? null`);
    if (t === null) fail(`${target}: not found`);
    console.log(t);
    await cdp.close();
  },

  async eval(js) {
    const cdp = await connect(readState().port);
    console.log(JSON.stringify(await evaluate(cdp, js), null, 2));
    await cdp.close();
  },

  async shot(name) {
    const cdp = await connect(readState().port);
    const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
    fs.mkdirSync(EVIDENCE, { recursive: true });
    const file = path.join(EVIDENCE, `${name}.png`);
    fs.writeFileSync(file, Buffer.from(data, "base64"));
    log(`shot ${file}`);
    console.log(file);
    await cdp.close();
  },

  note(...words) {
    log(`note ${words.join(" ")}`);
  },

  async stop() {
    if (!fs.existsSync(STATE)) return console.log(`run "${RUN}" has no instance`);
    const s = readState();
    let forgotten = false;
    if (alive(s.pid)) {
      try {
        const cdp = await connect(s.port);
        await evaluate(cdp, invoke("forget_recent_vault", { path: VAULT }));
        forgotten = true;
        await cdp.close();
      } catch {}
      // Kill only the process tree this run started (hush.exe and its WebView2 children).
      try {
        execFileSync("taskkill", ["/PID", String(s.pid), "/T", "/F"], { stdio: "ignore" });
      } catch {}
    }
    if (!forgotten) forgetInConfigFile();
    for (let i = 0; i < 20 && fs.existsSync(INSTANCE); i++) {
      try {
        fs.rmSync(INSTANCE, { recursive: true, force: true });
      } catch {
        await sleep(250); // WebView2 children release their files a moment after the kill.
      }
    }
    fs.rmSync(STATE, { force: true });
    log(`stop pid=${s.pid}`);
    console.log(`stopped; evidence kept in ${EVIDENCE}`);
  },
};

async function reconnect(port, old) {
  await old.close();
  for (let i = 0; i < 40; i++) {
    try {
      return await connect(port);
    } catch {
      await sleep(250);
    }
  }
  fail("lost the page after reload");
}

// Fallback when the app is already gone: drop the scratch vault from the shared recent list.
function forgetInConfigFile() {
  try {
    const config = JSON.parse(fs.readFileSync(USER_CONFIG, "utf8"));
    const before = config.recent_vaults?.length ?? 0;
    config.recent_vaults = (config.recent_vaults ?? []).filter((p) => !p.toLowerCase().startsWith(ROOT.toLowerCase()));
    if (config.recent_vaults.length !== before) fs.writeFileSync(USER_CONFIG, JSON.stringify(config, null, 2));
  } catch {}
}

const [name, ...rest] = argv;
const command = commands[name ?? "help"];
if (!command) fail(`unknown command ${name}\n\n${HELP}`);
await command(...rest);
