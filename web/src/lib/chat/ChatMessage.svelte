<script lang="ts">
  import { onMount } from 'svelte';
  import type { ChatMessageData, CitationData } from '../types';
  import { renderMarkdown, highlightCodeBlocks } from './markdown';

  interface Props {
    message: ChatMessageData;
    onCitationClick?: (file: string, line: number) => void;
  }

  let { message, onCitationClick }: Props = $props();

  let containerEl: HTMLElement | null = $state(null);

  let renderedHtml = $derived.by(() => {
    if (message.role === 'assistant') {
      return renderMarkdown(message.content);
    }
    return '';
  });

  onMount(() => {
    if (message.role === 'assistant' && containerEl) {
      // Post-process code blocks with shiki after mount
      highlightCodeBlocks(containerEl).catch(() => {
        // Silently ignore highlighting errors
      });
    }
  });

  function handleContainerClick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    const citationLink = target.closest('.citation-link') as HTMLAnchorElement | null;
    if (citationLink) {
      e.preventDefault();
      const file = citationLink.getAttribute('data-file');
      const line = parseInt(citationLink.getAttribute('data-line') || '0', 10);
      if (file) {
        onCitationClick?.(file, line);
      }
    }
  }

  // Build citation links from citations array
  let citationLinksHtml = $derived.by(() => {
    if (!message.citations || message.citations.length === 0) return '';
    const items = message.citations.map((c: CitationData) => {
      return `<a class="citation-source-link citation-link" data-file="${c.file}" data-line="${c.line}" href="#">[${c.index}] ${c.file}:${c.line} — ${c.symbol}</a>`;
    });
    return `<div class="citations-list">${items.join('')}</div>`;
  });
</script>

<div class="chat-message {message.role}">
  <div class="message-meta">
    <span class="message-author">{message.role === 'user' ? 'You' : 'Assistant'}</span>
  </div>

  {#if message.role === 'user'}
    <div class="message-bubble user-bubble">
      <span class="user-text">{message.content}</span>
    </div>
  {:else}
    <!-- eslint-disable-next-line svelte/no-at-html-tags -->
    <div
      class="message-bubble assistant-bubble markdown-content"
      bind:this={containerEl}
      onclick={handleContainerClick}
      role="presentation"
    >
      {@html renderedHtml}

      {#if message.citations && message.citations.length > 0}
        <!-- Source citations list at bottom -->
        {@html citationLinksHtml}
      {/if}
    </div>

    {#if message.toolsUsed && message.toolsUsed.length > 0}
      <div class="tools-footer">
        Used: {message.toolsUsed.join(', ')}
      </div>
    {/if}
  {/if}
</div>

<style>
  .chat-message {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px 12px;
  }

  .chat-message.user {
    align-items: flex-end;
  }

  .chat-message.assistant {
    align-items: flex-start;
  }

  .message-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 2px;
  }

  .message-author {
    font-size: 11px;
    font-weight: 600;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .message-bubble {
    max-width: 90%;
    border-radius: 10px;
    padding: 10px 14px;
    font-size: 13px;
    line-height: 1.6;
    word-break: break-word;
  }

  .user-bubble {
    background: var(--color-accent, #7c5cfc);
    color: white;
    border-bottom-right-radius: 3px;
  }

  .user-text {
    white-space: pre-wrap;
  }

  .assistant-bubble {
    background: var(--color-bg-elevated);
    color: var(--color-text-primary);
    border-bottom-left-radius: 3px;
    border: 1px solid var(--color-border);
  }

  /* Markdown content styles */
  .markdown-content :global(p) {
    margin: 0 0 8px 0;
  }

  .markdown-content :global(p:last-child) {
    margin-bottom: 0;
  }

  .markdown-content :global(code) {
    background: rgba(255, 255, 255, 0.08);
    padding: 1px 5px;
    border-radius: 3px;
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
    font-size: 12px;
  }

  .markdown-content :global(pre) {
    background: var(--color-bg-surface);
    border: 1px solid var(--color-border);
    border-radius: 6px;
    padding: 12px;
    overflow-x: auto;
    margin: 8px 0;
  }

  .markdown-content :global(pre code) {
    background: none;
    padding: 0;
    font-size: 12px;
  }

  .markdown-content :global(ul), .markdown-content :global(ol) {
    padding-left: 20px;
    margin: 4px 0;
  }

  .markdown-content :global(li) {
    margin: 2px 0;
  }

  .markdown-content :global(strong) {
    font-weight: 600;
    color: var(--color-text-primary);
  }

  .markdown-content :global(em) {
    font-style: italic;
  }

  .markdown-content :global(blockquote) {
    border-left: 3px solid var(--color-accent, #7c5cfc);
    margin: 8px 0;
    padding-left: 12px;
    color: var(--color-text-muted);
  }

  .markdown-content :global(a.citation-link) {
    color: var(--color-accent, #7c5cfc);
    text-decoration: none;
    cursor: pointer;
    font-size: 11px;
    vertical-align: super;
  }

  .markdown-content :global(a.citation-link:hover) {
    text-decoration: underline;
  }

  /* Citations list at bottom of assistant message */
  .markdown-content :global(.citations-list) {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 10px;
    padding-top: 10px;
    border-top: 1px solid var(--color-border);
  }

  .markdown-content :global(.citation-source-link) {
    font-size: 11px;
    color: var(--color-text-muted);
    text-decoration: none;
    cursor: pointer;
    font-family: 'JetBrains Mono', monospace;
    display: block;
    padding: 2px 0;
    transition: color 100ms ease;
  }

  .markdown-content :global(.citation-source-link:hover) {
    color: var(--color-accent, #7c5cfc);
  }

  /* Tools footer */
  .tools-footer {
    font-size: 11px;
    color: var(--color-text-muted);
    padding: 2px 0;
    font-style: italic;
    max-width: 90%;
  }
</style>
