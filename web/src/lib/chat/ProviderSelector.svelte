<script lang="ts">
  import { setProvider, setApiKey, getAuthStatus, fetchOllamaModels } from '../api';
  import type { AuthStatus, OllamaModel } from '../types';

  interface Props {
    authStatus: AuthStatus | null;
    onProviderChange?: (provider: string, model: string) => void;
  }

  let { authStatus = $bindable(null), onProviderChange }: Props = $props();

  let dropdownOpen = $state(false);
  let apiKeyModalOpen = $state(false);
  let apiKeyInput = $state('');
  let saving = $state(false);
  let saveError = $state('');

  // Ollama model picker
  let ollamaModels = $state<OllamaModel[]>([]);
  let loadingModels = $state(false);
  let modelPickerOpen = $state(false);

  let displayLabel = $derived.by(() => {
    if (!authStatus) return 'Chat';
    const providerName = authStatus.provider === 'claude' ? 'Claude' : 'Ollama';
    return `${providerName} · ${authStatus.model}`;
  });

  function formatSize(bytes: number): string {
    const gb = bytes / 1e9;
    if (gb >= 1) return `${gb.toFixed(1)}GB`;
    return `${(bytes / 1e6).toFixed(0)}MB`;
  }

  async function loadOllamaModels() {
    loadingModels = true;
    try {
      ollamaModels = await fetchOllamaModels();
    } catch {
      ollamaModels = [];
    } finally {
      loadingModels = false;
    }
  }

  async function selectProvider(provider: 'claude' | 'ollama') {
    dropdownOpen = false;
    if (provider === 'claude' && authStatus && !authStatus.configured) {
      apiKeyModalOpen = true;
      return;
    }
    if (provider === 'ollama') {
      await loadOllamaModels();
      modelPickerOpen = true;
      return;
    }
    try {
      await setProvider(provider);
      const updated = await getAuthStatus();
      authStatus = updated;
      onProviderChange?.(updated.provider, updated.model);
    } catch (e) {
      console.error('Failed to switch provider:', e);
    }
  }

  async function selectOllamaModel(modelName: string) {
    modelPickerOpen = false;
    try {
      await setProvider('ollama', modelName);
      const updated = await getAuthStatus();
      authStatus = updated;
      onProviderChange?.(updated.provider, updated.model);
    } catch (e) {
      console.error('Failed to set Ollama model:', e);
    }
  }

  async function saveApiKey() {
    if (!apiKeyInput.trim()) return;
    saving = true;
    saveError = '';
    try {
      await setApiKey(apiKeyInput.trim());
      await setProvider('claude');
      const updated = await getAuthStatus();
      authStatus = updated;
      onProviderChange?.(updated.provider, updated.model);
      apiKeyModalOpen = false;
      apiKeyInput = '';
    } catch (e) {
      saveError = e instanceof Error ? e.message : 'Failed to save API key';
    } finally {
      saving = false;
    }
  }

  function closeDropdown(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (!target.closest('.provider-selector') && !target.closest('.modal-backdrop')) {
      dropdownOpen = false;
    }
  }
</script>

<svelte:window onclick={closeDropdown} />

<div class="provider-selector">
  <button
    class="provider-btn"
    onclick={(e) => { e.stopPropagation(); dropdownOpen = !dropdownOpen; }}
    title="Switch AI provider"
  >
    <span class="provider-label">{displayLabel}</span>
    <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
      <polyline points="6 9 12 15 18 9"/>
    </svg>
  </button>

  {#if dropdownOpen}
    <div class="provider-dropdown">
      <button
        class="dropdown-item {authStatus?.provider === 'claude' ? 'active' : ''}"
        onclick={() => selectProvider('claude')}
      >
        <span class="item-name">Claude</span>
        {#if authStatus?.provider === 'claude' && authStatus?.configured}
          <span class="item-check">✓</span>
        {/if}
        {#if authStatus?.provider === 'claude' && !authStatus?.configured}
          <span class="item-warning">No key</span>
        {/if}
      </button>
      <button
        class="dropdown-item {authStatus?.provider === 'ollama' ? 'active' : ''}"
        onclick={() => selectProvider('ollama')}
      >
        <span class="item-name">Ollama</span>
        <span class="item-hint">Select model →</span>
      </button>
      {#if authStatus?.provider === 'claude' && !authStatus?.configured}
        <div class="dropdown-separator"></div>
        <button class="dropdown-item set-key-item" onclick={() => { dropdownOpen = false; apiKeyModalOpen = true; }}>
          Set API Key...
        </button>
      {/if}
    </div>
  {/if}
</div>

<!-- Ollama model picker modal -->
{#if modelPickerOpen}
  <div class="modal-backdrop" onclick={() => { modelPickerOpen = false; }}>
    <div class="modal-box model-picker-box" onclick={(e) => e.stopPropagation()}>
      <div class="modal-title">Select Ollama Model</div>
      {#if loadingModels}
        <div class="model-loading">Loading models...</div>
      {:else if ollamaModels.length === 0}
        <div class="model-empty">
          <p>No models found. Is Ollama running?</p>
          <code>ollama serve</code>
          <p class="model-hint">Then pull a model:</p>
          <code>ollama pull qwen2.5-coder:7b</code>
        </div>
      {:else}
        <div class="model-list">
          {#each ollamaModels as model}
            <button
              class="model-item {authStatus?.provider === 'ollama' && authStatus?.model === model.name ? 'active' : ''}"
              onclick={() => selectOllamaModel(model.name)}
            >
              <span class="model-name">{model.name}</span>
              <span class="model-size">{formatSize(model.size)}</span>
            </button>
          {/each}
        </div>
      {/if}
      <div class="modal-actions">
        <button class="btn-cancel" onclick={() => { modelPickerOpen = false; }}>
          Cancel
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- API key modal -->
{#if apiKeyModalOpen}
  <div class="modal-backdrop" onclick={() => { apiKeyModalOpen = false; }}>
    <div class="modal-box" onclick={(e) => e.stopPropagation()}>
      <div class="modal-title">Set Claude API Key</div>
      <input
        type="password"
        bind:value={apiKeyInput}
        placeholder="sk-ant-..."
        class="api-key-input"
        onkeydown={(e) => { if (e.key === 'Enter') saveApiKey(); }}
        autocomplete="off"
      />
      {#if saveError}
        <div class="modal-error">{saveError}</div>
      {/if}
      <div class="modal-actions">
        <button class="btn-cancel" onclick={() => { apiKeyModalOpen = false; apiKeyInput = ''; saveError = ''; }}>
          Cancel
        </button>
        <button class="btn-save" onclick={saveApiKey} disabled={saving || !apiKeyInput.trim()}>
          {saving ? 'Saving...' : 'Save'}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .provider-selector {
    position: relative;
  }

  .provider-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--color-text-muted);
    font-size: 12px;
    padding: 4px 6px;
    border-radius: 4px;
    transition: color 100ms ease, background 100ms ease;
  }

  .provider-btn:hover {
    color: var(--color-text-primary);
    background: rgba(255, 255, 255, 0.06);
  }

  .provider-label {
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .provider-dropdown {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    background: var(--color-bg-elevated);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    z-index: 100;
    min-width: 200px;
    overflow: hidden;
    padding: 4px;
  }

  .dropdown-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 8px 10px;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--color-text-primary);
    font-size: 13px;
    border-radius: 4px;
    text-align: left;
    transition: background 100ms ease;
  }

  .dropdown-item:hover {
    background: rgba(255, 255, 255, 0.06);
  }

  .dropdown-item.active {
    color: var(--color-accent, #7c5cfc);
  }

  .item-check {
    color: var(--color-accent, #7c5cfc);
    font-size: 12px;
  }

  .item-hint {
    font-size: 11px;
    color: var(--color-text-muted);
  }

  .item-warning {
    font-size: 11px;
    color: #e67e22;
  }

  .dropdown-separator {
    height: 1px;
    background: var(--color-border);
    margin: 4px 0;
  }

  .set-key-item {
    color: var(--color-accent, #7c5cfc);
  }

  /* Modal */
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    z-index: 200;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .modal-box {
    background: var(--color-bg-surface);
    border: 1px solid var(--color-border);
    border-radius: 12px;
    padding: 20px;
    width: 360px;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.5);
  }

  .modal-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--color-text-primary);
    margin-bottom: 12px;
  }

  .api-key-input {
    width: 100%;
    box-sizing: border-box;
    padding: 8px 10px;
    background: var(--color-bg-elevated);
    border: 1px solid var(--color-border);
    border-radius: 6px;
    color: var(--color-text-primary);
    font-size: 13px;
    font-family: 'JetBrains Mono', monospace;
    outline: none;
    transition: border-color 150ms ease;
  }

  .api-key-input:focus {
    border-color: var(--color-accent, #7c5cfc);
  }

  .modal-error {
    margin-top: 8px;
    font-size: 12px;
    color: #e74c3c;
  }

  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }

  .btn-cancel, .btn-save {
    padding: 6px 16px;
    border-radius: 6px;
    font-size: 13px;
    cursor: pointer;
    border: none;
    transition: opacity 150ms ease;
  }

  .btn-cancel {
    background: transparent;
    color: var(--color-text-muted);
    border: 1px solid var(--color-border);
  }

  .btn-cancel:hover {
    color: var(--color-text-primary);
  }

  .btn-save {
    background: var(--color-accent, #7c5cfc);
    color: white;
  }

  .btn-save:hover:not(:disabled) {
    opacity: 0.85;
  }

  .btn-save:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* Model picker */
  .model-picker-box {
    width: 400px;
    max-height: 480px;
    display: flex;
    flex-direction: column;
  }

  .model-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 320px;
    overflow-y: auto;
  }

  .model-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 10px 12px;
    background: transparent;
    border: 1px solid transparent;
    cursor: pointer;
    color: var(--color-text-primary);
    font-size: 13px;
    border-radius: 6px;
    text-align: left;
    transition: background 100ms ease, border-color 100ms ease;
  }

  .model-item:hover {
    background: rgba(255, 255, 255, 0.06);
    border-color: var(--color-border);
  }

  .model-item.active {
    border-color: var(--color-accent, #7c5cfc);
    background: rgba(124, 92, 252, 0.08);
  }

  .model-name {
    font-family: 'JetBrains Mono', monospace;
    font-size: 13px;
  }

  .model-size {
    font-size: 11px;
    color: var(--color-text-muted);
    flex-shrink: 0;
    margin-left: 12px;
  }

  .model-loading {
    padding: 24px;
    text-align: center;
    color: var(--color-text-muted);
    font-size: 13px;
  }

  .model-empty {
    padding: 16px;
    text-align: center;
    color: var(--color-text-muted);
    font-size: 13px;
  }

  .model-empty code {
    display: block;
    margin: 8px auto;
    padding: 6px 12px;
    background: var(--color-bg-elevated);
    border-radius: 4px;
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
    color: var(--color-text-primary);
    width: fit-content;
  }

  .model-hint {
    margin-top: 12px;
    margin-bottom: 0;
  }
</style>
