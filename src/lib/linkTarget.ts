// What the link bar at the bottom of the window shows for a hovered or focused link (#131).

/** The bar's text in parts: `muted` parts (userinfo, a "won't open" note) are drawn dimmer. */
export type Part = { text: string; muted?: boolean };

export type LinkTarget = {
  parts: Part[];
  /** Set when the link text names a different site than the link goes to. */
  warning?: string;
};

/** Shortens `s` to at most `max` characters by cutting out its middle. */
export function truncateMiddle(s: string, max: number): string {
  if (s.length <= max) return s;
  if (max <= 1) return "…";
  const head = Math.ceil((max - 1) / 2);
  return s.slice(0, head) + "…" + s.slice(s.length - (max - 1 - head));
}

function decode(s: string): string {
  try {
    return decodeURI(s);
  } catch {
    return s;
  }
}

/**
 * The parts for an `http(s):` URL: scheme, userinfo (muted), the host in punycode and
 * never cut, then the decoded path, query and hash, cut in the middle to fit `max`.
 */
function webParts(url: URL, max: number): Part[] {
  const scheme = url.protocol + "//";
  const user = url.username ? url.username + (url.password ? ":" + url.password : "") + "@" : "";
  const host = url.host;
  let rest = decode(url.pathname + url.search + url.hash);
  if (rest === "/") rest = "";
  const room = max - scheme.length - host.length;
  // Userinfo only hides the real host, so it is the first thing to give up its room.
  const shownUser = user ? truncateMiddle(decode(user), Math.max(4, Math.floor(room / 3))) : "";
  const left = room - shownUser.length;
  const parts: Part[] = [{ text: scheme }];
  if (shownUser) parts.push({ text: shownUser, muted: true });
  parts.push({ text: host });
  if (rest) {
    // The path keeps at least its first character and the ellipsis, even when the host fills the bar.
    parts.push({ text: truncateMiddle(rest, Math.max(left, Math.min(rest.length, 2))) });
  }
  return parts;
}

/** The host a link text names, if the text reads as a URL or bare domain (`mybank.com/login`). */
function hostInText(text: string): string | undefined {
  const t = text.trim();
  if (!/^(https?:\/\/)?[^\s/@]+\.[^\s/@.]{2,}(\/\S*)?$/i.test(t)) return undefined;
  try {
    const host = new URL(/^https?:\/\//i.test(t) ? t : "https://" + t).hostname;
    // A plain word with a dot ("v1.2") isn't a domain; a domain has a letter in its last label.
    return /\.[^.]*[a-z¡-￿][^.]*$/i.test(host) ? host : undefined;
  } catch {
    return undefined;
  }
}

const bare = (host: string) => host.toLowerCase().replace(/^www\./, "");

/**
 * What the bar shows for a link with this `href` and visible `text`, in at most about
 * `max` characters. Always the real target, never the link text.
 */
export function describeHref(href: string, text: string, max: number): LinkTarget {
  if (/^https?:/i.test(href)) {
    let url: URL;
    try {
      url = new URL(href);
    } catch {
      return { parts: [{ text: truncateMiddle(href, max - 14) }, { text: " (won't open)", muted: true }] };
    }
    const named = hostInText(text);
    const warning =
      named && bare(named) !== bare(url.hostname) ? `Link text says ${bare(named)}` : undefined;
    // The warning sits beside the URL in the same bar, so the URL gives up its room.
    return { parts: webParts(url, warning ? max - warning.length - 3 : max), warning };
  }
  if (/^mailto:/i.test(href)) return { parts: [{ text: truncateMiddle(decode(href), max) }] };
  return { parts: [{ text: truncateMiddle(href, max - 14) }, { text: " (won't open)", muted: true }] };
}

/** What a `[[wikilink]]` resolves to, in the shape the bar needs. */
export type WikiResolution =
  | { kind: "unique"; folder: string; title: string }
  | { kind: "ambiguous"; count: number }
  | { kind: "missing" }
  | { kind: "pending" };

/** What the bar shows for a `[[wikilink]]` to `target`. */
export function describeWikilink(target: string, r: WikiResolution, max: number): LinkTarget {
  const fit = (prefix: string, s: string, suffix = ""): LinkTarget => ({
    parts: [
      { text: prefix, muted: true },
      { text: truncateMiddle(s, max - prefix.length - suffix.length) },
      ...(suffix ? [{ text: suffix, muted: true }] : []),
    ],
  });
  switch (r.kind) {
    case "unique":
      return fit("Note: ", r.folder ? `${r.folder}/${r.title}` : r.title);
    case "ambiguous":
      return fit(`${r.count} notes match `, target);
    case "missing":
      return fit("No note ", target, " (click to create)");
    case "pending":
      return fit("Note: ", target);
  }
}
