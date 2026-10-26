// The graph view's controls, kept as module state so they survive leaving the view.
// They are stored in two places, by what they hold (see `layout.ts`):
// - the display controls hold no note data, so they go in localStorage per device and are
//   read when this module loads, before the first render;
// - the scope names folders and tags, which are note data, so it goes in the vault and is
//   read after unlock (`restoreGraphScope`).
import { api } from "$lib/api";
import { debounced, KEYS, loadStored, saveStored } from "$lib/layout";

export type ColorBy = "folder" | "tag" | "topic";
type ScopeKind = "folder" | "tag" | "search" | "all";

const display = {
  mode: "local" as "local" | "global",
  depth: 1,
  links: true,
  /** Meaning edges per mode: on in the local graph, off in the global one. */
  meaning: { local: true, global: false },
  tags: false,
  ghosts: false,
  threshold: 0.75,
  cap: 5000,
  colorBy: "folder" as ColorBy,
  /** Labels show on nodes drawn at least this many pixels wide. */
  labelSize: 8,
  repulsion: 2,
  gravity: 1,
  linkStrength: 1,
};
type Display = typeof display;

const ENUMS: Partial<Record<keyof Display, readonly string[]>> = {
  mode: ["local", "global"],
  colorBy: ["folder", "tag", "topic"],
};

/** The stored display controls, each one that is missing or invalid at its default. */
export function loadDisplay(): Display {
  const stored = loadStored<Record<string, unknown>>(
    KEYS.graph,
    {},
    (v): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v),
  );
  const out: Record<string, unknown> = { ...display, meaning: { ...display.meaning } };
  for (const key of Object.keys(display) as (keyof Display)[]) {
    const value = stored[key];
    const fallback = display[key];
    if (key === "meaning") {
      const m = value as Display["meaning"] | undefined;
      if (typeof m?.local === "boolean" && typeof m?.global === "boolean") out.meaning = { local: m.local, global: m.global };
    } else if (typeof value === typeof fallback && (typeof value !== "number" || Number.isFinite(value))) {
      if (!ENUMS[key] || ENUMS[key].includes(value as string)) out[key] = value;
    }
  }
  return out as Display;
}

export const graphSettings = $state({
  ...loadDisplay(),
  scopeKind: "folder" as ScopeKind,
  /** `null`: the open note's folder. */
  scopeFolder: null as string | null,
  scopeTag: "",
});

const SAVE_DELAY_MS = 300;

const saveDisplay = debounced<Display>(SAVE_DELAY_MS, (d) => saveStored(KEYS.graph, d));
// A write after the vault locked fails; it is dropped, as the lock flushes it first.
const saveScope = debounced<{ kind: ScopeKind; folder: string | null; tag: string }>(SAVE_DELAY_MS, (scope) =>
  api.setGraphScope(scope).catch(() => {}),
);
let scopeRestored = false;

/** Reads the scope from the vault, once after unlock. Until then, changes aren't saved. */
export async function restoreGraphScope() {
  try {
    const scope = await api.graphScope();
    if (scope) {
      // The search text isn't stored, so a "current search" scope would have nothing to show.
      graphSettings.scopeKind = scope.kind === "search" ? "folder" : scope.kind;
      graphSettings.scopeFolder = scope.folder;
      graphSettings.scopeTag = scope.tag;
    }
  } catch {
    // Nothing stored that can be read: today's defaults.
  }
  scopeRestored = true;
}

/** Saves the controls as they change. Call during the workspace's initialisation. */
export function persistGraphSettings() {
  $effect(() => {
    const d = Object.fromEntries(Object.keys(display).map((k) => [k, $state.snapshot(graphSettings[k as keyof Display])]));
    saveDisplay.call(d as Display);
  });
  $effect(() => {
    const scope = { kind: graphSettings.scopeKind, folder: graphSettings.scopeFolder, tag: graphSettings.scopeTag };
    if (scopeRestored) saveScope.call(scope);
  });
}

/** Writes pending changes now: before lock, so nothing is lost or written after it. */
export async function flushGraphSettings() {
  saveDisplay.flush();
  await saveScope.flush();
}
