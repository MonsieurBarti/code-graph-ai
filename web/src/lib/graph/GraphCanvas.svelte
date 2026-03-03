<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import Sigma from 'sigma';
  import Graph from 'graphology';
  import EdgeCurveProgram from '@sigma/edge-curve';
  import { loadGraph } from './graphData';
  import { startLayout, stopLayout } from './layout';
  import { dimColor, brightenColor } from './colorHelpers';
  import type FA2Layout from 'graphology-layout-forceatlas2/worker';
  import type { NodeDisplayData, EdgeDisplayData, PlainObject, CameraState } from 'sigma/types';

  interface Props {
    granularity?: 'file' | 'symbol' | 'package';
    depth?: number | null;
    selectedNode?: string | null;
    onNodeClick?: (nodeKey: string) => void;
    onNodeRightClick?: (nodeKey: string, x: number, y: number) => void;
    /** Set of edge types to display. Toggled-off types are hidden via edgeReducer. */
    visibleEdgeTypes?: Set<string>;
    /** Callback fired whenever the layout running state changes. */
    onLayoutChange?: (running: boolean) => void;
  }

  let {
    granularity = 'file',
    depth = null,
    selectedNode = null,
    onNodeClick,
    onNodeRightClick,
    visibleEdgeTypes = new Set(['Imports', 'Calls', 'Extends', 'Contains', 'Implements', 'Circular']),
    onLayoutChange,
  }: Props = $props();

  let container: HTMLDivElement;
  let renderer: Sigma | null = null;
  let graph: Graph | null = null;
  let layout: FA2Layout | null = null;
  let searchQuery = $state('');

  // Loading / error state
  let loadError: string | null = $state(null);
  let isLoading: boolean = $state(true);

  // Layout indicator state
  let layoutRunning: boolean = $state(false);

  // Depth filter: set of visible node keys (null = all visible)
  let visibleNodes: Set<string> | null = $state(null);

  // Drag state
  let draggedNode: string | null = null;
  let isDragging = false;

  /**
   * Plain mutable interaction state object — NOT $state().
   *
   * CRITICAL: Do NOT use $state() for values captured in reducer closures.
   * Svelte 5 reactive state captured at function creation time is stale inside
   * Sigma reducer callbacks (Pitfall 1 in RESEARCH.md). Use a plain mutable
   * object that reducers read from at call time (not capture time).
   *
   * Mutation: event handlers mutate these fields and call renderer.refresh().
   * Read: nodeReducer/edgeReducer read these fields every frame.
   */
  const interactionState = {
    hoveredNode: null as string | null,
    oneHopNeighbors: new Set<string>(),
    twoHopNeighbors: new Set<string>(),
  };

  // localStorage key for saving positions
  function getPositionKey(): string {
    return `code-graph-positions-${encodeURIComponent(window.location.host)}`;
  }

  function savePositions() {
    if (!graph) return;
    const positions: Record<string, { x: number; y: number }> = {};
    graph.forEachNode((node) => {
      positions[node] = {
        x: graph!.getNodeAttribute(node, 'x') as number,
        y: graph!.getNodeAttribute(node, 'y') as number,
      };
    });
    try {
      localStorage.setItem(getPositionKey(), JSON.stringify(positions));
    } catch {
      // Ignore storage errors
    }
  }

  function restorePositions(g: Graph): boolean {
    try {
      const saved = localStorage.getItem(getPositionKey());
      if (!saved) return false;
      const positions = JSON.parse(saved) as Record<string, { x: number; y: number }>;
      let restored = 0;
      g.forEachNode((node) => {
        if (positions[node]) {
          g.setNodeAttribute(node, 'x', positions[node].x);
          g.setNodeAttribute(node, 'y', positions[node].y);
          restored++;
        }
      });
      return restored > 0;
    } catch {
      return false;
    }
  }

  /**
   * BFS from startNode up to maxDepth hops, returns set of reachable node keys.
   */
  function bfsNeighborhood(g: Graph, startNode: string, maxDepth: number): Set<string> {
    const visited = new Set<string>([startNode]);
    const queue: Array<{ node: string; depth: number }> = [{ node: startNode, depth: 0 }];

    while (queue.length > 0) {
      const item = queue.shift()!;
      if (item.depth >= maxDepth) continue;

      g.forEachNeighbor(item.node, (neighbor) => {
        if (!visited.has(neighbor)) {
          visited.add(neighbor);
          queue.push({ node: neighbor, depth: item.depth + 1 });
        }
      });
    }

    return visited;
  }

  /**
   * Update depth filter visibility based on current selectedNode and depth.
   */
  function updateDepthFilter() {
    if (!graph || depth === null || !selectedNode) {
      visibleNodes = null;
    } else if (selectedNode && graph.hasNode(selectedNode)) {
      visibleNodes = bfsNeighborhood(graph, selectedNode, depth);
    } else {
      visibleNodes = null;
    }
    renderer?.refresh();
  }

  /**
   * Custom canvas hover renderer — replaces DOM tooltip.
   *
   * Draws a glow ring (node-colored arc) and a dark pill tooltip with the node
   * label directly onto the Sigma canvas. No DOM reflow, no z-index fighting.
   *
   * Pattern 3 from RESEARCH.md.
   */
  function customDrawNodeHover(
    context: CanvasRenderingContext2D,
    data: { x: number; y: number; size: number; label: string | null; color: string },
    settings: { labelFont: string; labelSize: number; labelWeight: string },
  ): void {
    const { x, y, size, label, color } = data;
    const nodeColor = color || '#6b7280';

    // Glow ring: node-colored arc slightly outside the node radius
    context.beginPath();
    context.arc(x, y, size + 4, 0, Math.PI * 2);
    context.strokeStyle = nodeColor;
    context.globalAlpha = 0.5;
    context.lineWidth = 2;
    context.stroke();
    context.globalAlpha = 1;

    if (!label) return;

    // Measure label for pill sizing
    const font = `${settings.labelWeight} ${settings.labelSize}px ${settings.labelFont}`;
    context.font = font;
    const textWidth = context.measureText(label).width;
    const padding = 8;
    const pillW = textWidth + padding * 2;
    const pillH = settings.labelSize + padding;
    const pillX = x + size + 6;
    const pillY = y - pillH / 2;

    // Dark pill background
    context.beginPath();
    if (context.roundRect) {
      context.roundRect(pillX, pillY, pillW, pillH, 4);
    } else {
      context.rect(pillX, pillY, pillW, pillH); // Fallback for Firefox < 112
    }
    context.fillStyle = '#0d0b12';
    context.fill();

    // Node-color border (2px)
    context.strokeStyle = nodeColor;
    context.lineWidth = 2;
    context.stroke();

    // Label text
    context.fillStyle = '#f0eeff';
    context.fillText(label, pillX + padding, pillY + settings.labelSize);
  }

  /**
   * NodeReducer — transforms stored node attributes into display data every frame.
   *
   * Priority order:
   * 1. Depth filter: dim nodes outside BFS neighborhood
   * 2. Search: dim non-matching nodes
   * 3. Selection hierarchy: selected (1.8x), 1-hop (1.3x), 2-hop (1.1x dimmed), others (0.6x dimmed)
   * 4. Hover dimming: dim non-neighbors when hovering (no selection active)
   * 5. Default
   *
   * Reads from interactionState (plain mutable object, not $state).
   * Reads selectedNode from the Props binding captured in the closure — this is safe
   * because we call renderer.refresh() whenever selectedNode changes via $effect.
   */
  function nodeReducer(node: string, data: PlainObject): Partial<NodeDisplayData> {
    if (!renderer) return data as Partial<NodeDisplayData>;

    const baseColor = data.color as string;
    const baseSize = data.size as number;

    // Build label — append decorator badges if present.
    // Only allocate a new string when decorators exist (hot-path optimization).
    const rawLabel = data.label as string;
    const decorators = data.decorators as string[] | undefined;
    const label = (decorators && decorators.length > 0)
      ? `${rawLabel} ${decorators.slice(0, 2).map((d) => `[@${d}]`).join(' ')}`
      : rawLabel;

    // 1. Depth filter: dim nodes not in visible set (preserve hue via dimColor)
    if (visibleNodes !== null && !visibleNodes.has(node)) {
      return {
        ...data,
        label: '',
        color: dimColor(baseColor, 0.25),
        size: baseSize * 0.3,
        hidden: false,
      } as Partial<NodeDisplayData>;
    }

    // 2. Search dimming: dim non-matching nodes
    if (searchQuery && searchQuery.length > 0) {
      const nodeLabel = rawLabel.toLowerCase();
      if (!nodeLabel.includes(searchQuery.toLowerCase())) {
        return {
          ...data,
          label: '',
          color: dimColor(baseColor, 0.3),
          size: baseSize * 0.5,
        } as Partial<NodeDisplayData>;
      }
    }

    // 3. Selection hierarchy
    if (selectedNode !== null) {
      if (selectedNode === node) {
        // Selected: full color, 1.8x size, highest z-index
        return {
          ...data,
          label,
          color: baseColor,
          size: baseSize * 1.8,
          zIndex: 2,
        } as Partial<NodeDisplayData>;
      }

      if (interactionState.oneHopNeighbors.has(node)) {
        // 1-hop neighbor: full color, 1.3x size
        return {
          ...data,
          label,
          color: baseColor,
          size: baseSize * 1.3,
          zIndex: 1,
        } as Partial<NodeDisplayData>;
      }

      if (interactionState.twoHopNeighbors.has(node)) {
        // 2-hop neighbor: slightly dimmed, 1.1x size
        return {
          ...data,
          label,
          color: dimColor(baseColor, 0.6),
          size: baseSize * 1.1,
          zIndex: 1,
        } as Partial<NodeDisplayData>;
      }

      // All others: heavily dimmed, 0.4x size, lowest z-index (dramatic spotlight)
      return {
        ...data,
        label: '',
        color: dimColor(baseColor, 0.15),
        size: baseSize * 0.4,
        zIndex: 0,
      } as Partial<NodeDisplayData>;
    }

    // 4. Hover dimming (no selection active)
    if (
      interactionState.hoveredNode !== null &&
      interactionState.hoveredNode !== node &&
      !interactionState.oneHopNeighbors.has(node)
    ) {
      return {
        ...data,
        label,
        color: dimColor(baseColor, 0.15),
        size: baseSize * 0.4,
      } as Partial<NodeDisplayData>;
    }

    // 5. Default — return data with label (only allocates when label differs)
    if (label === rawLabel) return data as Partial<NodeDisplayData>;
    return { ...data, label } as Partial<NodeDisplayData>;
  }

  /**
   * EdgeReducer — transforms stored edge attributes into display data every frame.
   *
   * Priority order:
   * 1. Edge type filtering: hide edges of toggled-off types
   * 2. Depth filter: dim edges outside BFS neighborhood
   * 3. Selection: brighten connected edges, dim non-connected
   * 4. Hover edge preview: slightly brighten edges connected to hovered node
   * 5. Circular: red coloring
   * 6. Default
   */
  function edgeReducer(edge: string, data: PlainObject): Partial<EdgeDisplayData> {
    // 1. Edge type filtering (belt-and-suspenders: hidden: true + size: 0)
    const edgeType = data.edgeType as string;
    if (!visibleEdgeTypes.has(edgeType)) {
      return { ...data, hidden: true, size: 0 } as Partial<EdgeDisplayData>;
    }

    // Get edge endpoints for selection/hover logic
    const source = graph?.source(edge);
    const target = graph?.target(edge);

    // 2. Depth filter: dim edges with endpoints outside visible set
    if (visibleNodes !== null && source && target) {
      if (!visibleNodes.has(source) || !visibleNodes.has(target)) {
        return { ...data, color: dimColor(data.color as string || '#4a4a4a', 0.15), size: 0.3 } as Partial<EdgeDisplayData>;
      }
    }

    // 3. Selection edge highlighting
    if (selectedNode !== null && source && target) {
      const isConnected = source === selectedNode || target === selectedNode;
      if (isConnected) {
        // Connected edge: brighten and grow, respecting weight
        const weight = (data.weight as number) || 1.0;
        return {
          ...data,
          color: brightenColor(data.color as string || '#4a4060', 1.5),
          size: Math.max(3, weight * 4),
        } as Partial<EdgeDisplayData>;
      } else {
        // Non-connected edge: dim to near-invisible
        return {
          ...data,
          color: dimColor(data.color as string || '#4a4a4a', 0.05),
          size: 0.3,
        } as Partial<EdgeDisplayData>;
      }
    }

    // 4. Hover edge preview (lighter than selection — 1.2x brightness)
    if (interactionState.hoveredNode !== null && source && target) {
      const isConnectedToHovered = source === interactionState.hoveredNode || target === interactionState.hoveredNode;
      if (isConnectedToHovered) {
        return {
          ...data,
          color: brightenColor(data.color as string || '#4a4a4a', 1.2),
          size: (data.size as number || 1) * 2,
        } as Partial<EdgeDisplayData>;
      }
    }

    // 5. Circular edge coloring
    if (data.isCircular) {
      return { ...data, color: '#EF4444', size: 2 } as Partial<EdgeDisplayData>;
    }

    // 6. Default — apply edge weight for visual hierarchy
    const baseWeight = (data.weight as number) || 1.0;
    return { ...data, size: Math.max(0.5, baseWeight) } as Partial<EdgeDisplayData>;
  }

  // Expose imperative API
  export function focusNode(nodeId: string) {
    if (!renderer) return;
    const pos = renderer.getNodeDisplayData(nodeId);
    if (!pos) return;
    renderer.getCamera().animate({ x: pos.x, y: pos.y, ratio: 0.3 }, { duration: 500 });
  }

  export function setSearchHighlight(query: string) {
    searchQuery = query;
    renderer?.refresh();
  }

  export function getCameraState(): CameraState | null {
    if (!renderer) return null;
    return renderer.getCamera().getState();
  }

  export function setCameraState(state: { x: number; y: number; ratio: number }) {
    if (!renderer) return;
    renderer.getCamera().animate(state, { duration: 400 });
  }

  export function zoomIn() {
    if (!renderer) return;
    const cam = renderer.getCamera();
    const s = cam.getState();
    cam.animate({ x: s.x, y: s.y, ratio: s.ratio * 0.8 }, { duration: 200 });
  }

  export function zoomOut() {
    if (!renderer) return;
    const cam = renderer.getCamera();
    const s = cam.getState();
    cam.animate({ x: s.x, y: s.y, ratio: s.ratio * 1.25 }, { duration: 200 });
  }

  export function fitToViewport() {
    if (!renderer) return;
    renderer.getCamera().animatedReset({ duration: 400 });
  }

  export async function refreshGraph() {
    if (!graph || !renderer) return;
    try {
      const camState = renderer.getCamera().getState();

      const newGraph = await loadGraph(granularity);
      // Copy positions from old graph where nodes overlap
      graph.forEachNode((node) => {
        if (newGraph.hasNode(node)) {
          newGraph.setNodeAttribute(node, 'x', graph!.getNodeAttribute(node, 'x') as number);
          newGraph.setNodeAttribute(node, 'y', graph!.getNodeAttribute(node, 'y') as number);
        }
      });

      // Stop old layout
      if (layout) {
        stopLayout(layout);
        layout = null;
      }

      // Update graph reference and re-attach
      graph = newGraph;
      renderer.setGraph(newGraph);
      layout = startLayout(newGraph, () => {
        layoutRunning = false;
        onLayoutChange?.(false);
      });
      layoutRunning = true;
      onLayoutChange?.(true);

      // Restore camera
      renderer.getCamera().animate(camState, { duration: 200 });
    } catch (e) {
      console.error('Failed to refresh graph:', e);
    }
  }

  /**
   * Handle layout Play/Pause toggle.
   */
  function handleLayoutToggle() {
    if (!graph) return;

    if (layoutRunning) {
      // Pause: stop the running layout
      if (layout) {
        stopLayout(layout);
        layout = null;
      }
      layoutRunning = false;
      onLayoutChange?.(false);
    } else {
      // Play: restart layout from current positions
      if (layout) {
        stopLayout(layout);
        layout = null;
      }
      layout = startLayout(graph, () => {
        layoutRunning = false;
        onLayoutChange?.(false);
      });
      layoutRunning = true;
      onLayoutChange?.(true);
    }
  }

  export function toggleLayout() {
    handleLayoutToggle();
  }

  /**
   * Switch granularity: kill Sigma + layout, load new graph, recreate Sigma.
   */
  async function switchGranularity(newGranularity: string) {
    if (!container) return;

    // Save camera state
    const camState = renderer?.getCamera().getState();
    savePositions();

    // Kill existing instances
    if (layout) {
      stopLayout(layout);
      layout = null;
    }
    if (renderer) {
      renderer.kill();
      renderer = null;
    }

    // Load new graph
    try {
      graph = await loadGraph(newGranularity);
      const hasPositions = restorePositions(graph);

      renderer = new Sigma(graph, container, {
        defaultNodeColor: '#6b6090',
        defaultEdgeColor: '#4a4060',
        labelColor: { color: '#f0eeff' },
        labelRenderedSizeThreshold: 6,
        labelFont: "'Outfit', system-ui, sans-serif",
        labelSize: 12,
        labelWeight: '500',
        renderEdgeLabels: false,
        hideEdgesOnMove: false,
        zIndex: true,
        defaultDrawNodeHover: customDrawNodeHover,
        defaultEdgeType: 'curved',
        edgeProgramClasses: { curved: EdgeCurveProgram },
        nodeReducer,
        edgeReducer,
        minEdgeThickness: 1,
        enableEdgeEvents: true,
        labelDensity: 0.1,
        labelGridCellSize: 70,
        minCameraRatio: 0.002,
        maxCameraRatio: 50,
      });

      // Re-render canvas labels after Outfit font loads from CDN
      document.fonts.ready.then(() => renderer?.refresh());

      // Restore camera state if available
      if (camState) {
        renderer.getCamera().setState(camState);
      }

      // Start layout (seedPositions handles initial positions if not restored).
      // After layout completes, auto-fit camera so all nodes are visible.
      if (!hasPositions) {
        layout = startLayout(graph, () => {
          layoutRunning = false;
          onLayoutChange?.(false);
          renderer?.getCamera().animatedReset({ duration: 600 });
        });
        layoutRunning = true;
        onLayoutChange?.(true);
      } else if (!camState) {
        // Positions restored but no previous camera state — fit to view
        renderer.getCamera().animatedReset({ duration: 400 });
      }

      attachSigmaHandlers();

      // Refresh depth filter for new graph
      updateDepthFilter();
    } catch (e) {
      console.error('Failed to switch granularity:', e);
    }
  }

  function attachSigmaHandlers() {
    if (!renderer) return;

    renderer.on('downNode', (e) => {
      isDragging = true;
      draggedNode = e.node;
      renderer!.getCamera().disable();
    });

    renderer.getMouseCaptor().on('mousemovebody', (e) => {
      if (!isDragging || !draggedNode || !renderer) return;
      const pos = renderer.viewportToGraph(e);
      graph!.setNodeAttribute(draggedNode, 'x', pos.x);
      graph!.setNodeAttribute(draggedNode, 'y', pos.y);
    });

    renderer.getMouseCaptor().on('mouseup', () => {
      isDragging = false;
      draggedNode = null;
      renderer!.getCamera().enable();
    });

    renderer.on('clickNode', (e) => {
      const node = e.node;

      // Pre-compute 1-hop and 2-hop neighbor sets for O(1) reducer lookups
      // (Pitfall 6 in RESEARCH.md: avoid doing this inside nodeReducer per-frame)
      const oneHop = new Set(graph!.neighbors(node));
      const twoHop = new Set<string>();
      oneHop.forEach((n) =>
        graph!.neighbors(n).forEach((nn) => {
          if (nn !== node && !oneHop.has(nn)) twoHop.add(nn);
        }),
      );
      interactionState.oneHopNeighbors = oneHop;
      interactionState.twoHopNeighbors = twoHop;

      renderer!.refresh({ skipIndexation: true });
      onNodeClick?.(node);
    });

    renderer.on('clickStage', () => {
      // Deselect: clear neighbor sets, notify parent
      interactionState.oneHopNeighbors = new Set();
      interactionState.twoHopNeighbors = new Set();
      renderer!.refresh({ skipIndexation: true });
      onNodeClick?.(null as unknown as string);
    });

    renderer.on('rightClickNode', (e) => {
      const event = e.event as { x?: number; y?: number };
      const x = event?.x ?? 0;
      const y = event?.y ?? 0;
      onNodeRightClick?.(e.node, x, y);
    });

    renderer.on('enterNode', (e) => {
      if (!graph) return;
      interactionState.hoveredNode = e.node;
      interactionState.oneHopNeighbors = new Set(graph.neighbors(e.node));
      renderer!.refresh({ skipIndexation: true });
    });

    renderer.on('leaveNode', () => {
      interactionState.hoveredNode = null;
      // Only clear neighbor sets if no selection is active
      if (selectedNode === null) {
        interactionState.oneHopNeighbors = new Set();
        interactionState.twoHopNeighbors = new Set();
      }
      renderer!.refresh({ skipIndexation: true });
    });
  }

  // React to granularity prop changes after first mount
  let hasMounted = false;
  $effect(() => {
    const g = granularity;
    if (hasMounted) {
      switchGranularity(g);
    }
  });

  // React to depth/selectedNode changes for depth filter
  $effect(() => {
    const _d = depth;
    const _s = selectedNode;
    updateDepthFilter();
  });

  // React to selectedNode changes: pre-compute neighbor sets for reducer
  $effect(() => {
    const sn = selectedNode;
    if (sn && graph?.hasNode(sn)) {
      const oneHop = new Set(graph.neighbors(sn));
      const twoHop = new Set<string>();
      oneHop.forEach((n) =>
        graph!.neighbors(n).forEach((nn) => {
          if (nn !== sn && !oneHop.has(nn)) twoHop.add(nn);
        }),
      );
      interactionState.oneHopNeighbors = oneHop;
      interactionState.twoHopNeighbors = twoHop;
    } else {
      interactionState.oneHopNeighbors = new Set();
      interactionState.twoHopNeighbors = new Set();
    }
    renderer?.refresh({ skipIndexation: true });
  });

  onMount(async () => {
    isLoading = true;
    loadError = null;
    try {
      graph = await loadGraph(granularity);

      // Restore saved positions if available; seedPositions handles fresh layout
      const hasPositions = restorePositions(graph);

      renderer = new Sigma(graph, container, {
        defaultNodeColor: '#6b6090',
        defaultEdgeColor: '#4a4060',
        labelColor: { color: '#f0eeff' },
        labelRenderedSizeThreshold: 6,
        labelFont: "'Outfit', system-ui, sans-serif",
        labelSize: 12,
        labelWeight: '500',
        renderEdgeLabels: false,
        hideEdgesOnMove: false,
        zIndex: true,
        defaultDrawNodeHover: customDrawNodeHover,
        defaultEdgeType: 'curved',
        edgeProgramClasses: { curved: EdgeCurveProgram },
        nodeReducer,
        edgeReducer,
        minEdgeThickness: 1,
        enableEdgeEvents: true,
        labelDensity: 0.1,
        labelGridCellSize: 70,
        minCameraRatio: 0.002,
        maxCameraRatio: 50,
      });

      // Re-render canvas labels after Outfit font loads from CDN
      document.fonts.ready.then(() => renderer?.refresh());

      // Start layout if no saved positions (seedPositions handles initial positions).
      // After layout completes, auto-fit camera so all nodes are visible.
      if (!hasPositions) {
        layout = startLayout(graph, () => {
          layoutRunning = false;
          onLayoutChange?.(false);
          // Fit all nodes into view after layout settles
          renderer?.getCamera().animatedReset({ duration: 600 });
        });
        layoutRunning = true;
        onLayoutChange?.(true);
      } else {
        // Positions were restored — fit to viewport so user sees the full graph
        renderer.getCamera().animatedReset({ duration: 400 });
      }

      attachSigmaHandlers();
      hasMounted = true;
    } catch (e) {
      console.error('Failed to initialize graph:', e);
      loadError = e instanceof Error ? e.message : 'Failed to load graph';
    } finally {
      isLoading = false;
    }
  });

  onDestroy(() => {
    savePositions();
    if (layout) {
      stopLayout(layout);
      layout = null;
    }
    if (renderer) {
      renderer.kill();
      renderer = null;
    }
  });
</script>

<div
  bind:this={container}
  class="graph-container"
  role="application"
  aria-label="Interactive code dependency graph"
></div>

{#if isLoading && !loadError}
  <div class="graph-overlay">
    <div class="graph-spinner" aria-label="Loading graph...">
      <svg width="32" height="32" viewBox="0 0 32 32" fill="none">
        <circle cx="16" cy="16" r="13" stroke="var(--color-border)" stroke-width="3"/>
        <path d="M16 3 A13 13 0 0 1 29 16" stroke="var(--color-accent)" stroke-width="3" stroke-linecap="round"/>
      </svg>
      <span class="graph-loading-text">Loading graph...</span>
    </div>
  </div>
{/if}

{#if loadError}
  <div class="graph-overlay">
    <div class="graph-error">
      <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="var(--color-danger, #EF4444)" stroke-width="2">
        <circle cx="12" cy="12" r="10"/>
        <line x1="12" y1="8" x2="12" y2="12"/>
        <line x1="12" y1="16" x2="12.01" y2="16"/>
      </svg>
      <p class="graph-error-title">Failed to load graph</p>
      <p class="graph-error-detail">{loadError}</p>
    </div>
  </div>
{/if}

<style>
  .graph-container {
    width: 100%;
    height: 100%;
    position: absolute;
    inset: 0;
    background: var(--color-bg-primary);
  }

  .graph-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    pointer-events: none;
    z-index: 5;
  }

  .graph-spinner {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
  }

  .graph-spinner svg {
    animation: spin 1s linear infinite;
  }

  .graph-loading-text {
    font-size: 13px;
    color: var(--color-text-muted);
  }

  .graph-error {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    background: var(--color-bg-panel);
    border: 1px solid var(--color-border);
    border-radius: 10px;
    padding: 24px 32px;
    max-width: 360px;
    text-align: center;
    pointer-events: all;
  }

  .graph-error-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--color-text-primary);
    margin: 0;
  }

  .graph-error-detail {
    font-size: 12px;
    color: var(--color-text-muted);
    font-family: 'SF Mono', 'Fira Code', monospace;
    margin: 0;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }
</style>
