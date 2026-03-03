<script lang="ts">
  interface Props {
    nodeKey: string | null;
    nodeLabel: string;
    nodeKind: string;
    nodeColor: string;
    onClear: () => void;
  }
  let { nodeKey, nodeLabel, nodeKind, nodeColor, onClear }: Props = $props();
</script>

{#if nodeKey !== null}
  <div
    class="selection-bar"
    style="--node-color: {nodeColor}; border-color: {nodeColor}40;"
  >
    <span class="selection-dot"></span>
    <span class="selection-name">{nodeLabel}</span>
    <span class="selection-kind">{nodeKind}</span>
    <button class="selection-clear" onclick={onClear} aria-label="Clear selection" title="Clear selection (Esc)">
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
        <path d="M1.5 1.5l7 7M8.5 1.5l-7 7" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
      </svg>
    </button>
  </div>
{/if}

<style>
  .selection-bar {
    position: absolute;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 10;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 16px;
    background: rgba(18, 18, 28, 0.85);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    border: 1px solid var(--node-color, #6B7280);
    border-radius: 8px;
    pointer-events: all;
    white-space: nowrap;
  }

  .selection-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--node-color, #6B7280);
    flex-shrink: 0;
    animation: pulse-dot 2s ease-in-out infinite;
  }

  @keyframes pulse-dot {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  .selection-name {
    font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
    font-size: 13px;
    color: #f5f5f5;
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .selection-kind {
    font-size: 11px;
    color: var(--color-text-muted, #6B7280);
    text-transform: capitalize;
  }

  .selection-clear {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border-radius: 4px;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--color-text-muted, #6B7280);
    padding: 0;
    flex-shrink: 0;
    transition: color 100ms ease, background 100ms ease;
  }

  .selection-clear:hover {
    color: #f5f5f5;
    background: rgba(255, 255, 255, 0.1);
  }
</style>
