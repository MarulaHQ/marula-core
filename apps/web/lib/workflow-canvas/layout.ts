// ELK auto-layout for the canvas (ADR 0016 decision 2, spec E).
//
// Purpose: layout node positions from the document's edges with `elkjs`
// (Apache-2.0, OSS) — integrated directly rather than inherited, because the
// alternative we rejected (Workflow Builder) gates its layout behind an
// enterprise licence. The document stores no positions (the schema has none),
// so this is how a loaded graph gets placed.
//
// Non-purpose: no incremental/graph-diffing layout. It lays out the whole graph
// on demand (a button and on first load); the user's own arrangement is kept
// in the component and only offered back to this function explicitly.

import ELK from "elkjs/lib/elk.bundled.js";
import type { WorkflowDocumentDoc } from "./document";

export interface NodePosition {
  x: number;
  y: number;
}

/** Node footprint passed to ELK so edges route to real card sizes. */
const NODE_WIDTH = 280;
const NODE_HEIGHT = 132;

let elkSingleton: InstanceType<typeof ELK> | null = null;
function elk(): InstanceType<typeof ELK> {
  if (!elkSingleton) {
    elkSingleton = new ELK();
  }
  return elkSingleton;
}

/**
 * Left-to-right layered layout, matching the mockup's reading spine (trigger
 * entry on the left, terminals on the right). Returns a position per node id;
 * an empty map for an empty document.
 */
export async function layoutDocument(
  doc: WorkflowDocumentDoc,
): Promise<Record<string, NodePosition>> {
  if (doc.nodes.length === 0) return {};
  const graph = {
    id: "root",
    layoutOptions: {
      "elk.algorithm": "layered",
      "elk.direction": "RIGHT",
      "elk.layered.spacing.nodeNodeBetweenLayers": "90",
      "elk.spacing.nodeNode": "48",
      "elk.layered.nodePlacement.strategy": "NETWORK_SIMPLEX",
      "elk.edgeRouting": "ORTHOGONAL",
      "elk.layered.considerModelOrder.strategy": "NODES_AND_EDGES",
    },
    children: doc.nodes.map((node) => ({
      id: node.id,
      width: NODE_WIDTH,
      height: NODE_HEIGHT,
    })),
    edges: doc.edges.map((edge) => ({
      id: `${edge.from}->${edge.to}:${edge.label ?? ""}`,
      sources: [edge.from],
      targets: [edge.to],
    })),
  };

  try {
    const result = await elk().layout(graph);
    const positions: Record<string, NodePosition> = {};
    for (const child of result.children ?? []) {
      positions[child.id] = { x: child.x ?? 0, y: child.y ?? 0 };
    }
    return positions;
  } catch {
    // Layout is a convenience, never a correctness gate: on failure fall back
    // to a simple vertical stack so the graph is still usable and the user
    // can place nodes themselves.
    const positions: Record<string, NodePosition> = {};
    doc.nodes.forEach((node, i) => {
      positions[node.id] = { x: 0, y: i * (NODE_HEIGHT + 48) };
    });
    return positions;
  }
}

export { NODE_WIDTH, NODE_HEIGHT };
