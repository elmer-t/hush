import DOMPurify from "dompurify";
import type { Mermaid, MermaidConfig } from "mermaid";

/**
 * Mermaid diagrams in the preview. `markdown.ts` turns a ```mermaid fence into a
 * `.mermaid-block` placeholder holding the escaped source; this module replaces each one
 * with the rendered SVG once it is near the viewport. Mermaid itself is loaded with a
 * dynamic import the first time a note has a diagram, so notes without one never fetch it.
 */

const MAX_TEXT = 50_000;
const MAX_EDGES = 1_000;
const CACHE_SIZE = 200;
/** Quiet time after the last keystroke before a changed diagram renders. */
const DEBOUNCE_MS = 300;
/** Updates closer together than this count as typing, and their diagrams wait for `DEBOUNCE_MS`. */
const TYPING_MS = 1_000;

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
const purifier = DOMPurify(window);

function token(style: CSSStyleDeclaration, name: string): string {
  return style.getPropertyValue(name).trim();
}

/** The base theme filled in from the app's CSS tokens, read now, so a restyle changes diagrams too. */
export function readTheme(): Theme {
  const style = getComputedStyle(document.documentElement);
  const font = token(style, "--sans");
  const variables: Record<string, string | boolean> = {
    darkMode: window.matchMedia("(prefers-color-scheme: dark)").matches,
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

function scroller(el: HTMLElement): HTMLElement | null {
  for (let p = el.parentElement; p; p = p.parentElement) {
    const { overflowY } = getComputedStyle(p);
    if (overflowY === "auto" || overflowY === "scroll") return p;
  }
  return null;
}

/**
 * Keeps the diagrams in `root` rendered. Call `update()` after every change to its HTML: cached
 * diagrams are put back at once, the rest render one at a time once near the viewport.
 */
export function diagrams(root: HTMLElement) {
  const sources = new WeakMap<HTMLElement, string>();
  const timers = new Set<ReturnType<typeof setTimeout>>();
  let theme: Theme | null = null;
  let lastUpdate = 0;
  let notBefore = 0;
  let observer: IntersectionObserver | null = null;
  /** What each block showed before the last `{@html}` update, by position in the note. */
  let previous: (Node[] | null)[] = [];

  function watch(block: HTMLElement) {
    observer ??= new IntersectionObserver(
      (entries) => {
        for (const entry of entries) if (entry.isIntersecting) request(entry.target as HTMLElement);
      },
      { root: scroller(root), rootMargin: "600px 0px" },
    );
    observer.unobserve(block);
    observer.observe(block);
  }

  function request(block: HTMLElement) {
    const wait = notBefore - Date.now();
    if (wait > 0) {
      const timer = setTimeout(() => {
        timers.delete(timer);
        request(block);
      }, wait);
      timers.add(timer);
      return;
    }
    // Replaced by a newer update (typing), or already rendered for this theme.
    if (!block.isConnected || block.dataset.state !== "pending" || !theme) return;
    const source = sources.get(block)!;
    void render(source, theme).then(() => fillFromCache());
  }

  /** Puts finished renders into every block still waiting for that source and theme. */
  function fillFromCache() {
    if (!theme) return;
    for (const block of root.querySelectorAll<HTMLElement>('.mermaid-block[data-state="pending"]')) {
      const source = sources.get(block)!;
      const hit = cached(`${theme.key}\n${source}`);
      if (hit) {
        observer?.unobserve(block);
        show(block, source, hit);
      }
    }
  }

  function refresh(blocks: Iterable<HTMLElement>) {
    for (const block of blocks) {
      block.dataset.state = "pending";
      watch(block);
    }
    fillFromCache();
  }

  const scheme = window.matchMedia("(prefers-color-scheme: dark)");
  function onScheme() {
    const next = readTheme();
    if (next.key === theme?.key) return;
    theme = next;
    // Old diagrams stay up until their new rendering is ready.
    refresh(root.querySelectorAll<HTMLElement>(".mermaid-block"));
  }
  scheme.addEventListener("change", onScheme);

  return {
    /** Call before the HTML is replaced, so a diagram being edited keeps showing its last rendering. */
    snapshot() {
      previous = [...root.querySelectorAll<HTMLElement>(".mermaid-block")].map((block) =>
        block.dataset.state === "pending" && block.dataset.stale === undefined ? null : [...block.childNodes],
      );
    },
    update() {
      const blocks = [...root.querySelectorAll<HTMLElement>(".mermaid-block:not([data-state])")];
      const now = Date.now();
      const typing = now - lastUpdate < TYPING_MS;
      if (typing) notBefore = now + DEBOUNCE_MS;
      lastUpdate = now;
      const stale = typing ? previous : [];
      previous = [];
      if (blocks.length === 0) return;
      theme = readTheme();
      blocks.forEach((block, i) => {
        sources.set(block, block.textContent ?? "");
        const nodes = stale[i];
        if (nodes) {
          block.replaceChildren(...nodes);
          block.dataset.stale = "";
        }
      });
      refresh(blocks);
    },
    destroy() {
      scheme.removeEventListener("change", onScheme);
      observer?.disconnect();
      for (const timer of timers) clearTimeout(timer);
    },
  };
}
