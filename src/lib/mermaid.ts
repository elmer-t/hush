import DOMPurify from "dompurify";
import type { Mermaid, MermaidConfig } from "mermaid";
import { currentScheme } from "$lib/theme";

/**
 * Mermaid diagrams: rendering, sanitising and caching, for the Live editor's widgets
 * (`liveMermaid.ts`). Mermaid itself is loaded with a dynamic import the first time a note has
 * a diagram, so notes without one never fetch it.
 */

const MAX_TEXT = 50_000;
const MAX_EDGES = 1_000;
const CACHE_SIZE = 200;
/** Quiet time after the last keystroke before a changed diagram renders. */
const DEBOUNCE_MS = 300;

/**
 * Keys a `%%{init: …}%%` directive in a note may not change. Mermaid's own list covers the
 * security level and limits; the rest keep a note from restyling the app's diagrams or
 * switching label rendering.
 */
const SECURE = [
  "secure",
  "securityLevel",
  "startOnLoad",
  "maxTextSize",
  "maxEdges",
  "suppressErrorRendering",
  "theme",
  "themeVariables",
  "themeCSS",
  "darkMode",
  "fontFamily",
  "altFontFamily",
  "htmlLabels",
  "dompurifyConfig",
  "arrowMarkerAbsolute",
  "logLevel",
  "legacyMathML",
  "forceLegacyMathML",
];

type Result = { svg: string } | { error: string };
type Theme = { key: string; variables: Record<string, string | boolean>; font: string };

const cache = new Map<string, Result>();
const inflight = new Map<string, Promise<Result>>();
let loading: Promise<Mermaid> | null = null;
let configured = "";
/** Mermaid's render isn't safe to run concurrently: every render waits for the one before it. */
let queue: Promise<unknown> = Promise.resolve();
let renders = 0;

// A purifier of our own: mermaid adds hooks to the shared DOMPurify instance while it renders.
// Made on first use, so the module can be imported where there is no `window` (the unit tests).
let purifier: ReturnType<typeof DOMPurify> | null = null;

function token(style: CSSStyleDeclaration, name: string): string {
  return style.getPropertyValue(name).trim();
}

/** The base theme filled in from the app's CSS tokens, read now, so a restyle changes diagrams too. */
export function readTheme(): Theme {
  const style = getComputedStyle(document.documentElement);
  const font = token(style, "--sans");
  const variables: Record<string, string | boolean> = {
    darkMode: currentScheme() === "dark",
    background: token(style, "--surface"),
    fontFamily: font,
    fontSize: "14px",
    textColor: token(style, "--text"),
    lineColor: token(style, "--text-2"),
    primaryColor: token(style, "--accent-soft"),
    primaryTextColor: token(style, "--accent-soft-text"),
    primaryBorderColor: token(style, "--accent"),
    secondaryColor: token(style, "--panel"),
    secondaryTextColor: token(style, "--text"),
    secondaryBorderColor: token(style, "--border-strong"),
    tertiaryColor: token(style, "--code"),
    tertiaryTextColor: token(style, "--text"),
    tertiaryBorderColor: token(style, "--border"),
    noteBkgColor: token(style, "--warn-soft"),
    noteTextColor: token(style, "--text"),
    noteBorderColor: token(style, "--warn"),
    errorBkgColor: token(style, "--danger-soft"),
    errorTextColor: token(style, "--danger"),
  };
  // A missing token would make mermaid's colour maths throw; its own default is used instead.
  for (const [k, v] of Object.entries(variables)) if (v === "") delete variables[k];
  return { key: JSON.stringify(variables), variables, font };
}

function config(theme: Theme): MermaidConfig {
  return {
    startOnLoad: false,
    securityLevel: "strict",
    suppressErrorRendering: true,
    maxTextSize: MAX_TEXT,
    maxEdges: MAX_EDGES,
    secure: SECURE,
    theme: "base",
    themeVariables: theme.variables,
    fontFamily: theme.font,
    logLevel: "fatal",
  };
}

function load(): Promise<Mermaid> {
  loading ??= import("mermaid").then((m) => m.default);
  return loading;
}

/**
 * Mermaid's SVG keeps its own `<style>` (scoped to the diagram's id) and inline styles, so it
 * gets an SVG config of its own instead of the markdown one. Event handlers and script URLs
 * still go; labels in `foreignObject` stay.
 */
function sanitizeSvg(svg: string): string {
  purifier ??= DOMPurify(window);
  return purifier.sanitize(svg, {
    USE_PROFILES: { svg: true, svgFilters: true, html: true },
    ADD_TAGS: ["style", "foreignObject"],
    // HTML labels live in <foreignObject>, which DOMPurify doesn't treat as an HTML island by default.
    HTML_INTEGRATION_POINTS: { foreignobject: true },
    FORBID_TAGS: ["script", "iframe", "object", "embed", "form", "input", "button", "textarea", "select"],
  });
}

function remember(key: string, result: Result) {
  cache.delete(key);
  cache.set(key, result);
  if (cache.size > CACHE_SIZE) cache.delete(cache.keys().next().value!);
}

function cached(key: string): Result | undefined {
  const hit = cache.get(key);
  if (hit) remember(key, hit);
  return hit;
}

function message(e: unknown): string {
  const text = e instanceof Error ? e.message : typeof e === "object" && e && "str" in e ? String(e.str) : String(e);
  // Mermaid's limit errors go on to tell the developer how to raise the limit.
  return text.split("\n\nInitialize mermaid")[0] || "The diagram could not be rendered.";
}

function render(source: string, theme: Theme): Promise<Result> {
  const key = `${theme.key}\n${source}`;
  const pending = inflight.get(key);
  if (pending) return pending;
  const run = queue.then(async (): Promise<Result> => {
    if (source.length > MAX_TEXT) {
      return { error: `The diagram has ${source.length.toLocaleString()} characters; the limit is ${MAX_TEXT.toLocaleString()}.` };
    }
    // Unique per render, so SVG ids and the styles scoped to them never collide in a note.
    const id = `hush-mermaid-${++renders}`;
    try {
      const mermaid = await load();
      if (configured !== theme.key) {
        mermaid.initialize(config(theme));
        configured = theme.key;
      }
      const { svg } = await mermaid.render(id, source);
      return { svg: sanitizeSvg(svg) };
    } catch (e) {
      return { error: message(e) };
    } finally {
      // Mermaid renders in temporary nodes on <body> and leaves them behind when it fails.
      for (const prefix of ["", "d", "i"]) document.getElementById(`${prefix}${id}`)?.remove();
    }
  });
  queue = run;
  inflight.set(key, run);
  void run.then((result) => {
    inflight.delete(key);
    remember(key, result);
  });
  return run;
}

function show(block: HTMLElement, source: string, result: Result) {
  delete block.dataset.stale;
  if ("svg" in result) {
    block.dataset.state = "done";
    block.innerHTML = result.svg;
    return;
  }
  block.dataset.state = "error";
  const box = document.createElement("div");
  box.className = "mermaid-error";
  box.setAttribute("role", "note");
  const title = document.createElement("strong");
  title.textContent = "Diagram error";
  const text = document.createElement("p");
  text.textContent = result.error;
  const pre = document.createElement("pre");
  const code = document.createElement("code");
  code.textContent = source;
  pre.append(code);
  box.append(title, text, pre);
  block.replaceChildren(box);
}

/**
 * One diagram in one element, for the Live editor (#136). `set` shows a cached rendering at
 * once; otherwise the element keeps its last rendering, faded, (or shows the source, muted)
 * until the new one is ready. `typing` waits for `DEBOUNCE_MS` of quiet first. `onShow` runs after each change of
 * content, so the editor can measure the new height.
 */
export function diagram(block: HTMLElement, onShow: () => void = () => {}) {
  let wanted = "";
  let timer: ReturnType<typeof setTimeout> | undefined;

  function put(source: string, result: Result) {
    show(block, source, result);
    onShow();
  }

  return {
    set(source: string, typing = false) {
      const theme = readTheme();
      const key = `${theme.key}\n${source}`;
      if (key === wanted) return;
      wanted = key;
      clearTimeout(timer);
      const hit = cached(key);
      if (hit) return put(source, hit);
      // A rendering, or one already faded by an edit that is still waiting: it stays up.
      if (block.dataset.state === "done" || block.dataset.state === "error" || block.dataset.stale !== undefined) {
        block.dataset.stale = "";
      } else {
        const pre = document.createElement("pre");
        pre.className = "mermaid-source";
        const code = document.createElement("code");
        code.textContent = source;
        pre.append(code);
        block.replaceChildren(pre);
        onShow();
      }
      block.dataset.state = "pending";
      const run = () =>
        void render(source, theme).then((result) => {
          if (wanted === key) put(source, result);
        });
      if (typing) timer = setTimeout(run, DEBOUNCE_MS);
      else run();
    },
    destroy() {
      clearTimeout(timer);
      wanted = "";
    },
  };
}
