import { syntaxTree } from "@codemirror/language";
import type { EditorState, Extension } from "@codemirror/state";
import { EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";
import type { DocRange } from "$lib/liveDecorations";
import { canOpen, linkOfElement, type Link } from "$lib/linkOpen";

/**
 * Links in the Live editor (#137): clicking opens them as the preview does, and hovering shows
 * the link bar. The link marks come from `liveDecorations.ts` (`data-href` / `data-target`);
 * this only adds event handlers, so the document and its decorations are left as they are.
 */

/** The source range of the link (`[text](url)`, `<url>`, bare URL or `[[wikilink]]`) at `pos`. */
export function linkRangeAt(state: EditorState, pos: number): DocRange | null {
  let url: DocRange | null = null;
  for (let n: ReturnType<typeof syntaxTree>["topNode"] | null = syntaxTree(state).resolveInner(pos, 1); n; n = n.parent) {
    if (n.name === "Link" || n.name === "Autolink" || n.name === "Wikilink") return { from: n.from, to: n.to };
    if (n.name === "Image") return null;
    if (n.name === "URL") url ??= { from: n.from, to: n.to };
  }
  return url;
}

/** Whether the link's source is shown for editing: a selection range touches it, ends included. */
export function linkRevealed(state: EditorState, range: DocRange): boolean {
  return state.selection.ranges.some((r) => r.from <= range.to && r.to >= range.from);
}

export type ClickAction = "open" | "cursor" | "ignore";

/**
 * What a primary-button press on `link` does: open it, leave it to the editor (place the
 * cursor or extend the selection), or nothing at all.
 *
 * Ctrl/Cmd+click opens any link that opens in the preview, and does nothing on one that
 * doesn't. A plain click opens a link that isn't being edited (as in Obsidian), and places the
 * cursor in one whose source is revealed, or one that can't open. Shift+click selects.
 */
export function clickAction(link: Link | null, e: { mod: boolean; shift: boolean; revealed: boolean }): ClickAction {
  if (!link || e.shift) return "cursor";
  if (e.mod) return canOpen(link) ? "open" : "ignore";
  return !e.revealed && canOpen(link) ? "open" : "cursor";
}

/** The link bar, as `LinkBar.svelte` provides it. */
export type LinkBarControl = {
  show(key: Element, link: Link, x?: number, y?: number): void;
  hide(): void;
  /** The element the bar is shown for, if any. */
  shown(): Element | null;
  /** Forgets resolved wikilinks: notes may have been made since. */
  reset(): void;
};

const isMac = typeof navigator !== "undefined" && /Mac/.test(navigator.userAgent);
const LINK = ".cm-live-link, .cm-live-wikilink";
/** On the scroller while the hovered link opens on a plain click, for its pointer cursor. */
const OPENS = "cm-live-link-opens";

function linkElement(node: EventTarget | null): Element | null {
  return node instanceof Element ? node.closest(LINK) : null;
}

/**
 * Clicks and hovers on the Live editor's links. `open` opens a link clicked at `x`, `y`
 * (`openLink` in `linkOpen.ts`); `bar` is the shared link bar.
 */
export function liveLinks(open: (link: Link, x: number, y: number) => void, bar: LinkBarControl): Extension {
  const action = (view: EditorView, el: Element, mod: boolean, shift: boolean) => {
    const range = linkRangeAt(view.state, view.posAtDOM(el, 0));
    return clickAction(linkOfElement(el), { mod, shift, revealed: range ? linkRevealed(view.state, range) : false });
  };
  const isMod = (e: MouseEvent) => (isMac ? e.metaKey : e.ctrlKey);

  const plugin = ViewPlugin.fromClass(
    class {
      /** The link pressed on, opened when the press ends on it as a click. */
      pressed: Element | null = null;
      constructor(readonly view: EditorView) {}
      update(u: ViewUpdate) {
        // The edit may have changed or dropped the hovered link, as a re-render does in Preview.
        if (u.docChanged) {
          bar.hide();
          bar.reset();
        }
      }
      destroy() {
        bar.hide();
        this.view.scrollDOM.classList.remove(OPENS);
      }
    },
    {
      eventHandlers: {
        mousedown(e, view) {
          this.pressed = null;
          const el = e.button === 0 ? linkElement(e.target) : null;
          if (!el) return false;
          const what = action(view, el, isMod(e), e.shiftKey);
          if (what === "cursor") return false;
          // Opening (or ignoring) a link leaves the cursor, the focus and the source as they are.
          e.preventDefault();
          if (what === "open") this.pressed = el;
          return true;
        },
        click(e) {
          const el = this.pressed;
          this.pressed = null;
          if (!el || linkElement(e.target) !== el) return false;
          e.preventDefault();
          open(linkOfElement(el), e.clientX, e.clientY);
          return true;
        },
        mouseover(e, view) {
          const el = linkElement(e.target);
          if (!el) return false;
          // The pointer cursor says a plain click opens it; a revealed link is text.
          view.scrollDOM.classList.toggle(OPENS, action(view, el, false, false) === "open");
          if (el !== bar.shown()) bar.show(el, linkOfElement(el), e.clientX, e.clientY);
          return false;
        },
        mouseout(e, view) {
          if (linkElement(e.relatedTarget) === null) view.scrollDOM.classList.remove(OPENS);
          const shown = bar.shown();
          if (shown && linkElement(e.relatedTarget) !== shown) bar.hide();
          return false;
        },
      },
    },
  );

  const theme = EditorView.theme({
    [`.${OPENS} .cm-live-link, .${OPENS} .cm-live-wikilink`]: { cursor: "pointer" },
  });

  return [plugin, theme];
}
