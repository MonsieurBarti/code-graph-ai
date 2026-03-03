<script lang="ts">
  import type { NodeAttributes } from '../types';

  interface Props {
    x: number;
    y: number;
    visible: boolean;
    nodeKey: string;
    nodeAttributes: NodeAttributes | null;
    onAction: (action: string, nodeKey: string) => void;
  }

  let { x, y, visible, nodeKey, nodeAttributes, onAction }: Props = $props();

  interface MenuItem {
    label: string;
    action: string;
    icon: string;
  }

  const menuItems: MenuItem[] = [
    { label: 'Show references', action: 'references', icon: '→' },
    { label: 'Show impact', action: 'impact', icon: '↕' },
    { label: 'Focus neighborhood', action: 'focus', icon: '◎' },
    { label: 'Copy path', action: 'copy-path', icon: '⎘' },
    { label: 'Open in editor', action: 'open-editor', icon: '✎' },
  ];

  function handleAction(e: MouseEvent, action: string) {
    e.stopPropagation();
    onAction(action, nodeKey);
  }

  // Clamp position to keep menu in viewport
  const MENU_WIDTH = 200;
  const MENU_HEIGHT = 200;

  let clampedX = $derived(
    typeof window !== 'undefined' ? Math.min(x, window.innerWidth - MENU_WIDTH - 8) : x
  );
  let clampedY = $derived(
    typeof window !== 'undefined' ? Math.min(y, window.innerHeight - MENU_HEIGHT - 8) : y
  );
</script>

{#if visible}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="context-menu"
    style="left: {clampedX}px; top: {clampedY}px;"
    onclick={(e) => e.stopPropagation()}
    oncontextmenu={(e) => e.preventDefault()}
  >
    {#if nodeAttributes}
      <div class="menu-header">
        <span class="menu-node-name">{nodeAttributes.label || nodeKey}</span>
        <span class="menu-node-kind">{nodeAttributes.kind}</span>
      </div>
      <div class="menu-divider"></div>
    {/if}

    {#each menuItems as item}
      <button
        class="menu-item"
        onclick={(e) => handleAction(e, item.action)}
      >
        <span class="menu-icon">{item.icon}</span>
        <span class="menu-label">{item.label}</span>
      </button>
    {/each}
  </div>
{/if}

<style>
  .context-menu {
    position: fixed;
    z-index: 200;
    background: var(--color-bg-panel, #1a1a1c);
    border: 1px solid var(--color-border, #2a2a3e);
    border-radius: 6px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4), 0 2px 8px rgba(0, 0, 0, 0.2);
    padding: 4px;
    min-width: 180px;
    user-select: none;
  }

  .menu-header {
    padding: 6px 10px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .menu-node-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--color-text-primary, #f5f5f5);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 160px;
  }

  .menu-node-kind {
    font-size: 10px;
    color: var(--color-text-muted, #6b7280);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .menu-divider {
    height: 1px;
    background: var(--color-border, #2a2a3e);
    margin: 2px 0;
  }

  .menu-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 10px;
    background: transparent;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    text-align: left;
    font-size: 13px;
    color: var(--color-text-primary, #f5f5f5);
    font-family: inherit;
    transition: background 100ms ease;
  }

  .menu-item:hover {
    background: rgba(255, 255, 255, 0.08);
  }

  .menu-icon {
    font-size: 12px;
    color: var(--color-text-muted, #6b7280);
    width: 14px;
    text-align: center;
    flex-shrink: 0;
  }

  .menu-label {
    flex: 1;
  }
</style>
