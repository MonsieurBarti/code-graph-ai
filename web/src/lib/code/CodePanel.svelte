<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { fetchFile } from '../api';
  import { highlightCode, langFromPath } from './syntax';

  interface OpenFile {
    path: string;
    symbolLine?: number;
    symbolLineEnd?: number;
  }

  interface Props {
    openFiles: OpenFile[];
    activeFile: string | null;
    onClose: (path: string) => void;
    onSelectTab: (path: string) => void;
    onSymbolClick?: (symbolName: string) => void;
  }

  let {
    openFiles,
    activeFile,
    onClose,
    onSelectTab,
    onSymbolClick,
  }: Props = $props();

  // Cache: path -> highlighted HTML
  const contentCache = new Map<string, string>();
  // Cache: path -> raw content
  const rawCache = new Map<string, string>();

  let highlightedHtml = $state('');
  let isLoading = $state(false);
  let error = $state<string | null>(null);
  let codeContainer: HTMLDivElement | undefined = $state(undefined);

  // Get the current file's tab info
  function getCurrentFile(): OpenFile | undefined {
    return openFiles.find((f) => f.path === activeFile);
  }

  // Compute line count from raw content
  function getLineCount(path: string): number {
    const raw = rawCache.get(path);
    if (!raw) return 0;
    return raw.split('\n').length;
  }

  // Filename from path
  function filename(path: string): string {
    return path.split('/').pop() ?? path;
  }

  // File-type icon based on extension
  function getFileIcon(path: string): string {
    const ext = path.split('.').pop()?.toLowerCase() ?? '';
    const icons: Record<string, string> = {
      rs: '⬡',
      ts: 'T',
      tsx: '⚛',
      js: '⚡',
      jsx: '⚛',
      svelte: '⚙',
      py: '🐍',
      go: 'G',
      css: '🎨',
      html: '🌐',
      json: '{}',
      md: '📄',
      toml: '⚙',
      yaml: '⚙',
      yml: '⚙',
      sh: '$',
      bash: '$',
    };
    return icons[ext] ?? '📄';
  }

  // Shiki wraps everything in a <pre><code> with individual lines in spans.
  // We inject line-number columns and highlight via CSS variables after render.
  async function loadFile(path: string) {
    if (contentCache.has(path)) {
      highlightedHtml = contentCache.get(path)!;
      return;
    }

    isLoading = true;
    error = null;
    try {
      const content = await fetchFile(path);
      rawCache.set(path, content);
      const lang = langFromPath(path);
      const html = await highlightCode(content, lang);
      contentCache.set(path, html);
      highlightedHtml = html;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      highlightedHtml = '';
    } finally {
      isLoading = false;
    }
  }

  // Auto-scroll to symbolLine after content renders
  async function scrollToSymbolLine() {
    const current = getCurrentFile();
    if (!current?.symbolLine || !codeContainer) return;
    await tick();
    const lineEl = codeContainer.querySelector<HTMLElement>(`.line-${current.symbolLine}`);
    if (lineEl) {
      lineEl.scrollIntoView({ block: 'center', behavior: 'smooth' });
    }
  }

  // Handle clicks on code tokens — best-effort symbol navigation
  function handleCodeClick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    // Only pick up clicks on span elements (Shiki token spans)
    if (target.tagName !== 'SPAN') return;
    const text = target.textContent?.trim();
    if (text && text.length > 0 && /^[a-zA-Z_$][a-zA-Z0-9_$]*$/.test(text)) {
      onSymbolClick?.(text);
    }
  }

  // Inject line number wrappers into Shiki output.
  // Shiki v1 outputs: <pre ...><code>...line content...\n</code></pre>
  // We wrap each line in a div with a data-line attribute for styling.
  function injectLineNumbers(html: string, symbolLine?: number, symbolLineEnd?: number): string {
    // Extract the inner content between <code> tags
    const match = html.match(/<code[^>]*>([\s\S]*?)<\/code>/);
    if (!match) return html;

    const innerContent = match[1];
    // Split by newline — each logical line
    const lines = innerContent.split('\n');

    // Remove trailing empty line if any (Shiki adds trailing \n)
    if (lines[lines.length - 1] === '' || lines[lines.length - 1] === '</span>') {
      // keep as is
    }

    const lineHtml = lines
      .map((lineContent, idx) => {
        const lineNum = idx + 1;
        const isHighlighted =
          symbolLine !== undefined &&
          symbolLineEnd !== undefined &&
          lineNum >= symbolLine &&
          lineNum <= symbolLineEnd;
        const highlightClass = isHighlighted ? ' highlighted-line' : '';
        return (
          `<div class="code-line${highlightClass} line-${lineNum}" data-line="${lineNum}">` +
          `<span class="line-number${isHighlighted ? ' line-number-highlighted' : ''}">${lineNum}</span>` +
          `<span class="line-content">${lineContent}</span>` +
          `</div>`
        );
      })
      .join('');

    // Replace the <code> block content
    return html.replace(
      /<code([^>]*)>[\s\S]*?<\/code>/,
      `<code$1>${lineHtml}</code>`,
    );
  }

  // Reactive: load file when activeFile changes
  $effect(() => {
    if (activeFile) {
      loadFile(activeFile).then(() => {
        scrollToSymbolLine();
      });
    } else {
      highlightedHtml = '';
    }
  });

  // Reactive: scroll to new symbol line when symbol changes
  $effect(() => {
    const current = getCurrentFile();
    if (current?.symbolLine && highlightedHtml) {
      scrollToSymbolLine();
    }
  });

  // Compute the final rendered HTML with line numbers injected
  let renderedHtml = $derived.by(() => {
    if (!highlightedHtml) return '';
    const current = getCurrentFile();
    return injectLineNumbers(highlightedHtml, current?.symbolLine, current?.symbolLineEnd);
  });
</script>

<div class="code-panel">
  <!-- Tab bar -->
  <div class="tab-bar" role="tablist">
    {#each openFiles as file (file.path)}
      <button
        class="tab {activeFile === file.path ? 'tab-active' : ''}"
        role="tab"
        aria-selected={activeFile === file.path}
        onclick={() => onSelectTab(file.path)}
      >
        <span class="tab-icon" aria-hidden="true">{getFileIcon(file.path)}</span>
        <span class="tab-name">{filename(file.path)}</span>
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <span
          class="tab-close"
          role="button"
          tabindex="0"
          aria-label="Close {filename(file.path)}"
          onclick={(e) => {
            e.stopPropagation();
            onClose(file.path);
          }}
          onkeydown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.stopPropagation();
              onClose(file.path);
            }
          }}
        >
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
            <path d="M1 1l8 8M9 1L1 9" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
          </svg>
        </span>
      </button>
    {/each}
  </div>

  <!-- Breadcrumb path bar -->
  {#if activeFile}
    <div class="breadcrumb-bar" aria-label="File path">
      {#each activeFile.split('/') as segment, i}
        {#if i > 0}
          <span class="breadcrumb-sep" aria-hidden="true">/</span>
        {/if}
        <span
          class="breadcrumb-segment {i === activeFile.split('/').length - 1 ? 'breadcrumb-filename' : ''}"
        >{segment}</span>
      {/each}
    </div>
  {/if}

  <!-- Code content area -->
  <div class="code-content" bind:this={codeContainer}>
    {#if isLoading}
      <div class="code-loading">
        <span>Loading...</span>
      </div>
    {:else if error}
      <div class="code-error">
        <span>Error: {error}</span>
      </div>
    {:else if activeFile && renderedHtml}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="shiki-wrapper"
        onclick={handleCodeClick}
      >
        {@html renderedHtml}
      </div>
    {:else if !activeFile}
      <div class="code-empty">
        <span>Select a file to view its content</span>
      </div>
    {/if}
  </div>
</div>

<style>
  .code-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--color-bg-secondary);
    overflow: hidden;
  }

  /* Tab bar */
  .tab-bar {
    display: flex;
    overflow-x: auto;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-bg-panel, #161618);
    flex-shrink: 0;
    scrollbar-width: none;
  }

  .tab-bar::-webkit-scrollbar {
    display: none;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    font-size: 12px;
    color: var(--color-text-muted);
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
    transition: color 100ms ease, border-color 100ms ease;
  }

  .tab:hover {
    color: var(--color-text-primary);
    background: rgba(255, 255, 255, 0.04);
  }

  .tab-active {
    color: var(--color-text-primary);
    border-bottom-color: var(--color-accent, #3B82F6);
  }

  .tab-icon {
    font-size: 11px;
    line-height: 1;
    opacity: 0.8;
  }

  .tab-name {
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
  }

  .tab-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border-radius: 3px;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--color-text-muted);
    opacity: 0.6;
    transition: opacity 100ms ease, background 100ms ease;
    padding: 0;
  }

  .tab-close:hover {
    opacity: 1;
    background: rgba(255, 255, 255, 0.1);
    color: var(--color-text-primary);
  }

  /* Breadcrumb path bar */
  .breadcrumb-bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 2px;
    padding: 4px 12px;
    font-size: 11px;
    color: var(--color-text-muted);
    background: var(--color-bg-secondary);
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
    overflow: hidden;
    white-space: nowrap;
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
  }

  .breadcrumb-sep {
    opacity: 0.4;
    padding: 0 1px;
  }

  .breadcrumb-segment {
    opacity: 0.6;
  }

  .breadcrumb-filename {
    color: var(--color-text-primary);
    opacity: 1;
    font-weight: 500;
  }

  /* Code content */
  .code-content {
    flex: 1;
    overflow: auto;
    position: relative;
  }

  .code-loading,
  .code-error,
  .code-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--color-text-muted);
    font-size: 13px;
  }

  .code-error {
    color: #EF4444;
  }

  .shiki-wrapper {
    min-height: 100%;
  }

  /* Override Shiki's <pre> styling */
  .shiki-wrapper :global(pre) {
    margin: 0;
    padding: 0;
    background: transparent !important;
    overflow: visible;
    font-family: 'JetBrains Mono', 'Fira Code', 'Cascadia Code', monospace;
    font-size: 13px;
    line-height: 1.6;
  }

  .shiki-wrapper :global(code) {
    display: block;
    background: transparent !important;
  }

  /* Line wrapper */
  .shiki-wrapper :global(.code-line) {
    display: flex;
    align-items: stretch;
    border-left: 3px solid transparent;
    transition: background 100ms ease;
  }

  .shiki-wrapper :global(.code-line:hover) {
    background: rgba(255, 255, 255, 0.03);
  }

  .shiki-wrapper :global(.highlighted-line) {
    background: rgba(59, 130, 246, 0.1);
    border-left-color: #3B82F6;
  }

  /* Line numbers */
  .shiki-wrapper :global(.line-number) {
    display: inline-block;
    width: 48px;
    min-width: 48px;
    padding: 0 12px 0 8px;
    color: var(--color-text-muted);
    text-align: right;
    user-select: none;
    font-size: 12px;
    opacity: 0.5;
    flex-shrink: 0;
  }

  .shiki-wrapper :global(.line-number-highlighted) {
    color: #3B82F6;
    opacity: 1;
  }

  /* Line content */
  .shiki-wrapper :global(.line-content) {
    flex: 1;
    padding: 0 16px 0 0;
    white-space: pre;
    min-width: 0;
  }

  /* Token click hints */
  .shiki-wrapper :global(span[class*="token"]:not(.line-number):not(.line-content)):hover,
  .shiki-wrapper :global(.line-content span):hover {
    cursor: pointer;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 2px;
  }
</style>
