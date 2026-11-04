import { findInSegments, nearestMatch } from "$lib/find";

/**
 * Highlights find matches in the rendered preview. It uses the CSS Custom Highlight API,
 * so the `{@html}` DOM and the diagrams aren't touched; where that API is missing (older
 * WebKitGTK), it wraps matches in `<mark>`, which the next render of the note replaces.
 */

const ALL = "hush-find";
const CURRENT = "hush-find-current";

const BLOCKS = new Set([
  "P", "LI", "UL", "OL", "H1", "H2", "H3", "H4", "H5", "H6", "PRE", "BLOCKQUOTE", "TD", "TH",
  "TR", "TABLE", "DIV", "DT", "DD", "HR", "BR", "IMG", "ARTICLE", "SECTION", "DETAILS", "SUMMARY",
]);

/** Whether the CSS Custom Highlight API is there; without it, matches are wrapped in `<mark>`. */
export const liveHighlights = () =>
  typeof CSS !== "undefined" && CSS.highlights != null && typeof Highlight !== "undefined";

/** The block element `node` sits in, below `root`. */
function blockOf(node: Node, root: Element): Node {
  for (let el = node.parentElement; el && el !== root; el = el.parentElement) if (BLOCKS.has(el.tagName)) return el;
  return root;
}

/** The ranges `pattern` matches in the text of `root`, skipping diagrams. */
export function findRanges(root: Element, pattern: RegExp): Range[] {
  const nodes: (Text | null)[] = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT, {
    acceptNode: (node) => {
      if (node instanceof Element) {
        // Diagrams are skipped for now.
        if (node.classList.contains("mermaid-block") || node instanceof SVGElement) return NodeFilter.FILTER_REJECT;
        return BLOCKS.has(node.tagName) ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP;
      }
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  let block: Node | undefined;
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (node instanceof Element) {
      // Entering a block (or a `<br>`, `<img>`) ends whatever came before it.
      if (nodes.length && nodes[nodes.length - 1] !== null) nodes.push(null);
      block = node;
      continue;
    }
    const at = blockOf(node, root);
    if (block !== undefined && at !== block && nodes.length && nodes[nodes.length - 1] !== null) nodes.push(null);
    block = at;
    nodes.push(node as Text);
  }
  const segments = nodes.map((n) => (n ? n.data : null));
  return findInSegments(segments, pattern).map(({ start, end }) => {
    const range = document.createRange();
    range.setStart(nodes[start.segment]!, start.offset);
    range.setEnd(nodes[end.segment]!, end.offset);
    return range;
  });
}

/**
 * Shows `ranges`, with the one at `current` (-1 for none) set apart. Call `clear` first and
 * find the ranges after it: the `<mark>` fallback changes the text nodes.
 */
export function paint(ranges: Range[], current: number) {
  if (liveHighlights()) {
    CSS.highlights.set(ALL, new Highlight(...ranges));
    CSS.highlights.set(CURRENT, new Highlight(...(current >= 0 && ranges[current] ? [ranges[current]] : [])));
    return;
  }
  // Back to front, so wrapping one match doesn't move the offsets of those before it.
  for (let i = ranges.length - 1; i >= 0; i--) wrap(ranges[i], i === current);
}

/** Removes every find highlight. */
export function clear(root: Element | undefined) {
  if (liveHighlights()) {
    CSS.highlights.delete(ALL);
    CSS.highlights.delete(CURRENT);
  } else if (root) {
    unwrap(root);
  }
}

/** Scrolls the nearest scrolling ancestor so `range` is in view, centred when it was not. */
export function reveal(range: Range | undefined) {
  if (!range) return;
  const scroller = scrollerOf(range.startContainer);
  if (!scroller) return;
  const box = scroller.getBoundingClientRect();
  const rect = range.getBoundingClientRect();
  if (rect.top >= box.top && rect.bottom <= box.bottom) return;
  scroller.scrollTop += rect.top - box.top - (box.height - rect.height) / 2;
}

/** The first of `ranges` at or below the top of the visible part of its scroller, or 0. */
export function firstVisible(ranges: Range[]): number {
  if (ranges.length === 0) return -1;
  const scroller = scrollerOf(ranges[0].startContainer);
  const top = scroller ? scroller.getBoundingClientRect().top : 0;
  return nearestMatch(
    ranges.map((r) => r.getBoundingClientRect().top),
    top,
  );
}

function scrollerOf(node: Node): HTMLElement | null {
  for (let el = node.parentElement; el; el = el.parentElement) {
    const { overflowY } = getComputedStyle(el);
    if ((overflowY === "auto" || overflowY === "scroll") && el.scrollHeight > el.clientHeight) return el;
  }
  return null;
}

/** Wraps a range in `<mark>` elements, one per text node it covers. */
function wrap(range: Range, current: boolean) {
  const texts: Text[] = [];
  const { startContainer, endContainer } = range;
  if (startContainer === endContainer) {
    texts.push(startContainer as Text);
  } else {
    const walker = document.createTreeWalker(range.commonAncestorContainer, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) if (range.intersectsNode(n)) texts.push(n as Text);
  }
  for (const text of texts) {
    const from = text === startContainer ? range.startOffset : 0;
    const to = text === endContainer ? range.endOffset : text.length;
    if (from >= to) continue;
    const middle = text.splitText(from);
    middle.splitText(to - from);
    const mark = document.createElement("mark");
    mark.className = current ? "hush-find hush-find-current" : "hush-find";
    middle.replaceWith(mark);
    mark.append(middle);
  }
}

function unwrap(root: Element) {
  for (const mark of root.querySelectorAll("mark.hush-find")) {
    const parent = mark.parentNode!;
    mark.replaceWith(...mark.childNodes);
    parent.normalize();
  }
}
