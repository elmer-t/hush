// Runs ForceAtlas2 iterations off the main thread. A module worker loaded from the app
// itself: the CSP allows no blob: workers, which the package's own supervisor uses.
import iterate from "graphology-layout-forceatlas2/iterate.js";
import type { ForceAtlas2Settings } from "graphology-layout-forceatlas2";

let edges = new Float32Array(0);

type Request = { settings: ForceAtlas2Settings; iterations: number; nodes: ArrayBuffer; edges?: ArrayBuffer };

self.onmessage = (e: MessageEvent<Request>) => {
  if (e.data.edges) edges = new Float32Array(e.data.edges);
  const nodes = new Float32Array(e.data.nodes);
  for (let i = 0; i < e.data.iterations; i++) iterate(e.data.settings, nodes, edges);
  (self as unknown as Worker).postMessage({ nodes: nodes.buffer }, [nodes.buffer]);
};
