import Graph from 'graphology';
import type { GraphResponse } from '../types';
import { fetchGraph as apiFetchGraph } from '../api';

/**
 * Compute FA2 layout mass for a node based on its kind and total graph size.
 *
 * Higher mass nodes act as layout anchors, keeping structural nodes (project,
 * package, folder) from being pushed to the periphery by many small symbol nodes.
 *
 * Mass multiplier scales with graph size so forces remain balanced in large graphs:
 * - nodeCount >= 5000: 2x
 * - nodeCount >= 1000: 1.5x
 * - nodeCount < 1000: 1x
 */
export function getNodeMass(kind: string, nodeCount: number): number {
  const multiplier = nodeCount >= 5000 ? 2 : nodeCount >= 1000 ? 1.5 : 1;
  switch (kind.toLowerCase()) {
    case 'project':
      return 50 * multiplier;
    case 'package':
      return 30 * multiplier;
    case 'module':
      return 20 * multiplier;
    case 'folder':
      return 15 * multiplier;
    case 'file':
      return 3 * multiplier;
    case 'class':
    case 'interface':
    case 'struct':
    case 'trait':
      return 5 * multiplier;
    case 'function':
    case 'method':
    case 'impl_method':
      return 2 * multiplier;
    case 'enum':
    case 'component':
      return 2 * multiplier;
    case 'type':
    case 'macro':
      return 1.5 * multiplier;
    default:
      return 1 * multiplier;
  }
}

/**
 * Compute a density-scaled node display size based on kind and total node count.
 *
 * Base sizes by kind (17-type hierarchy):
 * - structural: project (20), package (18), module (16), folder (15)
 * - file: 7
 * - major symbols: class/struct (5), interface/trait (5), enum/component (4.5)
 * - functions/methods: function/method/impl_method (4)
 * - minor types: type/macro (3.5), variable/const/static/property (3)
 *
 * Density scaling: large graphs compress sizes to prevent visual overcrowding.
 * The baseSize parameter is accepted for API clarity but size is derived from kind.
 */
export function getScaledNodeSize(_baseSize: number, nodeCount: number, kind: string): number {
  const lk = kind.toLowerCase();

  let baseSize: number;
  // Structural nodes: largest
  if (lk === 'project') baseSize = 20;
  else if (lk === 'package') baseSize = 18;
  else if (lk === 'module') baseSize = 16;
  else if (lk === 'folder') baseSize = 15;
  // File nodes: medium
  else if (lk === 'file') baseSize = 7;
  // Major symbol types: medium-small
  else if (['class', 'struct'].includes(lk)) baseSize = 5;
  else if (['interface', 'trait'].includes(lk)) baseSize = 5;
  else if (['enum', 'component'].includes(lk)) baseSize = 4.5;
  // Functions/methods: small
  else if (['function', 'method', 'impl_method'].includes(lk)) baseSize = 4;
  // Minor types: smallest
  else if (['type', 'macro'].includes(lk)) baseSize = 3.5;
  else if (['variable', 'const', 'static', 'property'].includes(lk)) baseSize = 3;
  else baseSize = 3;

  let scaledSize: number;
  let minSize: number;
  if (nodeCount > 50000) {
    scaledSize = baseSize * 0.4;
    minSize = 1;
  } else if (nodeCount > 20000) {
    scaledSize = baseSize * 0.5;
    minSize = 1.5;
  } else if (nodeCount > 5000) {
    scaledSize = baseSize * 0.65;
    minSize = 2;
  } else if (nodeCount > 1000) {
    scaledSize = baseSize * 0.8;
    minSize = 2.5;
  } else {
    scaledSize = baseSize;
    minSize = baseSize;
  }

  return Math.max(scaledSize, minSize);
}

export async function loadGraph(granularity: string = 'file'): Promise<Graph> {
  const data = await apiFetchGraph(granularity);
  return buildGraphologyGraph(data);
}

export function buildGraphologyGraph(data: GraphResponse): Graph {
  const graph = new Graph();
  const nodeCount = data.nodes.length;

  for (const node of data.nodes) {
    const kind = node.attributes.kind ?? '';
    const mass = getNodeMass(kind, nodeCount);
    const size = getScaledNodeSize(node.attributes.size, nodeCount, kind);
    graph.addNode(node.key, { ...node.attributes, mass, size });
  }

  for (const edge of data.edges) {
    try {
      graph.addEdgeWithKey(edge.key, edge.source, edge.target, {
        ...edge.attributes,
        type: 'curved',
        curvature: 0.12 + Math.random() * 0.08,
        weight: (edge.attributes as Record<string, unknown>).weight ?? 1.0,
      });
    } catch {
      // Skip duplicate edges
    }
  }

  return graph;
}
