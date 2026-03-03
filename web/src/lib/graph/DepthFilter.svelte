<script lang="ts">
  interface Props {
    value: number | null;
    onChange: (depth: number | null) => void;
    disabled?: boolean;
  }

  let { value, onChange, disabled = false }: Props = $props();

  const options: Array<{ label: string; value: number | null }> = [
    { label: '1', value: 1 },
    { label: '2', value: 2 },
    { label: '3', value: 3 },
    { label: 'All', value: null },
  ];
</script>

<div class="depth-filter" class:disabled role="group" aria-label="Neighborhood depth filter">
  <span class="depth-label">Depth</span>
  {#each options as option}
    <button
      class="depth-btn"
      class:active={value === option.value}
      onclick={() => !disabled && onChange(option.value)}
      disabled={disabled}
      aria-pressed={value === option.value}
    >
      {option.label}
    </button>
  {/each}
</div>

<style>
  .depth-filter {
    display: inline-flex;
    align-items: center;
    background: var(--color-bg-panel, #1a1a2e);
    border: 1px solid var(--color-border, #2a2a3e);
    border-radius: 6px;
    padding: 2px;
    gap: 2px;
  }

  .depth-filter.disabled {
    opacity: 0.4;
    pointer-events: none;
  }

  .depth-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--color-text-muted, #6b7280);
    padding: 4px 8px 4px 6px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .depth-btn {
    padding: 4px 10px;
    font-size: 12px;
    font-weight: 500;
    color: var(--color-text-muted, #6b7280);
    background: transparent;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    transition: background 150ms ease, color 150ms ease;
    font-family: inherit;
  }

  .depth-btn:hover:not(:disabled) {
    color: var(--color-text-primary, #f5f5f5);
    background: rgba(255, 255, 255, 0.06);
  }

  .depth-btn.active {
    background: var(--color-accent, #3b82f6);
    color: #fff;
  }

  .depth-btn:disabled {
    cursor: not-allowed;
  }
</style>
