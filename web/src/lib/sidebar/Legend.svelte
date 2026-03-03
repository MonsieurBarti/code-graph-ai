<script lang="ts">
  // Node kind -> color mappings for symbol granularity (17 types matching backend node_color())
  const symbolKinds = [
    { label: 'Function', color: '#7c5cfc' },
    { label: 'Class', color: '#4f8ef7' },
    { label: 'Struct', color: '#5ba3f5' },
    { label: 'Interface', color: '#2dba8c' },
    { label: 'Trait', color: '#26a87e' },
    { label: 'Method', color: '#9b7fe8' },
    { label: 'Enum', color: '#c4853a' },
    { label: 'Component', color: '#c0537a' },
    { label: 'Type', color: '#7a9fd4' },
    { label: 'Property', color: '#8897a8' },
    { label: 'Variable', color: '#8aa0b0' },
    { label: 'Const', color: '#96a8b8' },
    { label: 'Static', color: '#7d8fa0' },
    { label: 'Macro', color: '#a07ab0' },
    { label: 'Module', color: '#5e8bc0' },
    { label: 'File', color: '#6b6090' },
  ] as const;

  // Language -> color mappings for file granularity (matches backend language_color)
  const languageColors = [
    { label: 'TypeScript', color: '#3178C6' },
    { label: 'JavaScript', color: '#E8D44D' },
    { label: 'Rust', color: '#DEA584' },
    { label: 'Python', color: '#3572A5' },
    { label: 'Go', color: '#00ADD8' },
    { label: 'Svelte', color: '#FF3E00' },
    { label: 'HTML', color: '#E34C26' },
    { label: 'CSS', color: '#563D7C' },
    { label: 'Java', color: '#B07219' },
    { label: 'Other', color: '#6B7280' },
  ] as const;

  // Structural node types for file granularity (folder synthesis from Plan 01)
  const fileNodeTypes = [
    { label: 'Folder', color: '#6366f1' },
    { label: 'Module', color: '#5e8bc0' },
  ] as const;

  // Edge type -> color mappings (muted palette matching Plan 02 backend)
  const fileEdgeTypes = [
    { label: 'Imports', color: '#1d4ed8' },
    { label: 'Contains', color: '#2d5a3d' },
    { label: 'Circular', color: '#EF4444' },
  ] as const;

  const symbolEdgeTypes = [
    { label: 'Imports', color: '#1d4ed8' },
    { label: 'Calls', color: '#7c3aed' },
    { label: 'Extends', color: '#c2410c' },
    { label: 'Contains', color: '#2d5a3d' },
    { label: 'Implements', color: '#be185d' },
    { label: 'Circular', color: '#EF4444' },
  ] as const;

  interface Props {
    granularity?: 'file' | 'symbol' | 'package';
    visibleEdgeTypes?: Set<string>;
    onToggleEdgeType?: (type: string) => void;
  }
  let { granularity = 'file', visibleEdgeTypes, onToggleEdgeType }: Props = $props();

  let edgeItems = $derived(granularity === 'file' ? fileEdgeTypes : symbolEdgeTypes);
</script>

<div class="legend">
  <div class="legend-separator"></div>

  {#if granularity === 'file'}
    <div class="legend-section">
      <div class="legend-title">Languages</div>
      {#each languageColors as item}
        <div class="legend-row">
          <span class="legend-dot" style="background: {item.color};"></span>
          <span class="legend-label">{item.label}</span>
        </div>
      {/each}
    </div>
    <div class="legend-section">
      <div class="legend-title">Node Types</div>
      {#each fileNodeTypes as item}
        <div class="legend-row">
          <span class="legend-dot" style="background: {item.color};"></span>
          <span class="legend-label">{item.label}</span>
        </div>
      {/each}
    </div>
  {:else}
    <div class="legend-section">
      <div class="legend-title">Nodes</div>
      {#each symbolKinds as item}
        <div class="legend-row">
          <span class="legend-dot" style="background: {item.color};"></span>
          <span class="legend-label">{item.label}</span>
        </div>
      {/each}
    </div>
  {/if}

  <div class="legend-section">
    <div class="legend-title">Edges</div>
    {#each edgeItems as item}
      {@const isVisible = !visibleEdgeTypes || visibleEdgeTypes.has(item.label)}
      <div
        class="legend-row legend-row-edge {isVisible ? '' : 'legend-row-disabled'}"
        onclick={() => onToggleEdgeType?.(item.label)}
        role="button"
        tabindex="0"
        onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') onToggleEdgeType?.(item.label); }}
        aria-pressed={isVisible}
        aria-label="Toggle {item.label} edges"
        title="{isVisible ? 'Hide' : 'Show'} {item.label} edges"
      >
        <span class="legend-line" style="background: {isVisible ? item.color : '#555'};"></span>
        <span class="legend-label" style="color: {isVisible ? '' : '#555'};">{item.label}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  .legend {
    padding-bottom: 12px;
  }

  .legend-separator {
    height: 1px;
    background: var(--color-border);
    margin: 0 0 10px;
  }

  .legend-section {
    padding: 0 12px;
    margin-bottom: 10px;
  }

  .legend-title {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--color-text-muted);
    margin-bottom: 6px;
  }

  .legend-row {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 2px 0;
  }

  .legend-row-edge {
    cursor: pointer;
    border-radius: 3px;
    padding: 2px 4px;
    margin: 0 -4px;
    transition: background 100ms ease, opacity 100ms ease;
  }

  .legend-row-edge:hover {
    background: rgba(255, 255, 255, 0.06);
  }

  .legend-row-disabled {
    opacity: 0.35;
  }

  .legend-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .legend-line {
    width: 16px;
    height: 2px;
    border-radius: 1px;
    flex-shrink: 0;
  }

  .legend-label {
    font-size: 11px;
    color: var(--color-text-muted);
  }
</style>
