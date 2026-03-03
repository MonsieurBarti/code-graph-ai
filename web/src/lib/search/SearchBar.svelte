<script lang="ts">
  import { searchSymbols } from '../api';
  import type { SearchResult } from '../types';

  interface Props {
    onSelect: (result: SearchResult) => void;
    visible: boolean;
    onClose: () => void;
  }

  let { onSelect, visible, onClose }: Props = $props();

  let query = $state('');
  let results: SearchResult[] = $state([]);
  let selectedIndex = $state(0);
  let isLoading = $state(false);
  let searchInput: HTMLInputElement | undefined = $state(undefined);

  // Debounce timer
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  // Kind -> icon label
  function kindIcon(kind: string): string {
    const map: Record<string, string> = {
      function: 'f',
      method: 'm',
      class: 'C',
      struct: 'S',
      interface: 'I',
      trait: 'T',
      enum: 'E',
      constant: 'c',
      variable: 'v',
      type: 't',
      module: 'M',
      file: 'F',
    };
    return map[kind.toLowerCase()] ?? '?';
  }

  // Kind -> color
  function kindColor(kind: string): string {
    const map: Record<string, string> = {
      function: '#8B5CF6',
      method: '#8B5CF6',
      class: '#3B82F6',
      struct: '#3B82F6',
      interface: '#10B981',
      trait: '#10B981',
      enum: '#F59E0B',
      constant: '#F59E0B',
      component: '#EC4899',
      file: '#6B7280',
      module: '#6B7280',
    };
    return map[kind.toLowerCase()] ?? '#6B7280';
  }

  async function doSearch(q: string) {
    if (!q.trim()) {
      results = [];
      return;
    }
    isLoading = true;
    try {
      results = await searchSymbols(q, 20);
      selectedIndex = 0;
    } catch {
      results = [];
    } finally {
      isLoading = false;
    }
  }

  function handleInput() {
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => doSearch(query), 200);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      onClose();
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      selectedIndex = Math.min(selectedIndex + 1, results.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      selectedIndex = Math.max(selectedIndex - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (results[selectedIndex]) {
        selectResult(results[selectedIndex]);
      }
    }
  }

  function selectResult(result: SearchResult) {
    onSelect(result);
    query = '';
    results = [];
  }

  // Focus input when visible changes
  $effect(() => {
    if (visible && searchInput) {
      searchInput.focus();
    }
    if (!visible) {
      query = '';
      results = [];
    }
  });

  // Shorten display path
  function shortPath(path: string): string {
    const parts = path.replace(/\\/g, '/').split('/');
    if (parts.length <= 3) return path;
    return `.../${parts.slice(-2).join('/')}`;
  }
</script>

{#if visible}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class="search-overlay"
    role="dialog"
    aria-label="Search symbols"
    aria-modal="true"
  >
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="search-backdrop"
      onclick={onClose}
    ></div>

    <div class="search-modal">
      <!-- Input -->
      <div class="search-input-row">
        <svg class="search-icon" width="14" height="14" viewBox="0 0 14 14" fill="none">
          <circle cx="6" cy="6" r="4.5" stroke="currentColor" stroke-width="1.5"/>
          <path d="M9.5 9.5L12 12" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
        </svg>
        <input
          bind:this={searchInput}
          bind:value={query}
          class="search-input"
          type="text"
          placeholder="Search symbols, files..."
          oninput={handleInput}
          onkeydown={handleKeydown}
          autocomplete="off"
          spellcheck="false"
        />
        {#if query}
          <button class="search-clear" onclick={() => { query = ''; results = []; searchInput?.focus(); }} aria-label="Clear">
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
              <path d="M1 1l10 10M11 1L1 11" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
            </svg>
          </button>
        {/if}
        <kbd class="search-kbd">Esc</kbd>
      </div>

      <!-- Results -->
      {#if isLoading}
        <div class="search-status">Searching...</div>
      {:else if results.length === 0 && query.trim()}
        <div class="search-status">No results for "{query}"</div>
      {:else if results.length > 0}
        <ul class="search-results" role="listbox">
          {#each results as result, idx (result.symbol + result.file + result.line)}
            <li
              class="search-result {idx === selectedIndex ? 'result-selected' : ''}"
              role="option"
              aria-selected={idx === selectedIndex}
              onmouseenter={() => (selectedIndex = idx)}
            >
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="result-inner"
                onclick={() => selectResult(result)}
              >
                <span
                  class="result-kind"
                  style="background: {kindColor(result.kind)}22; color: {kindColor(result.kind)}; border-color: {kindColor(result.kind)}44;"
                >
                  {kindIcon(result.kind)}
                </span>
                <div class="result-info">
                  <span class="result-name">{result.symbol}</span>
                  <span class="result-path">{shortPath(result.file)}:{result.line}</span>
                </div>
                <span class="result-kind-label">{result.kind}</span>
              </div>
            </li>
          {/each}
        </ul>
      {:else if !query.trim()}
        <div class="search-hint-row">
          <span>Type to search symbols and files</span>
          <div class="search-shortcuts">
            <span><kbd>↑↓</kbd> Navigate</span>
            <span><kbd>Enter</kbd> Select</span>
            <span><kbd>Esc</kbd> Close</span>
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .search-overlay {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 80px;
    pointer-events: all;
  }

  .search-backdrop {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    backdrop-filter: blur(4px);
  }

  .search-modal {
    position: relative;
    z-index: 1;
    width: 560px;
    max-width: calc(100vw - 40px);
    background: var(--color-bg-panel, #1a1a1c);
    border: 1px solid var(--color-border);
    border-radius: 10px;
    box-shadow: 0 24px 80px rgba(0, 0, 0, 0.6);
    overflow: hidden;
  }

  /* Input row */
  .search-input-row {
    display: flex;
    align-items: center;
    padding: 10px 14px;
    gap: 8px;
    border-bottom: 1px solid var(--color-border);
  }

  .search-icon {
    color: var(--color-text-muted);
    flex-shrink: 0;
  }

  .search-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    font-size: 14px;
    color: var(--color-text-primary);
    min-width: 0;
  }

  .search-input::placeholder {
    color: var(--color-text-muted);
  }

  .search-clear {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    background: transparent;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    color: var(--color-text-muted);
    flex-shrink: 0;
    padding: 0;
    transition: color 100ms;
  }

  .search-clear:hover {
    color: var(--color-text-primary);
  }

  .search-kbd {
    font-size: 10px;
    padding: 2px 5px;
    background: rgba(255, 255, 255, 0.07);
    border: 1px solid var(--color-border);
    border-radius: 4px;
    color: var(--color-text-muted);
    flex-shrink: 0;
    font-family: inherit;
  }

  /* Status messages */
  .search-status {
    padding: 14px 16px;
    font-size: 13px;
    color: var(--color-text-muted);
    text-align: center;
  }

  /* Results list */
  .search-results {
    list-style: none;
    margin: 0;
    padding: 4px 0;
    max-height: 360px;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: rgba(255, 255, 255, 0.1) transparent;
  }

  .search-results::-webkit-scrollbar {
    width: 4px;
  }

  .search-results::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.1);
  }

  .search-result {
    padding: 0;
  }

  .result-inner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 14px;
    cursor: pointer;
    transition: background 80ms ease;
  }

  .result-selected .result-inner {
    background: rgba(59, 130, 246, 0.1);
  }

  .result-kind {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border-radius: 4px;
    border: 1px solid;
    font-size: 10px;
    font-weight: 700;
    font-family: 'JetBrains Mono', monospace;
    flex-shrink: 0;
  }

  .result-info {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }

  .result-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .result-path {
    font-size: 11px;
    color: var(--color-text-muted);
    font-family: 'JetBrains Mono', monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .result-kind-label {
    font-size: 10px;
    color: var(--color-text-muted);
    text-transform: capitalize;
    flex-shrink: 0;
  }

  /* Hint row (empty state) */
  .search-hint-row {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 16px;
    font-size: 12px;
    color: var(--color-text-muted);
  }

  .search-shortcuts {
    display: flex;
    gap: 12px;
    font-size: 11px;
    color: var(--color-text-muted);
  }

  .search-shortcuts span {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  kbd {
    font-size: 9px;
    padding: 1px 4px;
    background: rgba(255, 255, 255, 0.07);
    border: 1px solid var(--color-border);
    border-radius: 3px;
    font-family: inherit;
  }
</style>
