<script lang="ts">
  import { onMount } from "svelte";
  import Graph from "graphology";
  import Sigma from "sigma";
  import { EdgeArrowProgram, EdgeRectangleProgram } from "sigma/rendering";
  import { api, errorMessage, type Graph as GraphData, type GraphScope, type GraphScopeNotes } from "$lib/api";
  import { currentSearch } from "$lib/currentSearch.svelte";
  import { colorOf, NONE } from "$lib/graph/colors";
  import { Layout } from "$lib/graph/layout";
  import { graphSettings as s } from "$lib/graph/settings.svelte";
  import Icon from "$lib/Icon.svelte";

  type Props = {
    /** The open note, the local graph's centre. */
    centre: { id: number; folder: string } | null;
    /** Makes a note the open one; the view stays on the graph. */
    onopen: (id: number) => void;
    /** Opens a note in the editor. */
    onedit: (id: number) => void;
  };
  let { centre, onopen, onedit }: Props = $props();

  // Meaning edges per note, at most.
  const NEIGHBOURS = 5;
  // Layout iterations after a load; Resume runs this many more.
  const ITERATIONS = 400;
  // Above this many edges, hovering edges is off: picking them costs every frame.
  const EDGE_EVENTS_MAX = 5000;

  let container = $state<HTMLDivElement>();
  // Raw: thousands of nodes need no deep reactivity.
  let data = $state.raw<GraphData | null>(null);
  let loading = $state(false);
  let error = $state("");
  let running = $state(false);
  let hovered = $state<string | null>(null);
  let focused = $state<string | null>(null);
  let tooltip = $state<{ x: number; y: number; text: string } | null>(null);
  let announce = $state("");
  let folderInput = $state("");

  let graph: Graph | null = null;
  let sigma: Sigma | null = null;
  let layout: Layout | null = null;
  // Positions by node key, kept across reloads so the picture doesn't jump.
  const positions = new Map<string, { x: number; y: number }>();
  // Answers to an earlier request are dropped.
  let request = 0;

  const meaning = $derived(s.meaning[s.mode]);
  const folder = $derived(s.scopeFolder ?? centre?.folder ?? "");

  /** What to ask the backend for, or null when there is nothing to show yet. */
  const scope = $derived.by((): GraphScope | null => {
    if (s.mode === "local") return centre ? { kind: "local", id: centre.id, depth: s.depth } : null;
    let inner: GraphScopeNotes;
    switch (s.scopeKind) {
      case "all":
        inner = { kind: "all" };
        break;
      case "folder":
        inner = { kind: "folder", folder };
        break;
      case "tag":
        if (!s.scopeTag.trim()) return null;
        inner = { kind: "tag", tag: s.scopeTag.trim().replace(/^#/, "") };
        break;
      case "search":
        if (!currentSearch.query) return null;
        inner = { kind: "search", query: currentSearch.query };
        break;
    }
    return { kind: "global", scope: inner };
  });

  $effect(() => {
    const sc = scope;
    const options = { meaning, tags: s.tags, rareTags: s.rareTags, ghosts: s.ghosts, threshold: s.threshold, neighbours: NEIGHBOURS, cap: s.cap };
    void load(sc, options);
  });

  async function load(sc: GraphScope | null, options: Parameters<typeof api.graph>[1]) {
    const mine = ++request;
    if (!sc) {
      data = null;
      show(null);
      return;
    }
    loading = true;
    try {
      const result = await api.graph(sc, options);
      if (mine !== request) return;
      data = result;
      error = "";
      show(result);
    } catch (e) {
      if (mine !== request) return;
      error = errorMessage(e);
    }
    loading = false;
  }

  function css(name: string, fallback: string): string {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
  }

  function rgba(hex: string, alpha: number): string {
    const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
    if (!m) return hex;
    return `rgba(${parseInt(m[1], 16)}, ${parseInt(m[2], 16)}, ${parseInt(m[3], 16)}, ${alpha})`;
  }

  function nodeColor(attr: Record<string, unknown>): string {
    if (attr.kind === "tag") return css("--accent", "#7a2b3a");
    if (attr.kind === "ghost") return NONE;
    if (s.colorBy === "folder") return colorOf((attr.folder as string) || "/");
    if (s.colorBy === "topic") return colorOf(attr.topic as string | null);
    return colorOf((attr.tags as string[])[0]);
  }

  /** Builds the graph and its renderer for `result`, keeping known nodes where they were. */
  function show(result: GraphData | null) {
    layout?.kill();
    sigma?.kill();
    layout = null;
    sigma = null;
    running = false;
    hovered = focused = null;
    tooltip = null;
    if (!result || !container) return;

    const g = new Graph({ multi: true, type: "mixed" });
    const centreKey = s.mode === "local" && centre ? `n${centre.id}` : null;
    for (const n of result.nodes) {
      const known = positions.get(n.key);
      // Unknown nodes start on a ring by depth around the centre (or at random).
      const angle = Math.random() * 2 * Math.PI;
      const radius = n.depth !== null ? n.depth * 10 + Math.random() * 4 : 20 + Math.random() * 20;
      g.addNode(n.key, {
        x: known?.x ?? (n.key === centreKey ? 0 : Math.cos(angle) * radius),
        y: known?.y ?? (n.key === centreKey ? 0 : Math.sin(angle) * radius),
        label: n.label,
        kind: n.kind,
        noteId: n.id,
        folder: n.folder,
        topic: n.topic,
        tags: n.tags,
        size: 2,
      });
    }
    const linkColor = rgba(css("--muted", "#64675f"), 0.45);
    const meaningColor = rgba(css("--accent", "#7a2b3a"), 0.55);
    const tagColor = rgba(css("--accent", "#7a2b3a"), 0.25);
    for (const e of result.edges) {
      if (!g.hasNode(e.source) || !g.hasNode(e.target)) continue;
      if (e.kind === "link") {
        g.addDirectedEdge(e.source, e.target, { kind: "link", type: "arrow", color: linkColor, size: 1, weight: 1 });
      } else if (e.kind === "meaning") {
        g.addUndirectedEdge(e.source, e.target, {
          kind: "meaning",
          score: e.score,
          type: "line",
          color: meaningColor,
          size: 0.6,
          weight: Math.max(0.1, (e.score ?? 0) * 0.5),
        });
      } else {
        g.addUndirectedEdge(e.source, e.target, { kind: "tag", type: "line", color: tagColor, size: 0.5, weight: 0.5 });
      }
    }
    // Size by link count.
    g.forEachNode((key, attr) => {
      const links = g.edges(key).filter((edge) => g.getEdgeAttribute(edge, "kind") === "link").length;
      g.mergeNodeAttributes(key, {
        size: (key === centreKey ? 4 : 2) + 1.6 * Math.sqrt(links),
        color: nodeColor(attr),
      });
    });
    graph = g;

    sigma = new Sigma(g, container, {
      edgeProgramClasses: { arrow: EdgeArrowProgram, line: EdgeRectangleProgram },
      defaultEdgeType: "arrow",
      labelRenderedSizeThreshold: s.labelSize,
      labelColor: { color: css("--text", "#1a1b19") },
      labelFont: css("--sans", "sans-serif"),
      labelSize: 12,
      enableEdgeEvents: g.size <= EDGE_EVENTS_MAX,
      hideEdgesOnMove: g.size > EDGE_EVENTS_MAX,
      zIndex: true,
      nodeReducer,
      edgeReducer,
    });
    wire(sigma, g);

    layout = new Layout(g, forces());
    layout.onchange = () => {
      running = layout?.running ?? false;
      g.forEachNode((key, attr) => positions.set(key, { x: attr.x, y: attr.y }));
    };
    if (g.order > 1) {
      layout.start(ITERATIONS);
      running = true;
    }
  }

  const forces = () => ({ repulsion: s.repulsion, gravity: s.gravity, linkStrength: s.linkStrength });

  /** The node whose neighbourhood is highlighted: hovered, else keyboard-focused. */
  const highlight = () => hovered ?? focused;

  function nodeReducer(node: string, attr: Record<string, unknown>) {
    const res: Record<string, unknown> = { ...attr };
    const h = highlight();
    if (h && graph && h !== node && !graph.areNeighbors(h, node)) {
      res.color = rgba(css("--muted", "#64675f"), 0.35);
      res.label = "";
      res.zIndex = 0;
    } else if (h) {
      res.zIndex = 1;
      if (node === h) res.highlighted = true;
    }
    if (node === focused) res.highlighted = true;
    return res;
  }

  function edgeReducer(edge: string, attr: Record<string, unknown>) {
    const res: Record<string, unknown> = { ...attr };
    if (attr.kind === "link" && !s.links) res.hidden = true;
    const h = highlight();
    if (h && graph && !graph.hasExtremity(edge, h)) res.hidden = true;
    return res;
  }

  function title(key: string): string {
    return (graph?.getNodeAttribute(key, "label") as string) ?? key;
  }

  /** Why an edge exists, as its hover says. */
  function why(edge: string): string {
    if (!graph) return "";
    const a = graph.getEdgeAttributes(edge);
    const [from, to] = graph.extremities(edge);
    if (a.kind === "link") return `Link: ${title(from)} → ${title(to)}`;
    if (a.kind === "meaning") return `Meaning: ${(a.score as number).toFixed(3)} similarity (${title(from)} – ${title(to)})`;
    return `Tag ${title(to)}: ${title(from)}`;
  }

  function wire(r: Sigma, g: Graph) {
    let dragged: string | null = null;
    let wasRunning = false;
    let moved = false;
    r.on("enterNode", ({ node }) => {
      hovered = node;
      r.refresh({ skipIndexation: true });
    });
    r.on("leaveNode", () => {
      hovered = null;
      r.refresh({ skipIndexation: true });
    });
    r.on("enterEdge", ({ edge, event }) => (tooltip = { x: event.x, y: event.y, text: why(edge) }));
    r.on("leaveEdge", () => (tooltip = null));
    r.on("clickNode", ({ node }) => {
      if (moved) return;
      focused = node;
      const a = g.getNodeAttributes(node);
      if (a.kind === "note") onopen(a.noteId as number);
      else if (a.kind === "tag") {
        s.mode = "global";
        s.scopeKind = "tag";
        s.scopeTag = (a.label as string).replace(/^#/, "");
      }
    });
    r.on("doubleClickNode", ({ node, preventSigmaDefault }) => {
      preventSigmaDefault();
      const a = g.getNodeAttributes(node);
      if (a.kind === "note") onedit(a.noteId as number);
    });
    // Dragging a node pauses the layout until it is let go.
    r.on("downNode", ({ node }) => {
      dragged = node;
      moved = false;
      wasRunning = layout?.running ?? false;
      layout?.stop();
    });
    r.on("moveBody", ({ event, preventSigmaDefault }) => {
      if (!dragged) return;
      moved = true;
      const pos = r.viewportToGraph(event);
      g.mergeNodeAttributes(dragged, { x: pos.x, y: pos.y });
      positions.set(dragged, pos);
      preventSigmaDefault();
      event.original.preventDefault();
      event.original.stopPropagation();
    });
    const release = () => {
      if (!dragged) return;
      dragged = null;
      if (wasRunning) {
        layout?.start(ITERATIONS);
        running = true;
      }
    };
    r.on("upNode", release);
    r.on("upStage", release);
  }

  function toggleLayout() {
    if (!layout) return;
    if (layout.running) {
      layout.stop();
      running = false;
    } else {
      layout.setForces(forces());
      layout.start(ITERATIONS);
      running = true;
    }
  }

  function fit() {
    void sigma?.getCamera().animatedReset({ duration: 250 });
  }

  // Display-only controls: no reload, just redraw.
  $effect(() => {
    void s.links;
    void s.colorBy;
    sigma?.setSetting("labelRenderedSizeThreshold", s.labelSize);
    graph?.forEachNode((key, attr) => graph?.setNodeAttribute(key, "color", nodeColor(attr)));
    sigma?.refresh({ skipIndexation: true });
  });

  // Force changes apply to the running layout from its next batch.
  $effect(() => {
    layout?.setForces(forces());
  });

  /** Keyboard: arrows move between neighbours in roughly that direction, Enter opens. */
  function onkeydown(e: KeyboardEvent) {
    if (!graph || !sigma || graph.order === 0) return;
    const dirs: Record<string, [number, number]> = {
      ArrowLeft: [-1, 0],
      ArrowRight: [1, 0],
      ArrowUp: [0, -1],
      ArrowDown: [0, 1],
    };
    if (e.key === "Escape") {
      focused = null;
    } else if (e.key === "Enter" && focused) {
      const a = graph.getNodeAttributes(focused);
      if (a.kind === "note") (e.ctrlKey || e.metaKey ? onedit : onopen)(a.noteId as number);
    } else if (dirs[e.key]) {
      if (!focused || !graph.hasNode(focused)) {
        const start = s.mode === "local" && centre && graph.hasNode(`n${centre.id}`) ? `n${centre.id}` : graph.nodes()[0];
        focused = start;
      } else {
        const [dx, dy] = dirs[e.key];
        const from = sigma.graphToViewport(graph.getNodeAttributes(focused) as { x: number; y: number });
        let best: string | null = null;
        let bestScore = -Infinity;
        for (const n of graph.neighbors(focused)) {
          const p = sigma.graphToViewport(graph.getNodeAttributes(n) as { x: number; y: number });
          const vx = p.x - from.x;
          const vy = p.y - from.y;
          const len = Math.hypot(vx, vy) || 1;
          const cos = (vx * dx + vy * dy) / len;
          // Mostly the direction; nearer wins among similar directions.
          const score = cos - len / 2000;
          if (cos > 0.2 && score > bestScore) {
            best = n;
            bestScore = score;
          }
        }
        if (best) focused = best;
      }
    } else {
      return;
    }
    e.preventDefault();
    if (focused) {
      const a = graph.getNodeAttributes(focused);
      announce = `${a.label}${a.folder ? `, ${a.folder}` : ""}: ${graph.degree(focused)} connections`;
      const display = sigma.getNodeDisplayData(focused);
      if (display) void sigma.getCamera().animate({ x: display.x, y: display.y }, { duration: 200 });
    }
    sigma.refresh({ skipIndexation: true });
  }

  function setMode(mode: "local" | "global") {
    s.mode = mode;
  }

  function applyFolder() {
    s.scopeFolder = folderInput.trim().replace(/^\/+|\/+$/g, "");
  }

  onMount(() => {
    folderInput = folder;
    return () => {
      layout?.kill();
      sigma?.kill();
    };
  });

  const counts = $derived.by(() => {
    if (!data) return null;
    const notes = data.nodes.filter((n) => n.kind === "note").length;
    const by = (k: string) => data!.edges.filter((e) => e.kind === k).length;
    return { notes, links: by("link"), meaning: by("meaning"), tags: by("tag") };
  });

  const status = $derived.by(() => {
    if (!data || !counts) return "";
    const n = counts.notes.toLocaleString();
    if (data.capped && s.mode === "global")
      return `Showing ${n} of ${data.total.toLocaleString()} notes, most-linked first`;
    if (data.capped) return `Showing ${n} notes: the cap stopped it (${data.total.toLocaleString()}+ found)`;
    return `${n} notes`;
  });
</script>

<div class="graph-view">
  <div class="toolbar" role="toolbar" aria-label="Graph controls">
    <div class="segmented" role="group" aria-label="Graph">
      <button aria-pressed={s.mode === "local"} onclick={() => setMode("local")}>Local</button>
      <button aria-pressed={s.mode === "global"} onclick={() => setMode("global")}>Global</button>
    </div>
    {#if s.mode === "local"}
      <label class="ctl" title="Links away from the open note">
        Depth
        <input type="range" min="1" max="3" step="1" bind:value={s.depth} />
        <span class="val">{s.depth}</span>
      </label>
    {:else}
      <label class="ctl">
        Scope
        <select bind:value={s.scopeKind}>
          <option value="folder">Folder</option>
          <option value="tag">Tag</option>
          <option value="search" disabled={!currentSearch.query}>Current search</option>
          <option value="all">Whole vault</option>
        </select>
      </label>
      {#if s.scopeKind === "folder"}
        <input
          class="scope"
          aria-label="Folder"
          placeholder="(root: whole vault)"
          bind:value={folderInput}
          onchange={applyFolder}
        />
      {:else if s.scopeKind === "tag"}
        <input class="scope" aria-label="Tag" placeholder="tag" value={s.scopeTag} onchange={(e) => (s.scopeTag = e.currentTarget.value)} />
      {/if}
    {/if}
    <label class="ctl" title="Most notes shown; above it the most-linked are kept">
      Cap
      <input
        type="number"
        min="50"
        max="50000"
        step="50"
        value={s.cap}
        onchange={(e) => (s.cap = Math.max(50, Math.min(50000, Number(e.currentTarget.value) || 5000)))}
      />
    </label>
    <span class="sep" aria-hidden="true"></span>
    <label class="chk"><input type="checkbox" bind:checked={s.links} /> Links</label>
    <label class="chk"><input type="checkbox" bind:checked={s.meaning[s.mode]} /> Meaning</label>
    <label class="chk"><input type="checkbox" bind:checked={s.tags} /> Tags</label>
    <label class="chk" title="Also show tags that only one note has">
      <input type="checkbox" bind:checked={s.rareTags} disabled={!s.tags} /> One-note tags
    </label>
    <label class="chk"><input type="checkbox" bind:checked={s.ghosts} /> Unresolved</label>
    <label class="ctl" title="Lowest similarity of a meaning edge">
      ≥
      <input type="range" min="0.3" max="0.99" step="0.01" bind:value={s.threshold} disabled={!meaning} />
      <span class="val">{s.threshold.toFixed(2)}</span>
    </label>
    <span class="sep" aria-hidden="true"></span>
    <label class="ctl">
      Colour
      <select bind:value={s.colorBy}>
        <option value="folder">Folder</option>
        <option value="tag">Tag</option>
        <option value="topic">Topic</option>
      </select>
    </label>
    <label class="ctl" title="Labels show on nodes drawn at least this many pixels wide">
      Labels
      <input type="range" min="0" max="30" step="1" bind:value={s.labelSize} />
    </label>
    <details class="forces">
      <summary>Forces</summary>
      <div class="pop">
        <label class="ctl">Repulsion <input type="range" min="0.1" max="20" step="0.1" bind:value={s.repulsion} /><span class="val">{s.repulsion}</span></label>
        <label class="ctl">Gravity <input type="range" min="0" max="10" step="0.1" bind:value={s.gravity} /><span class="val">{s.gravity}</span></label>
        <label class="ctl">Link strength <input type="range" min="0" max="3" step="0.1" bind:value={s.linkStrength} /><span class="val">{s.linkStrength}</span></label>
      </div>
    </details>
    <span class="spacer"></span>
    <button class="small" onclick={toggleLayout} disabled={!data}>{running ? "Pause" : "Resume"} layout</button>
    <button class="small icon" onclick={fit} aria-label="Fit to view" title="Fit to view"><Icon name="eye" size={14} /></button>
  </div>

  <div class="status" aria-live="polite">
    {#if error}
      <span class="error">{error}</span>
    {:else if loading && !data}
      Loading…
    {:else if !scope}
      {s.mode === "local" ? "Open a note to see its graph." : "Choose a scope."}
    {:else if data}
      <span>{status}</span>
      {#if counts}
        <span class="legend link">{counts.links} links</span>
        {#if meaning}<span class="legend meaning">{counts.meaning} meaning</span>{/if}
        {#if s.tags}<span class="legend tag">{counts.tags} tag</span>{/if}
      {/if}
      {#if data.meaningUnavailable && meaning}<span class="muted">Meaning: notes not indexed yet</span>{/if}
      {#if loading}<span class="muted">Updating…</span>{/if}
    {/if}
  </div>

  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="canvas"
    bind:this={container}
    tabindex="0"
    role="application"
    aria-label="Graph of notes. Arrow keys move between neighbours, Enter opens, Ctrl+Enter edits."
    {onkeydown}
  ></div>
  {#if tooltip}
    <div class="tooltip" style:left="{tooltip.x + 12}px" style:top="{tooltip.y + 12}px">{tooltip.text}</div>
  {/if}
  <p class="sr-only" aria-live="polite">{announce}</p>
</div>

<style>
  .graph-view {
    position: relative;
    display: grid;
    grid-template-rows: auto auto minmax(0, 1fr);
    height: 100%;
    min-height: 0;
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 10px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--panel);
    font-size: 12px;
  }

  .ctl,
  .chk {
    display: inline-flex;
    flex-direction: row;
    align-items: center;
    gap: 4px;
    color: var(--text-2);
    white-space: nowrap;
  }

  .ctl input[type="range"] {
    width: 72px;
  }

  .ctl input[type="number"] {
    width: 72px;
    height: 26px;
    font-family: var(--mono);
    font-size: 12px;
  }

  .ctl select,
  .scope {
    height: 26px;
    font-size: 12px;
  }

  .scope {
    width: 160px;
    font-family: var(--mono);
  }

  .val {
    min-width: 2.5em;
    font-family: var(--mono);
    font-size: 11px;
    color: var(--muted);
  }

  .sep {
    width: 1px;
    height: 18px;
    background: var(--border);
  }

  .spacer {
    flex: 1;
  }

  .forces {
    position: relative;
  }

  .forces summary {
    cursor: pointer;
    color: var(--text-2);
  }

  .forces .pop {
    position: absolute;
    z-index: 10;
    top: 22px;
    left: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--raised);
    box-shadow: var(--shadow);
  }

  .status {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    padding: 4px 12px;
    border-bottom: 1px solid var(--border);
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-2);
  }

  .legend::before {
    content: "";
    display: inline-block;
    width: 14px;
    height: 0;
    margin-right: 4px;
    vertical-align: middle;
    border-top: 2px solid var(--muted);
  }

  .legend.meaning::before {
    border-top-color: var(--accent);
  }

  .legend.tag::before {
    border-top: 1px solid var(--accent);
  }

  .canvas {
    position: relative;
    min-height: 0;
    background: var(--surface);
  }

  .canvas:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .tooltip {
    position: absolute;
    z-index: 20;
    max-width: 320px;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--raised);
    box-shadow: var(--shadow);
    font-size: 12px;
    pointer-events: none;
    transform: translateY(var(--tooltip-offset, 64px));
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
</style>
