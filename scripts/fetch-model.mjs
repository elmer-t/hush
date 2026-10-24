// Fetches the bundled embedding model into src-tauri/models/ and checks each file
// against the SHA-256 pinned in src-tauri/models/manifest.json. Files already present
// with the right checksum are left alone. This is the only place Hush's model is
// downloaded: the app itself has no network code and checks the files again on load.
//
//   npm run fetch-model               fail if a file can't be fetched (release builds)
//   npm run fetch-model -- --optional warn instead (dev: Hush runs without the model)

import { createHash } from "node:crypto";
import { readFile, rename, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const dir = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri", "models");
const optional = process.argv.includes("--optional");
const manifest = JSON.parse(await readFile(join(dir, "manifest.json"), "utf8"));

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

let failed = false;
for (const file of manifest.files) {
  const path = join(dir, file.name);
  try {
    if (sha256(await readFile(path)) === file.sha256) {
      console.log(`model: ${file.name} ok`);
      continue;
    }
    console.log(`model: ${file.name} has the wrong checksum, fetching again`);
  } catch {
    console.log(`model: fetching ${file.name}`);
  }
  try {
    const response = await fetch(file.url);
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const bytes = Buffer.from(await response.arrayBuffer());
    const actual = sha256(bytes);
    if (actual !== file.sha256) throw new Error(`checksum mismatch: expected ${file.sha256}, got ${actual}`);
    // Written beside the target and renamed, so a half-written file is never taken for the model.
    await writeFile(`${path}.part`, bytes);
    await rename(`${path}.part`, path);
    console.log(`model: ${file.name} ok`);
  } catch (e) {
    await rm(`${path}.part`, { force: true });
    console.error(`model: could not fetch ${file.name} from ${file.url}: ${e.message}`);
    failed = true;
  }
}

if (failed) {
  if (optional) {
    console.warn("model: continuing without the embedding model; indexing will report it missing.");
  } else {
    process.exit(1);
  }
}
