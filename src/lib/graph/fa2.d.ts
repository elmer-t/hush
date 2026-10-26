// The parts of graphology-layout-forceatlas2 the layout worker uses directly; the
// package ships no types for them.
declare module "graphology-layout-forceatlas2/iterate.js" {
  import type { ForceAtlas2Settings } from "graphology-layout-forceatlas2";
  /** One iteration over the node and edge matrices, in place. */
  export default function iterate(settings: ForceAtlas2Settings, nodes: Float32Array, edges: Float32Array): void;
}

declare module "graphology-layout-forceatlas2/helpers.js" {
  import type Graph from "graphology-types";
  const helpers: {
    graphToByteArrays(
      graph: Graph,
      getEdgeWeight: ((edge: string, attributes: Record<string, unknown>) => number) | null,
    ): { nodes: Float32Array; edges: Float32Array };
    assignLayoutChanges(graph: Graph, nodes: Float32Array, outputReducer: null): void;
  };
  export default helpers;
}
