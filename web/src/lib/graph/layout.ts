import Graph from 'graphology';
import forceAtlas2 from 'graphology-layout-forceatlas2';
import FA2Layout from 'graphology-layout-forceatlas2/worker';
import noverlap from 'graphology-layout-noverlap';

/**
 * 4-tier ForceAtlas2 settings adapted to graph size.
 *
 * Key design decisions (Phase 19.2 alignment):
 * - adjustSizes: true — size-aware repulsion for 17-type node hierarchy (3-20px range)
 * - gravity: 0.8–0.15 — higher gravity at small graph sizes keeps nodes compact;
 *   lower gravity for large graphs gives breathing room
 * - scalingRatio: 15–100 — higher ratios spread nodes further; scaled up from
 *   Phase 19.1 values (2-10) to account for adjustSizes and outboundAttractionDistribution
 * - outboundAttractionDistribution: true — distributes attraction by node degree
 *   so hub nodes (files with many symbols) don't over-attract
 * - edgeWeightInfluence: 1 — layout respects per-edge weight multipliers from API
 */
export function getFA2Settings(nodeCount: number, graph: Graph): ReturnType<typeof forceAtlas2.inferSettings> & Record<string, unknown> {
  const base = forceAtlas2.inferSettings(graph);
  const common = {
    strongGravityMode: false,
    outboundAttractionDistribution: true,
    linLogMode: false,
    adjustSizes: true,
    edgeWeightInfluence: 1,
  };

  if (nodeCount < 500) {
    return {
      ...base,
      ...common,
      gravity: 0.8,
      scalingRatio: 15,
      slowDown: 1,
      barnesHutOptimize: false,
    };
  }
  if (nodeCount < 2000) {
    return {
      ...base,
      ...common,
      gravity: 0.5,
      scalingRatio: 30,
      slowDown: 2,
      barnesHutOptimize: true,
      barnesHutTheta: 0.5,
    };
  }
  if (nodeCount < 10000) {
    return {
      ...base,
      ...common,
      gravity: 0.3,
      scalingRatio: 60,
      slowDown: 3,
      barnesHutOptimize: true,
      barnesHutTheta: 0.7,
    };
  }
  // Tier 4: 10000+ nodes
  return {
    ...base,
    ...common,
    gravity: 0.15,
    scalingRatio: 100,
    slowDown: 5,
    barnesHutOptimize: true,
    barnesHutTheta: 0.8,
  };
}

/**
 * Layout duration in milliseconds, scaled to graph size.
 * Smaller graphs converge quickly — no need to run for 20+ seconds.
 * Larger graphs need more iterations to settle cluster structure.
 */
export function getLayoutDuration(nodeCount: number): number {
  if (nodeCount >= 10000) return 30000;
  if (nodeCount >= 5000) return 20000;
  if (nodeCount >= 2000) return 15000;
  if (nodeCount >= 500) return 10000;
  return 6000;
}

/** Golden angle (radians) for even radial distribution of structural nodes. */
const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5));

/** Structural node kinds that get golden-angle radial positions first. */
const STRUCTURAL_KINDS = new Set(['project', 'package', 'module', 'folder']);

/**
 * Golden-angle pre-seeding for graph positions.
 *
 * First pass: structural nodes get golden-angle radial positions so FA2 has
 * a good structural skeleton to converge from.
 *
 * Second pass: non-structural nodes are placed near a seeded neighbor with
 * jitter, or randomly within spread if no seeded neighbor exists.
 *
 * Spread formula: sqrt(nodeCount) * 10 gives each node ~100 sq units of space
 * on average, which matches FA2's scalingRatio=2 natural equilibrium distance.
 */
export function seedPositions(graph: Graph): void {
  const nodeCount = graph.order;
  const spread = Math.sqrt(nodeCount) * 10;
  const jitter = Math.sqrt(nodeCount) * 1.5;

  // First pass: golden-angle positions for structural nodes.
  // Skip nodes that already have non-origin positions (restored from localStorage).
  let idx = 0;
  graph.forEachNode((node, attrs) => {
    const kind = ((attrs.kind as string) || '').toLowerCase();
    if (!STRUCTURAL_KINDS.has(kind)) return;
    // If already positioned (localStorage restore), keep the position but still
    // count it for indexing so golden-angle spacing remains consistent.
    const existingX = (attrs.x as number) ?? 0;
    const existingY = (attrs.y as number) ?? 0;
    if (Math.hypot(existingX, existingY) > 0.001) {
      idx++;
      return;
    }
    const angle = idx * GOLDEN_ANGLE;
    const r = spread * Math.sqrt(idx + 1);
    graph.setNodeAttribute(node, 'x', r * Math.cos(angle));
    graph.setNodeAttribute(node, 'y', r * Math.sin(angle));
    idx++;
  });

  // Second pass: non-structural nodes near a seeded neighbor, or random.
  //
  // CRITICAL: The backend API sends x: 0.0, y: 0.0 for all nodes. We cannot
  // use "x !== undefined" to detect nodes that have already been positioned
  // (e.g. via localStorage restore), because 0.0 !== undefined is true.
  //
  // Solution: track which nodes were positioned by localStorage restore using a
  // Set, then only skip those. All other nodes need seeding. The
  // restorePositions() call happens before startLayout(), which marks restored
  // nodes via setNodeAttribute(). We detect "real" positions as any (x,y) pair
  // where Math.hypot(x,y) > 0 (i.e. not the API-default origin).
  graph.forEachNode((node, attrs) => {
    const kind = ((attrs.kind as string) || '').toLowerCase();
    if (STRUCTURAL_KINDS.has(kind)) return; // Already seeded

    const existingX = (attrs.x as number) ?? 0;
    const existingY = (attrs.y as number) ?? 0;
    // If the node has a non-origin position, it was restored from localStorage — keep it
    if (Math.hypot(existingX, existingY) > 0.001) return;

    // Find a neighbor that has a real (non-origin) position from the first pass
    const neighbors = graph.neighbors(node);
    const seededNeighbor = neighbors.find((n) => {
      const nx = graph.getNodeAttribute(n, 'x') as number;
      const ny = graph.getNodeAttribute(n, 'y') as number;
      // A "seeded" neighbor has a non-origin position (set by first pass or localStorage)
      return Math.hypot(nx ?? 0, ny ?? 0) > 0.001;
    });

    if (seededNeighbor) {
      const sx = graph.getNodeAttribute(seededNeighbor, 'x') as number;
      const sy = graph.getNodeAttribute(seededNeighbor, 'y') as number;
      graph.setNodeAttribute(node, 'x', sx + (Math.random() - 0.5) * jitter);
      graph.setNodeAttribute(node, 'y', sy + (Math.random() - 0.5) * jitter);
    } else {
      graph.setNodeAttribute(node, 'x', (Math.random() - 0.5) * spread);
      graph.setNodeAttribute(node, 'y', (Math.random() - 0.5) * spread);
    }
  });
}

/**
 * Noverlap post-pass to eliminate node overlap after FA2 converges.
 * Must run AFTER FA2 stops so positions are settled.
 * 20 iterations synchronously — safe even for 5K nodes (<100ms).
 *
 * ratio: 1.2 — nodes repel when within 120% of combined size
 * margin: 10 — larger gap between nodes for label readability with 17 types
 * expansion: 1.05 — slight global scale increase to accommodate varied node sizes
 */
export function runNoverlap(graph: Graph): void {
  noverlap.assign(graph, {
    maxIterations: 20,
    settings: { ratio: 1.2, margin: 10, expansion: 1.05 },
  });
}

/**
 * Start ForceAtlas2 layout with 4-tier settings, golden-angle pre-seeding,
 * and duration-based auto-stop with noverlap post-pass.
 *
 * @param graph - The graphology graph to lay out.
 * @param onComplete - Optional callback fired after FA2 stops and noverlap runs.
 * @returns The FA2Layout worker instance (for stop/kill control).
 */
export function startLayout(graph: Graph, onComplete?: () => void): FA2Layout {
  // Pre-seed structural nodes before FA2 starts for better convergence
  seedPositions(graph);

  const layout = new FA2Layout(graph, {
    settings: getFA2Settings(graph.order, graph),
  });

  layout.start();

  const duration = getLayoutDuration(graph.order);

  setTimeout(() => {
    try {
      layout.stop();
    } catch {
      // Layout may already be stopped
    }

    // 100ms buffer for FA2 worker to flush positions to the main-thread graph object
    // before noverlap reads them (avoids race condition with stale positions).
    setTimeout(() => {
      try {
        runNoverlap(graph);
      } catch {
        // Noverlap may fail on edge cases (e.g. no positions set)
      }
      onComplete?.();
    }, 100);
  }, duration);

  return layout;
}

/**
 * Stop and kill an FA2Layout worker safely.
 */
export function stopLayout(layout: FA2Layout): void {
  try {
    layout.stop();
    layout.kill();
  } catch {
    // Layout may already be stopped/killed
  }
}
