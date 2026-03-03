<script lang="ts">
  interface Props {
    nodeCount: number;
    edgeCount: number;
    onSearchClick: () => void;
  }
  let { nodeCount, edgeCount, onSearchClick }: Props = $props();
</script>

<header class="app-header">
  <div class="header-brand">
    <!-- Graph node icon -->
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="var(--color-accent)" stroke-width="2">
      <circle cx="12" cy="12" r="3"/>
      <circle cx="5" cy="7" r="2"/>
      <circle cx="19" cy="7" r="2"/>
      <circle cx="5" cy="17" r="2"/>
      <circle cx="19" cy="17" r="2"/>
      <line x1="12" y1="9" x2="5" y2="8"/>
      <line x1="12" y1="9" x2="19" y2="8"/>
      <line x1="12" y1="15" x2="5" y2="16"/>
      <line x1="12" y1="15" x2="19" y2="16"/>
    </svg>
    <span class="header-title">code-graph</span>
  </div>

  <button class="header-search-trigger" onclick={onSearchClick} aria-label="Search (Cmd+K)">
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
      <circle cx="11" cy="11" r="8"/>
      <line x1="21" y1="21" x2="16.65" y2="16.65"/>
    </svg>
    <span class="header-search-text">Search</span>
    <kbd class="header-kbd">⌘K</kbd>
  </button>

  <div class="header-stats">
    <span>{nodeCount.toLocaleString()} nodes</span>
    <span class="header-stat-sep">·</span>
    <span>{edgeCount.toLocaleString()} edges</span>
  </div>
</header>

<style>
  .app-header {
    height: 38px;
    display: flex;
    align-items: center;
    padding: 0 12px;
    gap: 12px;
    background: var(--color-bg-deep);
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
  }

  .header-brand {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }

  .header-title {
    font-weight: 600;
    font-size: 13px;
    color: var(--color-text-primary);
    letter-spacing: -0.02em;
  }

  .header-search-trigger {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: 6px;
    border: 1px solid var(--color-border);
    background: var(--color-bg-surface);
    color: var(--color-text-muted);
    font-size: 12px;
    cursor: pointer;
    transition: color 100ms ease, background 100ms ease, border-color 100ms ease;
    flex-shrink: 0;
  }

  .header-search-trigger:hover {
    color: var(--color-text-primary);
    background: var(--color-bg-elevated);
    border-color: var(--color-border-hover, var(--color-accent));
  }

  .header-search-text {
    color: inherit;
  }

  .header-kbd {
    display: inline-flex;
    align-items: center;
    padding: 1px 4px;
    border-radius: 3px;
    border: 1px solid var(--color-border);
    background: var(--color-bg-deep);
    font-size: 10px;
    color: var(--color-text-muted);
    font-family: inherit;
    line-height: 1.4;
  }

  .header-stats {
    font-size: 11px;
    color: var(--color-text-muted);
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }

  .header-stat-sep {
    opacity: 0.5;
  }
</style>
