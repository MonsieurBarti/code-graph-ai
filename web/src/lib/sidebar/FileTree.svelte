<script lang="ts">
  import type { GraphNode } from '../types';

  interface Props {
    graphNodes: GraphNode[];
    onFileSelect: (path: string) => void;
    selectedFile: string | null;
  }

  let { graphNodes, onFileSelect, selectedFile }: Props = $props();

  // Tree node representation
  interface TreeNode {
    name: string;
    path: string; // full path to this node
    isFile: boolean;
    children: Map<string, TreeNode>;
    expanded: boolean;
  }

  function makeTreeNode(name: string, path: string, isFile: boolean): TreeNode {
    return { name, path, isFile, children: new Map(), expanded: true };
  }

  // Build a tree from flat file paths
  function buildTree(nodes: GraphNode[]): Map<string, TreeNode> {
    const root = new Map<string, TreeNode>();

    for (const node of nodes) {
      const path = node.attributes.path;
      if (!path) continue;

      // Normalize separators
      const parts = path.replace(/\\/g, '/').split('/').filter(Boolean);
      if (parts.length === 0) continue;

      let currentMap = root;
      let builtPath = '';

      for (let i = 0; i < parts.length; i++) {
        const part = parts[i];
        builtPath = builtPath ? `${builtPath}/${part}` : part;
        const isLast = i === parts.length - 1;

        if (!currentMap.has(part)) {
          currentMap.set(part, makeTreeNode(part, builtPath, isLast));
        } else if (isLast) {
          // Mark existing node as a file
          const existing = currentMap.get(part)!;
          existing.isFile = true;
        }

        const child = currentMap.get(part)!;
        currentMap = child.children;
      }
    }

    return root;
  }

  // Collect unique file paths from graph nodes (filter duplicates for file-granularity)
  function collectFilePaths(nodes: GraphNode[]): GraphNode[] {
    const seen = new Set<string>();
    const result: GraphNode[] = [];
    for (const node of nodes) {
      const path = node.attributes.path;
      if (path && !seen.has(path)) {
        seen.add(path);
        result.push(node);
      }
    }
    return result;
  }

  // Language icon by extension
  function getLangIcon(filename: string): string {
    const ext = filename.split('.').pop()?.toLowerCase() ?? '';
    const icons: Record<string, string> = {
      ts: 'TS',
      tsx: 'TSX',
      js: 'JS',
      jsx: 'JSX',
      rs: 'RS',
      py: 'PY',
      go: 'GO',
      json: 'JS',
      toml: 'TM',
      yaml: 'YM',
      yml: 'YM',
      css: 'CS',
      html: 'HT',
      svelte: 'SV',
      md: 'MD',
    };
    return icons[ext] ?? 'FI';
  }

  // Get color for language
  function getLangColor(filename: string): string {
    const ext = filename.split('.').pop()?.toLowerCase() ?? '';
    const colors: Record<string, string> = {
      ts: '#3B82F6',
      tsx: '#3B82F6',
      js: '#F59E0B',
      jsx: '#F59E0B',
      rs: '#F97316',
      py: '#10B981',
      go: '#06B6D4',
      json: '#6B7280',
      toml: '#6B7280',
      yaml: '#6B7280',
      yml: '#6B7280',
      css: '#EC4899',
      html: '#EF4444',
      svelte: '#EC4899',
    };
    return colors[ext] ?? '#6B7280';
  }

  // Toggle folder expand/collapse
  const expandState = new Map<string, boolean>();

  function isExpanded(path: string): boolean {
    if (!expandState.has(path)) return true; // default expanded
    return expandState.get(path)!;
  }

  function toggleFolder(path: string) {
    expandState.set(path, !isExpanded(path));
    // Force Svelte to re-render
    treeVersion++;
  }

  let treeVersion = $state(0);

  // Build tree reactively
  let tree = $derived.by(() => {
    void treeVersion; // track for forced re-renders
    const filePaths = collectFilePaths(graphNodes);
    return buildTree(filePaths);
  });

  // Render tree nodes recursively — returns flat array of render items
  interface RenderItem {
    type: 'folder' | 'file';
    name: string;
    path: string;
    depth: number;
    hasChildren: boolean;
    isExpanded: boolean;
  }

  function flattenTree(map: Map<string, TreeNode>, depth: number): RenderItem[] {
    const items: RenderItem[] = [];
    // Sort: folders first, then files, alphabetically
    const sorted = Array.from(map.values()).sort((a, b) => {
      if (a.isFile !== b.isFile) return a.isFile ? 1 : -1;
      return a.name.localeCompare(b.name);
    });

    for (const node of sorted) {
      const hasChildren = node.children.size > 0;
      const expanded = isExpanded(node.path);

      if (!node.isFile || hasChildren) {
        // It's a folder (or folder+file hybrid)
        items.push({
          type: 'folder',
          name: node.name,
          path: node.path,
          depth,
          hasChildren,
          isExpanded: expanded,
        });
        if (expanded && hasChildren) {
          items.push(...flattenTree(node.children, depth + 1));
        }
      } else {
        items.push({
          type: 'file',
          name: node.name,
          path: node.path,
          depth,
          hasChildren: false,
          isExpanded: false,
        });
      }
    }
    return items;
  }

  let renderItems = $derived.by(() => {
    void treeVersion;
    return flattenTree(tree, 0);
  });
</script>

<div class="file-tree">
  <div class="tree-header">
    <span class="tree-title">Files</span>
    <span class="tree-count">{collectFilePaths(graphNodes).length}</span>
  </div>

  <div class="tree-body">
    {#each renderItems as item (item.path + item.type)}
      {#if item.type === 'folder'}
        <button
          class="tree-item tree-folder"
          style="padding-left: {8 + item.depth * 14}px;"
          onclick={() => toggleFolder(item.path)}
          aria-expanded={item.isExpanded}
        >
          <span class="tree-chevron {item.isExpanded ? 'chevron-open' : ''}">
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
              <path d="M3 2l4 3-4 3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
            </svg>
          </span>
          <span class="folder-icon">
            <svg width="13" height="13" viewBox="0 0 13 13" fill="none">
              {#if item.isExpanded}
                <path d="M1 4a1 1 0 011-1h2.5L6 4.5H11a1 1 0 011 1V10a1 1 0 01-1 1H2a1 1 0 01-1-1V4z" fill="rgba(100,160,255,0.3)" stroke="#5B9BD5" stroke-width="1"/>
              {:else}
                <path d="M1 3.5a1 1 0 011-1h2.5L6 4H11a1 1 0 011 1v5a1 1 0 01-1 1H2a1 1 0 01-1-1V3.5z" fill="rgba(100,160,255,0.2)" stroke="#5B9BD5" stroke-width="1"/>
              {/if}
            </svg>
          </span>
          <span class="item-name">{item.name}</span>
        </button>
      {:else}
        <button
          class="tree-item tree-file {selectedFile === item.path ? 'tree-item-selected' : ''}"
          style="padding-left: {8 + item.depth * 14 + 20}px;"
          onclick={() => onFileSelect(item.path)}
        >
          <span
            class="file-badge"
            style="background: {getLangColor(item.name)}22; color: {getLangColor(item.name)}; border-color: {getLangColor(item.name)}44;"
          >
            {getLangIcon(item.name)}
          </span>
          <span class="item-name">{item.name}</span>
        </button>
      {/if}
    {/each}

    {#if renderItems.length === 0}
      <div class="tree-empty">No files in graph</div>
    {/if}
  </div>
</div>

<style>
  .file-tree {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .tree-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px 6px;
    flex-shrink: 0;
  }

  .tree-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--color-text-muted);
  }

  .tree-count {
    font-size: 10px;
    color: var(--color-text-muted);
    background: rgba(255, 255, 255, 0.07);
    padding: 1px 5px;
    border-radius: 10px;
  }

  .tree-body {
    flex: 1;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: rgba(255, 255, 255, 0.1) transparent;
  }

  .tree-body::-webkit-scrollbar {
    width: 4px;
  }

  .tree-body::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.1);
    border-radius: 2px;
  }

  .tree-item {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    padding-top: 3px;
    padding-bottom: 3px;
    padding-right: 8px;
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
    color: var(--color-text-muted);
    font-size: 12px;
    transition: background 80ms ease, color 80ms ease;
    border-radius: 0;
    white-space: nowrap;
    overflow: hidden;
  }

  .tree-item:hover {
    background: rgba(255, 255, 255, 0.05);
    color: var(--color-text-primary);
  }

  .tree-item-selected {
    background: rgba(59, 130, 246, 0.12) !important;
    color: var(--color-text-primary) !important;
  }

  .tree-chevron {
    display: flex;
    align-items: center;
    flex-shrink: 0;
    color: var(--color-text-muted);
    transition: transform 150ms ease;
    width: 10px;
  }

  .chevron-open {
    transform: rotate(90deg);
  }

  .folder-icon {
    display: flex;
    align-items: center;
    flex-shrink: 0;
  }

  .file-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 8px;
    font-weight: 700;
    width: 20px;
    height: 14px;
    border-radius: 2px;
    border: 1px solid;
    flex-shrink: 0;
    letter-spacing: 0.02em;
    font-family: 'JetBrains Mono', monospace;
  }

  .item-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }

  .tree-empty {
    padding: 16px 12px;
    color: var(--color-text-muted);
    font-size: 11px;
    text-align: center;
  }
</style>
