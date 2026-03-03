<script lang="ts">
  interface Props {
    value: 'file' | 'symbol' | 'package';
    onChange: (granularity: 'file' | 'symbol' | 'package') => void;
  }

  let { value, onChange }: Props = $props();

  const options: Array<{ label: string; value: 'file' | 'symbol' | 'package' }> = [
    { label: 'File', value: 'file' },
    { label: 'Symbol', value: 'symbol' },
    { label: 'Package', value: 'package' },
  ];
</script>

<div class="granularity-toggle" role="group" aria-label="Graph granularity">
  {#each options as option}
    <button
      class="toggle-btn"
      class:active={value === option.value}
      onclick={() => onChange(option.value)}
      aria-pressed={value === option.value}
    >
      {option.label}
    </button>
  {/each}
</div>

<style>
  .granularity-toggle {
    display: inline-flex;
    background: var(--color-bg-panel, #1a1a2e);
    border: 1px solid var(--color-border, #2a2a3e);
    border-radius: 6px;
    padding: 2px;
    gap: 2px;
  }

  .toggle-btn {
    padding: 4px 12px;
    font-size: 12px;
    font-weight: 500;
    color: var(--color-text-muted, #6b7280);
    background: transparent;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    transition: background 150ms ease, color 150ms ease;
    white-space: nowrap;
    font-family: inherit;
  }

  .toggle-btn:hover {
    color: var(--color-text-primary, #f5f5f5);
    background: rgba(255, 255, 255, 0.06);
  }

  .toggle-btn.active {
    background: var(--color-accent, #3b82f6);
    color: #fff;
  }
</style>
