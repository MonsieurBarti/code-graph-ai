<script lang="ts">
  interface Props {
    layoutRunning: boolean;
    nodeCount: number;
    edgeCount: number;
    primaryLanguage: string;
    onToggleLayout: () => void;
  }
  let { layoutRunning, nodeCount, edgeCount, primaryLanguage, onToggleLayout }: Props = $props();
</script>

<footer class="app-statusbar">
  <span class="status-indicator">
    <span class="status-dot" class:running={layoutRunning}></span>
    <span>{layoutRunning ? 'Layout running' : 'Ready'}</span>
  </span>
  <span class="status-sep">|</span>
  <span>{nodeCount.toLocaleString()} nodes · {edgeCount.toLocaleString()} edges</span>
  {#if primaryLanguage}
    <span class="status-sep">|</span>
    <span>{primaryLanguage}</span>
  {/if}
  <button
    class="status-layout-btn"
    onclick={onToggleLayout}
    aria-label={layoutRunning ? 'Pause layout' : 'Run layout'}
  >
    {#if layoutRunning}
      <svg width="8" height="10" viewBox="0 0 8 10" fill="none">
        <rect width="3" height="10" rx="1" fill="currentColor"/>
        <rect x="5" width="3" height="10" rx="1" fill="currentColor"/>
      </svg>
    {:else}
      <svg width="8" height="10" viewBox="0 0 8 10" fill="none">
        <path d="M0 0 L8 5 L0 10 Z" fill="currentColor"/>
      </svg>
    {/if}
  </button>
</footer>

<style>
  .app-statusbar {
    height: 28px;
    display: flex;
    align-items: center;
    padding: 0 12px;
    gap: 8px;
    background: var(--color-bg-deep);
    border-top: 1px solid var(--color-border);
    font-size: 11px;
    color: var(--color-text-muted);
    flex-shrink: 0;
  }

  .status-indicator {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--color-text-muted);
    opacity: 0.5;
    flex-shrink: 0;
    transition: background 200ms ease, opacity 200ms ease;
  }

  .status-dot.running {
    background: var(--color-accent);
    opacity: 1;
    animation: pulse 1.5s ease-in-out infinite;
  }

  .status-sep {
    opacity: 0.3;
    user-select: none;
  }

  .status-layout-btn {
    width: 20px;
    height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    background: transparent;
    border: 1px solid var(--color-border);
    cursor: pointer;
    color: var(--color-text-muted);
    margin-left: auto;
    flex-shrink: 0;
    transition: color 100ms ease, background 100ms ease, border-color 100ms ease;
  }

  .status-layout-btn:hover {
    color: var(--color-text-primary);
    background: var(--color-bg-elevated);
    border-color: var(--color-accent);
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
  }
</style>
