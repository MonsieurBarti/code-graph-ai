<script lang="ts">
  import { onMount } from 'svelte';
  import { fetchStats } from './api';
  import type { StatsResponse } from './types';

  interface Props {
    onExplore: () => void;
  }

  let { onExplore }: Props = $props();

  let stats: StatsResponse | null = $state(null);
  let error: string | null = $state(null);
  let visible = $state(false);

  onMount(async () => {
    try {
      stats = await fetchStats();
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to load stats';
    }
    // Trigger fade-in after data is ready
    requestAnimationFrame(() => {
      visible = true;
    });
  });

  function projectName(root: string): string {
    const parts = root.replace(/\\/g, '/').split('/');
    return parts[parts.length - 1] || 'Project';
  }
</script>

<div class="flex h-screen items-center justify-center bg-[var(--color-bg-primary)]">
  <div
    class="card"
    style="opacity: {visible ? 1 : 0}; transform: translateY({visible ? 0 : 12}px); transition: opacity 300ms ease, transform 300ms ease;"
  >
    {#if error}
      <div class="error-state">
        <p class="text-[var(--color-text-secondary)] mb-2">Could not connect to the graph server.</p>
        <p class="text-[var(--color-text-muted)] text-sm font-mono">{error}</p>
        <p class="text-[var(--color-text-muted)] text-xs mt-3">Make sure the server is running: <code class="bg-[var(--color-bg-hover)] px-1 py-0.5 rounded">code-graph serve</code></p>
      </div>
    {:else if !stats}
      <!-- Loading skeleton -->
      <div class="skeleton-header"></div>
      <div class="skeleton-line w-40 mt-3"></div>
      <div class="stats-grid mt-6">
        <div class="stat-card skeleton-stat"></div>
        <div class="stat-card skeleton-stat"></div>
        <div class="stat-card skeleton-stat"></div>
      </div>
      <div class="skeleton-button mt-6"></div>
    {:else}
      <div class="header">
        <div class="project-icon">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <circle cx="12" cy="12" r="3"/>
            <circle cx="4" cy="6" r="2"/>
            <circle cx="20" cy="6" r="2"/>
            <circle cx="4" cy="18" r="2"/>
            <circle cx="20" cy="18" r="2"/>
            <line x1="12" y1="9" x2="4" y2="7"/>
            <line x1="12" y1="9" x2="20" y2="7"/>
            <line x1="12" y1="15" x2="4" y2="17"/>
            <line x1="12" y1="15" x2="20" y2="17"/>
          </svg>
        </div>
        <div>
          <h1 class="project-title">{projectName(stats.project_root)}</h1>
          <p class="project-path">{stats.project_root}</p>
        </div>
      </div>

      <div class="stats-grid mt-6">
        <div class="stat-card">
          <div class="stat-value">{stats.total_files.toLocaleString()}</div>
          <div class="stat-label">Files</div>
        </div>
        <div class="stat-card">
          <div class="stat-value">{stats.total_symbols.toLocaleString()}</div>
          <div class="stat-label">Symbols</div>
        </div>
        <div class="stat-card">
          <div class="stat-value">{stats.languages.length}</div>
          <div class="stat-label">Languages</div>
        </div>
      </div>

      {#if stats.languages.length > 0}
        <div class="lang-breakdown mt-4">
          {#each stats.languages as lang}
            <div class="lang-item">
              <span class="lang-name">{lang.language}</span>
              <span class="lang-count">{lang.files} files</span>
            </div>
          {/each}
        </div>
      {/if}

      <button class="explore-btn mt-6" onclick={onExplore}>
        Explore Graph
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="ml-2">
          <line x1="5" y1="12" x2="19" y2="12"/>
          <polyline points="12 5 19 12 12 19"/>
        </svg>
      </button>
    {/if}
  </div>
</div>

<style>
  .card {
    background: var(--color-bg-panel);
    border: 1px solid var(--color-border);
    border-radius: 12px;
    padding: 32px;
    width: 420px;
    max-width: calc(100vw - 48px);
    backdrop-filter: blur(8px);
    box-shadow: 0 4px 24px rgba(0, 0, 0, 0.4), 0 0 0 1px var(--color-border);
    transition: box-shadow 200ms ease;
  }

  .header {
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }

  .project-icon {
    width: 40px;
    height: 40px;
    background: var(--color-bg-hover);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--color-accent);
    flex-shrink: 0;
  }

  .project-title {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
    color: var(--color-text-primary);
    line-height: 1.2;
  }

  .project-path {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--color-text-muted);
    font-family: var(--font-mono);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 340px;
  }

  .stats-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }

  .stat-card {
    background: var(--color-bg-secondary);
    border: 1px solid var(--color-border-subtle);
    border-radius: 8px;
    padding: 12px;
    text-align: center;
  }

  .stat-value {
    font-size: 22px;
    font-weight: 700;
    color: var(--color-text-primary);
    line-height: 1;
  }

  .stat-label {
    font-size: 11px;
    color: var(--color-text-muted);
    margin-top: 4px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .lang-breakdown {
    display: flex;
    flex-direction: column;
    gap: 4px;
    border-top: 1px solid var(--color-border-subtle);
    padding-top: 12px;
  }

  .lang-item {
    display: flex;
    justify-content: space-between;
    font-size: 13px;
  }

  .lang-name {
    color: var(--color-text-secondary);
    text-transform: capitalize;
  }

  .lang-count {
    color: var(--color-text-muted);
  }

  .explore-btn {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--color-accent);
    color: white;
    border: none;
    border-radius: 8px;
    padding: 10px 20px;
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
    transition: background 150ms ease;
  }

  .explore-btn:hover {
    background: var(--color-accent-hover);
    box-shadow: 0 0 20px rgba(124, 92, 252, 0.3);
  }

  .error-state {
    text-align: center;
    padding: 8px 0;
  }

  .error-state code {
    font-family: var(--font-mono);
  }

  /* Skeleton styles */
  .skeleton-header {
    height: 40px;
    background: var(--color-bg-hover);
    border-radius: 6px;
    animation: pulse 1.5s ease-in-out infinite;
  }

  .skeleton-line {
    height: 14px;
    background: var(--color-bg-hover);
    border-radius: 4px;
    animation: pulse 1.5s ease-in-out infinite;
  }

  .skeleton-stat {
    height: 60px;
    animation: pulse 1.5s ease-in-out infinite;
  }

  .skeleton-button {
    height: 40px;
    background: var(--color-bg-hover);
    border-radius: 8px;
    animation: pulse 1.5s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }
</style>
