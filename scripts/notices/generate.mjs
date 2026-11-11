// Regenerates THIRD_PARTY_NOTICES.md: the licences of everything Hush ships.
//
//   npm run notices            write the file
//   npm run notices -- --check exit 1 when the committed file is out of date
//
// Three parts:
// - manual.md, by hand: native libraries, fonts and models, with their licence texts read
//   from the crate sources in ~/.cargo/registry, node_modules or vendored/ (Microsoft's
//   files that come from no crate or package).
// - Rust crates: cargo-about (`cargo install --locked --features cli cargo-about`), with
//   about.toml, for every platform Hush builds for.
// - npm packages: only those in the frontend bundle. A production build runs with a plugin
//   that records which node_modules files end up in the client output (vite.collect.config.js),
//   so the Svelte runtime counts and an unused dependency doesn't. This rewrites build/.
//
// Prints each licence family, and every dependency whose licence is copyleft or unknown.
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..", "..");
const OUT = join(ROOT, "THIRD_PARTY_NOTICES.md");
const CHECK = process.argv.includes("--check");

/** Licences that ask something of a redistributor beyond keeping the notice. */
const COPYLEFT = /\b(A?GPL|LGPL|MPL|EPL|CDDL|EUPL|OSL|CC-BY-SA)/i;
/** Where a package offers a choice, the first of these it offers is used. */
const PREFERRED = ["MIT", "Apache-2.0", "ISC", "BSD-3-Clause", "BSD-2-Clause", "0BSD", "Zlib", "CC0-1.0", "Unlicense"];

const tmp = mkdtempSync(join(tmpdir(), "hush-notices-"));
const flags = [];

function run(cmd, args, env = {}) {
  const r = spawnSync(cmd, args, { cwd: ROOT, encoding: "utf8", maxBuffer: 1 << 28, env: { ...process.env, ...env } });
  if (r.error || r.status !== 0) {
    const hint = cmd === "cargo" && args[0] === "about" ? "\nInstall cargo-about with: cargo install --locked --features cli cargo-about" : "";
    throw new Error(`${cmd} ${args.join(" ")} failed:\n${r.stderr || r.error}${hint}`);
  }
  return r.stdout;
}

/** A licence text as a fenced block; trailing spaces and blank edges trimmed. */
function block(text) {
  const clean = text.replace(/^﻿/, "").replace(/\r\n?/g, "\n").replace(/[ \t]+$/gm, "").replace(/^\n+|\s+$/g, "");
  return `~~~~text\n${clean}\n~~~~`;
}

const read = (path) => readFileSync(path, "utf8");

// ---- Cargo.lock and crate sources ----

const lock = new Map();
for (const m of read(join(ROOT, "src-tauri/Cargo.lock")).matchAll(/\[\[package\]\]\nname = "([^"]+)"\nversion = "([^"]+)"/g)) {
  if (!lock.has(m[1])) lock.set(m[1], m[2]);
}
const metadata = JSON.parse(run("cargo", ["metadata", "--format-version", "1", "--locked", "--manifest-path", "src-tauri/Cargo.toml"]));
/** A locked crate's source folder. */
function crateDir(name) {
  const p = metadata.packages.find((p) => p.name === name && p.version === lock.get(name));
  if (!p) throw new Error(`${name} is not in Cargo.lock`);
  return dirname(p.manifest_path);
}
function locked(name) {
  if (!lock.has(name)) throw new Error(`${name} is not in Cargo.lock`);
  return lock.get(name);
}

// ---- Hand-written section ----

function match(path, re) {
  const m = read(path).match(re);
  if (!m) throw new Error(`no version in ${path}`);
  return m[1];
}
// libsqlite3-sys 0.40+ takes the SQLCipher amalgamation from the sqlcipher-src crate (#164).
const sqlcipherC = () => join(crateDir("sqlcipher-src"), "sqlcipher/sqlcipher.c");
const values = {
  sqlcipher: () => match(sqlcipherC(), /#define CIPHER_VERSION_NUMBER (\S+)/),
  sqlite: () => match(sqlcipherC(), /#define SQLITE_VERSION\s+"([^"]+)"/),
  openssl: () => {
    const v = read(join(crateDir("openssl-src"), "openssl/VERSION.dat"));
    const get = (k) => v.match(new RegExp(`^${k}=(\\d+)`, "m"))[1];
    return `${get("MAJOR")}.${get("MINOR")}.${get("PATCH")}`;
  },
  oniguruma: () => {
    const h = read(join(crateDir("onig_sys"), "oniguruma/src/oniguruma.h"));
    return ["MAJOR", "MINOR", "TEENY"].map((k) => h.match(new RegExp(`ONIGURUMA_VERSION_${k}\\s+(\\d+)`))[1]).join(".");
  },
};
function textOf(ref) {
  const [kind, rest] = ref.split(/:(.*)/s);
  if (kind === "crate") {
    const [name, ...path] = rest.split("/");
    return read(join(crateDir(name), ...path));
  }
  if (kind === "vendored") return read(join(HERE, "vendored", rest));
  if (kind === "repo") return read(join(ROOT, rest));
  throw new Error(`unknown text source ${ref}`);
}
const npmVersion = (name) => JSON.parse(read(join(ROOT, "node_modules", name, "package.json"))).version;

const manual = read(join(HERE, "manual.md")).replace(/\{\{(\w+)(?: ([^}]+))?\}\}/g, (_, what, arg) => {
  if (what === "lock") return locked(arg);
  if (what === "npm") return npmVersion(arg);
  if (what === "text") return block(textOf(arg));
  if (values[what]) return values[what]();
  throw new Error(`unknown placeholder {{${what}}} in manual.md`);
});

// ---- Rust crates ----

const aboutJson = join(tmp, "about.json");
run("cargo", ["about", "generate", "--format", "json", "--locked", "-m", "src-tauri/Cargo.toml", "-c", "scripts/notices/about.toml", "-o", aboutJson]);
const about = JSON.parse(read(aboutJson));
const repoOf = new Map(metadata.packages.map((p) => [`${p.name} ${p.version}`, p.repository]));

/** Licence groups: one per distinct text, with the packages that use it. */
const rust = about.licenses.map((l) => ({
  id: l.id,
  name: l.name,
  text: l.text,
  users: l.used_by.map((u) => `${u.crate.name} ${u.crate.version}`),
}));
for (const g of rust) {
  if (COPYLEFT.test(g.id)) for (const u of g.users) flags.push(`Rust crate ${u}: ${g.id} (source: ${repoOf.get(u) ?? "crates.io"})`);
}
// Crates cargo-about found no licence for.
const licensed = new Set(rust.flatMap((g) => g.users));
for (const c of about.crates) {
  const key = `${c.package.name} ${c.package.version}`;
  if (c.package.publish?.length === 0) continue;
  if (!licensed.has(key)) flags.push(`Rust crate ${key}: no licence found`);
}

// ---- npm packages in the bundle ----

const collected = join(tmp, "modules.tsv");
run("npx", ["vite", "build", "--config", "scripts/notices/vite.collect.config.js", "--logLevel", "error"], { COLLECT_OUT: collected });
const packageDirs = new Set();
for (const line of read(collected).split("\n")) {
  // "client\t<module id>" or "client\tasset\t<source file>", from the browser build only.
  const fields = line.split("\t");
  if (fields[0] !== "client") continue;
  const id = fields[fields.length - 1].replace(/^\0/, "");
  const m = id.match(/^((?:.*\/)?node_modules\/(?:@[^/]+\/)?[^/]+)\//);
  if (m) packageDirs.add(m[1].startsWith("/") ? m[1] : join(ROOT, m[1]));
}

const LICENCE_FILE = /^(licen[cs]e|copying|notice)([-._].*)?$/i;
/** The licence id a package declares, or null. */
function declared(pkg) {
  if (typeof pkg.license === "string") return pkg.license;
  if (pkg.license?.type) return pkg.license.type;
  if (Array.isArray(pkg.licenses)) return pkg.licenses.map((l) => l.type ?? l).join(" OR ");
  return null;
}
/** The licence used from an SPDX expression: the preferred alternative of an OR. */
function chosen(expr) {
  const options = expr.replace(/[()]/g, "").split(/\s+OR\s+/).map((s) => s.trim());
  return PREFERRED.find((p) => options.includes(p)) ?? expr;
}

const npm = new Map();
const npmFlags = [];
for (const dir of [...packageDirs].sort()) {
  const pkg = JSON.parse(read(join(dir, "package.json")));
  const name = `${pkg.name} ${pkg.version}`;
  const files = readdirSync(dir).filter((f) => LICENCE_FILE.test(f) && !/\.(m?js|ts|json)$/.test(f)).sort();
  let id = declared(pkg);
  const text = files.map((f) => read(join(dir, f))).join("\n\n");
  if (!id) {
    // khroma's package.json has no licence field; its licence file is MIT.
    if (/^(The )?MIT License/i.test(text.trim())) id = "MIT";
    else npmFlags.push(`npm package ${name}: no licence declared`);
  }
  id = id ? chosen(id) : "Unknown";
  if (!files.length) npmFlags.push(`npm package ${name}: ${id}, but no licence file in the package`);
  if (COPYLEFT.test(id)) {
    const repo = typeof pkg.repository === "string" ? pkg.repository : pkg.repository?.url;
    npmFlags.push(`npm package ${name}: ${id} (source: ${repo ?? "npm"})`);
  }
  const key = `${id}\n${text.replace(/\s+/g, " ").trim()}`;
  if (!npm.has(key)) npm.set(key, { id, name: id, text: text || `(The package has no licence file; it declares ${id}.)`, users: [] });
  npm.get(key).users.push(name);
}
flags.push(...npmFlags);

// ---- The file ----

function families(groups) {
  const count = new Map();
  for (const g of groups) count.set(g.id, (count.get(g.id) ?? 0) + new Set(g.users).size);
  return count;
}
const rustFamilies = families(rust);
const npmFamilies = families([...npm.values()]);
const ids = [...new Set([...rustFamilies.keys(), ...npmFamilies.keys()])].sort(
  (a, b) => (rustFamilies.get(b) ?? 0) + (npmFamilies.get(b) ?? 0) - ((rustFamilies.get(a) ?? 0) + (npmFamilies.get(a) ?? 0)) || a.localeCompare(b),
);

function section(groups) {
  const order = [...groups].sort((a, b) => a.id.localeCompare(b.id) || a.users[0].localeCompare(b.users[0]));
  return order
    .map((g) => `### ${g.id}\n\nUsed by: ${[...new Set(g.users)].sort().join(", ")}\n\n${block(g.text)}`)
    .join("\n\n");
}

const out = `# Third-party notices

Hush is built on the software listed here. Each part is under its own licence, reproduced below with its copyright notices. This file covers third-party software only, not Hush's own licence.

This file is generated by \`npm run notices\` (scripts/notices/generate.mjs); edit scripts/notices/manual.md, not this file.

## Summary

| Licence | Rust crates | npm packages |
|---|---:|---:|
${ids.map((id) => `| ${id} | ${rustFamilies.get(id) ?? 0} | ${npmFamilies.get(id) ?? 0} |`).join("\n")}

Native libraries, fonts and models are listed in the next section.

The source code of the components under the Mozilla Public License 2.0 and the Eclipse Public License 2.0 is available from the package registries they come from (crates.io and npmjs.com), at the versions named here, and from each project's repository.

${manual.trim()}

## Rust crates

Statically linked into Hush. Licences as found by cargo-about in each crate's source; where a crate offers a choice of licence, the one used is shown.

${section(rust)}

## JavaScript packages

Bundled into Hush's user interface. Only the packages whose code or files end up in the bundle are listed.

${section([...npm.values()])}
`;

rmSync(tmp, { recursive: true, force: true });

console.log("Licence families (Rust crates / npm packages):");
for (const id of ids) console.log(`  ${id}: ${rustFamilies.get(id) ?? 0} / ${npmFamilies.get(id) ?? 0}`);
console.log(flags.length ? `Needs attention (copyleft or unknown):\n${flags.map((f) => `  ${f}`).join("\n")}` : "Nothing copyleft or unknown.");

if (CHECK) {
  const same = existsSync(OUT) && read(OUT) === out;
  console.log(same ? `${relative(ROOT, OUT)} is up to date.` : `${relative(ROOT, OUT)} is out of date: run npm run notices.`);
  process.exit(same ? 0 : 1);
}
writeFileSync(OUT, out);
console.log(`Wrote ${relative(ROOT, OUT)} (${Math.round(out.length / 1024)} KB).`);
