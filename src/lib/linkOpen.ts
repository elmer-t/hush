import { openUrl } from "@tauri-apps/plugin-opener";

/**
 * What a rendered link points at, read from the element the Live editor drew for it (a link
 * mark, or an `<a>` in a table rendered by `markdown.ts`), and what clicking it does. One set of
 * rules for every place a link is shown, so they all open the same things (#131, #137).
 */

export type Link = { kind: "wiki"; target: string } | { kind: "href"; href: string; text: string };

/**
 * The link an element stands for: a rendered `<a>` (`a.wikilink` with `data-target`, an
 * `href`, or a diagram's SVG `<a>`, which may only carry `xlink:href`) or a Live link mark
 * (`data-target` / `data-href`).
 */
export function linkOfElement(el: Element): Link {
  const target = (el as HTMLElement).dataset?.target;
  if (target && (el.classList.contains("wikilink") || el.classList.contains("cm-live-wikilink"))) {
    return { kind: "wiki", target };
  }
  const href = el.getAttribute("href") ?? el.getAttribute("xlink:href") ?? el.getAttribute("data-href") ?? "";
  return { kind: "href", href, text: el.textContent ?? "" };
}

/** Wikilinks open notes; `http(s):` and `mailto:` open in the system browser; nothing else navigates. */
export function canOpen(link: Link): boolean {
  return link.kind === "wiki" || /^(https?:|mailto:)/i.test(link.href);
}

/**
 * Opens `link`, clicked at viewport position `x`, `y`, if it is one that opens. Returns
 * whether it did. `open` is the system browser, replaceable for tests.
 */
export function openLink(
  link: Link,
  x: number,
  y: number,
  onwikilink: ((target: string, x: number, y: number) => void) | undefined,
  open: (href: string) => unknown = openUrl,
): boolean {
  if (!canOpen(link)) return false;
  if (link.kind === "wiki") {
    if (!onwikilink) return false;
    onwikilink(link.target, x, y);
  } else {
    void open(link.href);
  }
  return true;
}
