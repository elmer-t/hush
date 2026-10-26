// Drives the layout worker: the graph's positions go to the worker as typed arrays and
// come back after a batch of iterations, so the UI never waits for the layout.
import type Graph from "graphology";
import helpers from "graphology-layout-forceatlas2/helpers.js";
import forceAtlas2, { type ForceAtlas2Settings } from "graphology-layout-forceatlas2";

/** Iterations per round trip to the worker. */
const BATCH = 5;

export type ForceSettings = { repulsion: number; gravity: number; linkStrength: number };

export class Layout {
  private worker: Worker | null = null;
  private nodes: Float32Array | null = null;
  private edges: Float32Array | null = null;
  private settings: ForceAtlas2Settings;
  private remaining = 0;
  running = false;
  /** Called after each batch has moved the nodes, and when the layout stops by itself. */
  onchange: () => void = () => {};

  constructor(
    private graph: Graph,
    force: ForceSettings,
  ) {
    this.settings = Layout.settings(graph, force);
  }

  static settings(graph: Graph, force: ForceSettings): ForceAtlas2Settings {
    return {
      ...forceAtlas2.inferSettings(graph),
      barnesHutOptimize: graph.order > 500,
      scalingRatio: force.repulsion,
      gravity: force.gravity,
      edgeWeightInfluence: force.linkStrength,
    };
  }

  setForces(force: ForceSettings) {
    this.settings = Layout.settings(this.graph, force);
  }

  /** Runs `iterations` more (or until stopped), from the graph's current positions. */
  start(iterations: number) {
    this.remaining = iterations;
    if (this.running) return;
    this.running = true;
    if (!this.worker) {
      this.worker = new Worker(new URL("./layout.worker.ts", import.meta.url), { type: "module" });
      this.worker.onmessage = (e: MessageEvent<{ nodes: ArrayBuffer }>) => this.received(e.data.nodes);
    }
    const matrices = helpers.graphToByteArrays(this.graph, (_, attr) => (attr.weight as number) ?? 1);
    this.nodes = matrices.nodes;
    this.edges = matrices.edges;
    this.ask(true);
  }

  stop() {
    this.running = false;
  }

  kill() {
    this.running = false;
    this.worker?.terminate();
    this.worker = null;
  }

  private ask(withEdges: boolean) {
    if (!this.worker || !this.nodes || !this.edges) return;
    const buffers: ArrayBuffer[] = [this.nodes.buffer as ArrayBuffer];
    const message: Record<string, unknown> = { settings: this.settings, iterations: BATCH, nodes: this.nodes.buffer };
    if (withEdges) {
      // A copy: the edges are kept here for the next start.
      const copy = this.edges.slice();
      message.edges = copy.buffer;
      buffers.push(copy.buffer);
    }
    this.worker.postMessage(message, buffers);
  }

  private received(buffer: ArrayBuffer) {
    const nodes = new Float32Array(buffer);
    this.nodes = nodes;
    if (!this.running) return;
    helpers.assignLayoutChanges(this.graph, nodes, null);
    this.remaining -= BATCH;
    if (this.remaining <= 0) this.running = false;
    this.onchange();
    if (this.running) this.ask(false);
  }
}
