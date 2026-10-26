// The graph view's controls, kept for the unlocked session (module state, so they
// survive leaving the view; lock reloads the page, which resets them). Not stored: the
// scope names folders and tags, which are note data.
export type ColorBy = "folder" | "tag" | "topic";

export const graphSettings = $state({
  mode: "local" as "local" | "global",
  depth: 1,
  scopeKind: "folder" as "folder" | "tag" | "search" | "all",
  /** `null`: the open note's folder. */
  scopeFolder: null as string | null,
  scopeTag: "",
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
});
