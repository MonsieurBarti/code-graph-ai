<script lang="ts">
  import Landing from './lib/Landing.svelte';
  import GraphCanvas from './lib/graph/GraphCanvas.svelte';
  import GranularityToggle from './lib/graph/GranularityToggle.svelte';
  import DepthFilter from './lib/graph/DepthFilter.svelte';
  import ContextMenu from './lib/graph/ContextMenu.svelte';
  import CodePanel from './lib/code/CodePanel.svelte';
  import FileTree from './lib/sidebar/FileTree.svelte';
  import Legend from './lib/sidebar/Legend.svelte';
  import SelectionBar from './lib/SelectionBar.svelte';
  import SearchBar from './lib/search/SearchBar.svelte';
  import DraggableDivider from './lib/layout/DraggableDivider.svelte';
  import Header from './lib/Header.svelte';
  import StatusBar from './lib/StatusBar.svelte';
  import { NavigationHistory, syncToUrl, syncFromUrl } from './lib/navigation';
  import { createWsClient } from './lib/ws';
  import { fetchGraph } from './lib/api';
  import { onDestroy } from 'svelte';
  import type { NodeAttributes, GraphNode, SearchResult } from './lib/types';
  import ChatPanel from './lib/chat/ChatPanel.svelte';

  interface OpenFile {
    path: string;
    symbolLine?: number;
    symbolLineEnd?: number;
  }

  // Root app state
  let view: 'landing' | 'graph' = $state('landing');
  let selectedNode: string | null = $state(null);
  let granularity: 'file' | 'symbol' | 'package' = $state('file');
  let depth: number | null = $state(null);

  // Edge type visibility state — all visible by default
  let visibleEdgeTypes: Set<string> = $state(new Set(['Imports', 'Calls', 'Extends', 'Contains', 'Implements', 'Circular']));

  // Code panel state
  let openFiles: OpenFile[] = $state([]);
  let activeFile: string | null = $state(null);

  // Sidebar state
  let sidebarCollapsed: boolean = $state(false);
  let sidebarWidth: number = $state(240);
  let codePanelWidth: number = $state(500);

  // Sidebar tab state
  let activeSidebarTab: 'explorer' | 'filters' = $state('explorer');

  // Code overlay collapsed state (48px icon strip vs full panel)
  let codePanelCollapsed: boolean = $state(false);

  // Chat panel state
  let chatOpen: boolean = $state(false);
  let chatCollapsed: boolean = $state(false);
  let chatPanelWidth: number = $state(400);

  // Search bar state
  let searchVisible: boolean = $state(false);

  // GraphCanvas imperative handle
  let graphCanvas: GraphCanvas | null = $state(null);

  // WebSocket client (created when graph view is active)
  let wsClient: { close: () => void } | null = null;

  // Graph node attributes — stored for file path lookup
  let nodeAttributesMap: Map<string, NodeAttributes> = $state(new Map());
  // Graph nodes as flat array — for FileTree
  let graphNodes: GraphNode[] = $state([]);

  // Edge count for StatusBar
  let graphEdgeCount: number = $state(0);

  // Layout running state for StatusBar (mirrored from GraphCanvas via onLayoutChange)
  let layoutRunning: boolean = $state(false);

  // Primary language derived from graphNodes
  let primaryLanguage = $derived.by(() => {
    if (graphNodes.length === 0) return '';
    const langCount: Record<string, number> = {};
    for (const node of graphNodes) {
      const lang = node.attributes.language;
      if (lang) langCount[lang] = (langCount[lang] || 0) + 1;
    }
    const sorted = Object.entries(langCount).sort((a, b) => b[1] - a[1]);
    return sorted[0]?.[0] || '';
  });

  async function loadGraphNodes() {
    try {
      const data = await fetchGraph(granularity);
      graphNodes = data.nodes;
      graphEdgeCount = data.edges.length;
      nodeAttributesMap = new Map(data.nodes.map((n) => [n.key, n.attributes]));
    } catch (e) {
      console.error('Failed to load graph nodes for sidebar:', e);
    }
  }

  // Derived selected file path for FileTree highlighting
  let selectedFilePath = $derived.by(() => {
    if (!selectedNode) return null;
    return nodeAttributesMap.get(selectedNode)?.path ?? null;
  });

  // Navigation history for Alt+Left / Alt+Right
  const navHistory = new NavigationHistory();

  function getCurrentNavState() {
    const camState = graphCanvas?.getCameraState();
    return {
      selectedNode,
      granularity,
      cameraX: camState?.x ?? 0,
      cameraY: camState?.y ?? 0,
      cameraRatio: camState?.ratio ?? 1,
    };
  }

  function pushNavState() {
    const state = getCurrentNavState();
    navHistory.push(state);
    syncToUrl(state);
  }

  function restoreNavState(state: { selectedNode: string | null; granularity: string; cameraX: number; cameraY: number; cameraRatio: number }) {
    if (state.granularity !== granularity) {
      granularity = state.granularity as 'file' | 'symbol' | 'package';
    }
    selectedNode = state.selectedNode ?? null;
    // Animate camera to restored position
    graphCanvas?.setCameraState({ x: state.cameraX, y: state.cameraY, ratio: state.cameraRatio });
    syncToUrl(state);
  }

  function enterGraph() {
    view = 'graph';
    // Start WebSocket client for live updates
    wsClient = createWsClient(() => {
      graphCanvas?.refreshGraph();
      // Reload node map on graph updates for FileTree
      loadGraphNodes();
    });
    // Load persisted panel widths from localStorage
    try {
      const saved = localStorage.getItem('panelWidths');
      if (saved) {
        const parsed = JSON.parse(saved) as { sidebar?: number; codePanel?: number };
        if (parsed.sidebar && parsed.sidebar >= 200 && parsed.sidebar <= 400) {
          sidebarWidth = parsed.sidebar;
        }
        if (parsed.codePanel && parsed.codePanel >= 300) {
          codePanelWidth = parsed.codePanel;
        }
      }
    } catch {
      // Ignore storage errors
    }

    // Restore from URL if present (deep linking)
    const urlState = syncFromUrl();
    if (urlState.granularity && urlState.granularity !== 'file') {
      granularity = urlState.granularity as 'file' | 'symbol' | 'package';
    }
    if (urlState.selectedNode) {
      selectedNode = urlState.selectedNode;
    }

    // Load initial graph node data for FileTree
    loadGraphNodes();
  }

  // Context menu state
  let contextMenu: { visible: boolean; x: number; y: number; nodeKey: string } = $state({
    visible: false,
    x: 0,
    y: 0,
    nodeKey: '',
  });

  function handleToggleEdgeType(type: string) {
    const newSet = new Set(visibleEdgeTypes);
    if (newSet.has(type)) {
      newSet.delete(type);
    } else {
      newSet.add(type);
    }
    visibleEdgeTypes = newSet;
  }

  function handleNodeClick(nodeKey: string | null) {
    if (!nodeKey) { selectedNode = null; return; }
    selectedNode = nodeKey;
    contextMenu = { ...contextMenu, visible: false };
    // Push to navigation history + sync URL
    // Use a small delay so camera state is current
    setTimeout(() => pushNavState(), 50);
    // Look up node attributes to get file path and line numbers
    const attrs = nodeAttributesMap.get(nodeKey);
    if (attrs?.path) {
      openFileInPanel(attrs.path, attrs.line, attrs.lineEnd);
    }
  }

  function handleNodeRightClick(nodeKey: string, x: number, y: number) {
    selectedNode = nodeKey;
    contextMenu = { visible: true, x, y, nodeKey };
  }

  function handleContextMenuAction(action: string, nodeKey: string) {
    contextMenu = { ...contextMenu, visible: false };
    const attrs = nodeAttributesMap.get(nodeKey);

    switch (action) {
      case 'references':
        // Phase 22 territory — for now search and highlight
        if (attrs?.label) {
          graphCanvas?.setSearchHighlight(attrs.label);
        }
        break;
      case 'impact':
        // Phase 22 territory — for now search and highlight
        if (attrs?.label) {
          graphCanvas?.setSearchHighlight(attrs.label);
        }
        break;
      case 'focus':
        // Set depth filter to 2-hop on this node
        selectedNode = nodeKey;
        depth = 2;
        graphCanvas?.focusNode(nodeKey);
        break;
      case 'copy-path':
        if (attrs?.path) {
          navigator.clipboard.writeText(attrs.path).catch(() => {
            // Fallback: show in console
            console.log('Path:', attrs.path);
          });
        }
        break;
      case 'open-editor':
        if (attrs?.path) {
          const line = attrs.line || 1;
          const vscodeUrl = `vscode://file/${attrs.path}:${line}`;
          window.open(vscodeUrl, '_blank');
        }
        break;
    }
  }

  function handleGranularityChange(newGranularity: 'file' | 'symbol' | 'package') {
    granularity = newGranularity;
    // Reset depth when granularity changes
    depth = null;
    // Sync URL
    setTimeout(() => pushNavState(), 50);
  }

  function handleDepthChange(newDepth: number | null) {
    depth = newDepth;
  }

  function openFileInPanel(path: string, symbolLine?: number, symbolLineEnd?: number) {
    // Add to open files if not already present
    const existingIdx = openFiles.findIndex((f) => f.path === path);
    if (existingIdx >= 0) {
      // Update symbol line info
      openFiles[existingIdx] = { path, symbolLine, symbolLineEnd };
      openFiles = [...openFiles];
    } else {
      openFiles = [...openFiles, { path, symbolLine, symbolLineEnd }];
    }
    activeFile = path;
    codePanelCollapsed = false;
  }

  function handleCloseTab(path: string) {
    openFiles = openFiles.filter((f) => f.path !== path);
    if (activeFile === path) {
      activeFile = openFiles.length > 0 ? openFiles[openFiles.length - 1].path : null;
    }
  }

  function handleSelectTab(path: string) {
    activeFile = path;
  }

  function handleSymbolClick(symbolName: string) {
    // Search for the symbol in the graph by name
    graphCanvas?.setSearchHighlight(symbolName);
    // Find matching node and focus it
    if (nodeAttributesMap.size > 0) {
      for (const [nodeKey, attrs] of nodeAttributesMap) {
        if (attrs.label === symbolName) {
          selectedNode = nodeKey;
          graphCanvas?.focusNode(nodeKey);
          break;
        }
      }
    }
  }

  function handleFileSelect(path: string) {
    // Find the graph node for this file path
    for (const [nodeKey, attrs] of nodeAttributesMap) {
      if (attrs.path === path) {
        selectedNode = nodeKey;
        graphCanvas?.focusNode(nodeKey);
        break;
      }
    }
    openFileInPanel(path);
  }

  function handleSearchSelect(result: { symbol: string; kind: string; file: string; line: number }) {
    // Find the graph node for this result
    for (const [nodeKey, attrs] of nodeAttributesMap) {
      if (attrs.label === result.symbol && attrs.path === result.file) {
        selectedNode = nodeKey;
        graphCanvas?.focusNode(nodeKey);
        break;
      }
    }
    openFileInPanel(result.file, result.line, result.line);
    searchVisible = false;
  }

  // Citation click from chat panel — navigate to file/node in graph and open CodePanel
  function handleCitationClick(file: string, line: number) {
    // Find the graph node matching this file path
    const matchingKey = Array.from(nodeAttributesMap.entries()).find(([_key, attrs]) => {
      return attrs.path === file
        || attrs.label === file
        || file.endsWith(attrs.path)
        || attrs.path.endsWith(file);
    })?.[0];
    if (matchingKey) {
      selectedNode = matchingKey;
      graphCanvas?.focusNode(matchingKey);
    }
    // Open the file in CodePanel at the referenced line
    openFileInPanel(file, line, line);
  }

  // Persist panel widths
  function savePanelWidths() {
    try {
      localStorage.setItem('panelWidths', JSON.stringify({ sidebar: sidebarWidth, codePanel: codePanelWidth }));
    } catch {
      // Ignore
    }
  }

  // Layout change callback from GraphCanvas
  function handleLayoutChange(running: boolean) {
    layoutRunning = running;
  }

  // Layout toggle button handler for StatusBar
  function handleLayoutToggle() {
    graphCanvas?.toggleLayout();
  }

  // Sidebar tab helper
  function openSidebarTab(tab: 'explorer' | 'filters') {
    activeSidebarTab = tab;
    sidebarCollapsed = false;
  }

  // Keyboard shortcuts
  function handleKeydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement;
    const isInput =
      target.tagName === 'INPUT' ||
      target.tagName === 'TEXTAREA' ||
      target.isContentEditable;

    if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
      e.preventDefault();
      searchVisible = !searchVisible;
      return;
    }
    if ((e.metaKey || e.ctrlKey) && e.key === 'p') {
      e.preventDefault();
      searchVisible = !searchVisible;
      return;
    }
    if ((e.metaKey || e.ctrlKey) && e.key === 'b') {
      e.preventDefault();
      sidebarCollapsed = !sidebarCollapsed;
      return;
    }
    if (e.key === 'Escape') {
      if (contextMenu.visible) {
        contextMenu = { ...contextMenu, visible: false };
      } else if (searchVisible) {
        searchVisible = false;
      } else if (selectedNode) {
        selectedNode = null;
      } else if (chatOpen) {
        chatOpen = false;
      } else if (openFiles.length > 0) {
        if (activeFile) handleCloseTab(activeFile);
      }
      return;
    }

    // Shortcuts that shouldn't fire when typing in an input
    if (isInput) return;

    // Granularity shortcuts
    if (e.key === '1') { granularity = 'file'; depth = null; setTimeout(() => pushNavState(), 50); }
    else if (e.key === '2') { granularity = 'symbol'; depth = null; setTimeout(() => pushNavState(), 50); }
    else if (e.key === '3') { granularity = 'package'; depth = null; setTimeout(() => pushNavState(), 50); }
    // Zoom shortcuts
    else if (e.key === '+' || e.key === '=') { graphCanvas?.zoomIn(); }
    else if (e.key === '-') { graphCanvas?.zoomOut(); }
    // Fit to viewport
    else if (e.key === 'f' || e.key === 'F') { graphCanvas?.fitToViewport(); }
    // Browser-style back/forward navigation
    else if (e.altKey && e.key === 'ArrowLeft') {
      e.preventDefault();
      const prev = navHistory.back();
      if (prev) restoreNavState(prev);
    }
    else if (e.altKey && e.key === 'ArrowRight') {
      e.preventDefault();
      const next = navHistory.forward();
      if (next) restoreNavState(next);
    }
    // Ctrl+Tab — switch between file tabs
    else if ((e.ctrlKey) && e.key === 'Tab') {
      e.preventDefault();
      if (openFiles.length > 1 && activeFile) {
        const idx = openFiles.findIndex((f) => f.path === activeFile);
        const nextIdx = (idx + 1) % openFiles.length;
        activeFile = openFiles[nextIdx].path;
      }
    }
  }

  // Computed sidebar display width
  let effectiveSidebarWidth = $derived(sidebarCollapsed ? 48 : sidebarWidth);
  let codePanelVisible = $derived(openFiles.length > 0);

  onDestroy(() => {
    wsClient?.close();
  });
</script>

<svelte:window onkeydown={handleKeydown} />

{#if view === 'landing'}
  <Landing onExplore={enterGraph} />
{:else}
  <div class="app-shell">
    <!-- Row 1: Header -->
    <Header
      nodeCount={graphNodes.length}
      edgeCount={graphEdgeCount}
      onSearchClick={() => { searchVisible = !searchVisible; }}
    />

    <!-- Row 2: Body (sidebar + graph area) -->
    <div class="app-body">
      <!-- Sidebar with icon rail -->
      <aside class="sidebar" style="width: {effectiveSidebarWidth}px;">
        <div class="icon-rail">
          <button
            class="rail-btn {activeSidebarTab === 'explorer' && !sidebarCollapsed ? 'active' : ''}"
            onclick={() => openSidebarTab('explorer')}
            aria-label="Explorer"
            title="Explorer"
          >
            <!-- Folder icon -->
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/>
            </svg>
          </button>
          <button
            class="rail-btn {activeSidebarTab === 'filters' && !sidebarCollapsed ? 'active' : ''}"
            onclick={() => openSidebarTab('filters')}
            aria-label="Filters"
            title="Filters"
          >
            <!-- Funnel icon -->
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <polygon points="22 3 2 3 10 12.46 10 19 14 21 14 12.46 22 3"/>
            </svg>
          </button>
          <button
            class="rail-btn {chatOpen ? 'active' : ''}"
            onclick={() => { chatOpen = !chatOpen; chatCollapsed = false; }}
            aria-label="Chat with AI"
            title="Chat with AI"
          >
            <!-- Chat bubble icon -->
            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
            </svg>
          </button>
        </div>

        {#if !sidebarCollapsed}
          <div class="sidebar-content">
            <div class="sidebar-header">
              <span class="sidebar-title">{activeSidebarTab === 'explorer' ? 'Explorer' : 'Filters'}</span>
              <button
                class="sidebar-collapse-btn"
                aria-label="Collapse sidebar"
                onclick={() => { sidebarCollapsed = true; }}
              >
                <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
                  <path d="M9 2L4 7l5 5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
                </svg>
              </button>
            </div>

            {#if activeSidebarTab === 'explorer'}
              <div class="sidebar-tree-area">
                <FileTree {graphNodes} onFileSelect={handleFileSelect} selectedFile={selectedFilePath} />
              </div>
            {:else}
              <!-- Filters tab: GranularityToggle, DepthFilter, Legend (node + edge types) -->
              <div class="sidebar-filters-area">
                <div class="filter-section">
                  <div class="filter-section-title">View</div>
                  <GranularityToggle value={granularity} onChange={handleGranularityChange} />
                </div>
                <div class="filter-section">
                  <div class="filter-section-title">Depth</div>
                  <DepthFilter value={depth} onChange={handleDepthChange} disabled={selectedNode === null} />
                </div>
                <Legend {granularity} {visibleEdgeTypes} onToggleEdgeType={handleToggleEdgeType} />
              </div>
            {/if}
          </div>
        {/if}
      </aside>

      <!-- Sidebar resize divider -->
      <DraggableDivider
        position="left"
        onResize={(delta) => {
          if (!sidebarCollapsed) {
            sidebarWidth = Math.min(400, Math.max(200, sidebarWidth + delta));
          }
        }}
        onResizeEnd={savePanelWidths}
      />

      <!-- Graph area (position: relative for overlay) -->
      <main class="graph-area">
        <SelectionBar
          nodeKey={selectedNode}
          nodeLabel={selectedNode ? (nodeAttributesMap.get(selectedNode)?.label ?? '') : ''}
          nodeKind={selectedNode ? (nodeAttributesMap.get(selectedNode)?.kind ?? '') : ''}
          nodeColor={selectedNode ? (nodeAttributesMap.get(selectedNode)?.color ?? '#6b6090') : '#6b6090'}
          onClear={() => { selectedNode = null; }}
        />

        <GraphCanvas
          bind:this={graphCanvas}
          {granularity}
          {depth}
          {selectedNode}
          {visibleEdgeTypes}
          onNodeClick={handleNodeClick}
          onNodeRightClick={handleNodeRightClick}
          onLayoutChange={handleLayoutChange}
        />

        <!-- Code panel overlay (left side of graph area) -->
        {#if codePanelVisible}
          <div
            class="code-overlay {codePanelCollapsed ? 'code-overlay-collapsed' : ''}"
            style="width: {codePanelCollapsed ? '48px' : codePanelWidth + 'px'};"
          >
            {#if codePanelCollapsed}
              <!-- Collapsed icon strip -->
              <div class="code-overlay-icons">
                {#each openFiles as file}
                  <button
                    class="code-overlay-icon-btn {activeFile === file.path ? 'active' : ''}"
                    onclick={() => { codePanelCollapsed = false; activeFile = file.path; }}
                    title={file.path.split('/').pop()}
                    aria-label="Open {file.path.split('/').pop()}"
                  >
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                      <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/>
                      <polyline points="14 2 14 8 20 8"/>
                    </svg>
                  </button>
                {/each}
                <button
                  class="code-overlay-expand-btn"
                  onclick={() => { codePanelCollapsed = false; }}
                  aria-label="Expand code panel"
                >
                  <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
                    <path d="M5 2l5 5-5 5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
                  </svg>
                </button>
              </div>
            {:else}
              <!-- Full code panel -->
              <div class="code-overlay-content">
                <div class="code-overlay-header">
                  <button
                    class="code-overlay-collapse-btn"
                    onclick={() => { codePanelCollapsed = true; }}
                    aria-label="Collapse code panel"
                  >
                    <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
                      <path d="M9 2L4 7l5 5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
                    </svg>
                  </button>
                </div>
                <CodePanel
                  {openFiles}
                  {activeFile}
                  onClose={handleCloseTab}
                  onSelectTab={handleSelectTab}
                  onSymbolClick={handleSymbolClick}
                />
              </div>
              <!-- Resize handle on right edge -->
              <DraggableDivider
                position="right"
                onResize={(delta) => {
                  const maxWidth = Math.floor(window.innerWidth * 0.6);
                  codePanelWidth = Math.min(maxWidth, Math.max(300, codePanelWidth + delta));
                }}
                onResizeEnd={savePanelWidths}
              />
            {/if}
          </div>
        {/if}

        <!-- Chat panel overlay (right side of graph area) -->
        {#if chatOpen}
          {#if chatCollapsed}
            <!-- 48px icon-only strip when collapsed -->
            <div class="chat-overlay chat-collapsed">
              <button
                class="rail-btn"
                onclick={() => { chatCollapsed = false; }}
                title="Expand chat"
                aria-label="Expand chat"
              >
                <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                  <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
                </svg>
              </button>
            </div>
          {:else}
            <div class="chat-overlay" style="width: {chatPanelWidth}px">
              <!-- DraggableDivider on left edge for resize -->
              <DraggableDivider
                position="left"
                onResize={(delta) => {
                  chatPanelWidth = Math.max(300, Math.min(800, chatPanelWidth - delta));
                }}
                onResizeEnd={savePanelWidths}
              />
              <ChatPanel
                onCitationClick={handleCitationClick}
                onClose={() => { chatOpen = false; }}
                onCollapse={() => { chatCollapsed = true; }}
              />
            </div>
          {/if}
        {/if}
      </main>
    </div>

    <!-- Row 3: StatusBar -->
    <StatusBar
      {layoutRunning}
      nodeCount={graphNodes.length}
      edgeCount={graphEdgeCount}
      {primaryLanguage}
      onToggleLayout={handleLayoutToggle}
    />
  </div>

  <!-- Context menu + Search overlay — outside app-shell for z-index -->
  <ContextMenu
    x={contextMenu.x}
    y={contextMenu.y}
    visible={contextMenu.visible}
    nodeKey={contextMenu.nodeKey}
    nodeAttributes={contextMenu.nodeKey ? (nodeAttributesMap.get(contextMenu.nodeKey) ?? null) : null}
    onAction={handleContextMenuAction}
  />

  <SearchBar
    visible={searchVisible}
    onSelect={handleSearchSelect}
    onClose={() => (searchVisible = false)}
  />
{/if}

<style>
  .app-shell {
    display: grid;
    grid-template-rows: 38px 1fr 28px;
    height: 100vh;
    overflow: hidden;
    background: var(--color-bg-primary);
  }

  /* Row 2: Body contains sidebar + graph area */
  .app-body {
    display: flex;
    overflow: hidden;
    min-height: 0;
  }

  /* Left sidebar — flex row: icon rail + content area */
  .sidebar {
    flex-shrink: 0;
    border-right: 1px solid var(--color-border);
    background: var(--color-bg-secondary);
    overflow: hidden;
    transition: width 200ms ease;
    min-width: 0;
    display: flex;
    flex-direction: row;
  }

  /* Icon rail (always visible, 48px wide) */
  .icon-rail {
    width: 48px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 8px 0;
    gap: 2px;
    border-right: 1px solid var(--color-border);
  }

  .rail-btn {
    width: 36px;
    height: 36px;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--color-text-muted);
    transition: color 100ms ease, background 100ms ease;
  }

  .rail-btn:hover {
    color: var(--color-text-primary);
    background: rgba(255, 255, 255, 0.06);
  }

  .rail-btn.active {
    color: var(--color-text-primary);
    background: var(--color-bg-elevated);
  }

  /* Sidebar content panel (hidden when collapsed) */
  .sidebar-content {
    flex: 1;
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
    min-width: 0;
  }

  .sidebar-tree-area {
    flex: 1;
    overflow: hidden;
    min-height: 0;
  }

  .sidebar-filters-area {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    padding: 8px 0;
  }

  .filter-section {
    padding: 8px 12px;
  }

  .filter-section-title {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--color-text-muted);
    margin-bottom: 6px;
  }

  .sidebar-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
  }

  .sidebar-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--color-text-muted);
  }

  .sidebar-collapse-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: 4px;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--color-text-muted);
    transition: color 100ms ease, background 100ms ease;
  }

  .sidebar-collapse-btn:hover {
    color: var(--color-text-primary);
    background: rgba(255, 255, 255, 0.06);
  }

  /* Center graph area */
  .graph-area {
    flex: 1;
    position: relative;
    overflow: hidden;
    min-width: 0;
  }

  /* Code overlay — absolute left on graph area */
  .code-overlay {
    position: absolute;
    left: 0;
    top: 0;
    height: 100%;
    z-index: 20;
    background: var(--color-bg-surface);
    border-right: 1px solid var(--color-border);
    box-shadow: 4px 0 24px rgba(0, 0, 0, 0.5);
    display: flex;
    flex-direction: row;
    overflow: hidden;
    transition: width 150ms ease;
  }

  .code-overlay-collapsed {
    width: 48px;
  }

  .code-overlay-content {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-width: 0;
  }

  .code-overlay-header {
    display: flex;
    align-items: center;
    padding: 4px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--color-border);
  }

  .code-overlay-icons {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 8px 0;
    gap: 4px;
    width: 100%;
  }

  .code-overlay-icon-btn {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 6px;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--color-text-muted);
    transition: color 100ms ease, background 100ms ease;
  }

  .code-overlay-icon-btn:hover,
  .code-overlay-icon-btn.active {
    color: var(--color-text-primary);
    background: var(--color-bg-elevated);
  }

  .code-overlay-expand-btn,
  .code-overlay-collapse-btn {
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--color-text-muted);
    transition: color 100ms ease, background 100ms ease;
  }

  .code-overlay-expand-btn:hover,
  .code-overlay-collapse-btn:hover {
    color: var(--color-text-primary);
    background: rgba(255, 255, 255, 0.06);
  }

  /* Chat overlay — absolute right on graph area, higher z-index than code-overlay */
  .chat-overlay {
    position: absolute;
    right: 0;
    top: 0;
    height: 100%;
    z-index: 24;
    background: var(--color-bg-surface);
    border-left: 1px solid var(--color-border);
    box-shadow: -4px 0 24px rgba(0, 0, 0, 0.5);
    display: flex;
    flex-direction: row;
    overflow: hidden;
    transition: width 150ms ease;
  }

  .chat-collapsed {
    width: 48px !important;
    align-items: flex-start;
    padding-top: 8px;
    justify-content: center;
    flex-direction: column;
  }
</style>
