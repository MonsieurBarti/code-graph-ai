<script lang="ts">
  import { onMount, tick } from 'svelte';
  import ChatMessage from './ChatMessage.svelte';
  import ChatInput from './ChatInput.svelte';
  import ProviderSelector from './ProviderSelector.svelte';
  import { sendChatMessage, getAuthStatus } from '../api';
  import type { ChatMessageData, AuthStatus } from '../types';

  interface Props {
    onCitationClick?: (file: string, line: number) => void;
    onClose?: () => void;
    onCollapse?: () => void;
  }

  let { onCitationClick, onClose, onCollapse }: Props = $props();

  let messages: ChatMessageData[] = $state([]);
  let sessionId: string | null = $state(null);
  let isThinking: boolean = $state(false);
  let authStatus: AuthStatus | null = $state(null);
  let messagesEndEl: HTMLElement | null = $state(null);
  let messagesContainerEl: HTMLElement | null = $state(null);

  onMount(async () => {
    try {
      authStatus = await getAuthStatus();
    } catch {
      // Backend may not be running — show default state
      authStatus = null;
    }
  });

  async function scrollToBottom() {
    await tick();
    messagesEndEl?.scrollIntoView({ behavior: 'smooth' });
  }

  async function sendMessage(text: string) {
    if (!text.trim() || isThinking) return;
    if (!authStatus?.configured) return;

    // Add user message immediately
    messages = [...messages, { role: 'user', content: text }];
    isThinking = true;
    await scrollToBottom();

    try {
      const response = await sendChatMessage(text, sessionId ?? undefined, authStatus?.provider);
      sessionId = response.session_id;

      // Add assistant response with citations and tools
      messages = [
        ...messages,
        {
          role: 'assistant',
          content: response.answer,
          citations: response.citations,
          toolsUsed: response.tools_used,
        },
      ];
    } catch (e) {
      // Show error as a system-style message
      const errorText = e instanceof Error ? e.message : 'An error occurred while contacting the AI.';
      messages = [
        ...messages,
        {
          role: 'assistant',
          content: `**Error:** ${errorText}\n\nPlease check that the backend server is running and an LLM provider is configured.`,
          citations: [],
          toolsUsed: [],
        },
      ];
    } finally {
      isThinking = false;
      await scrollToBottom();
    }
  }

  function handleProviderChange(provider: string, model: string) {
    if (authStatus) {
      authStatus = { ...authStatus, provider: provider as 'claude' | 'ollama', model };
    }
  }

  let headerLabel = $derived.by(() => {
    if (!authStatus) return 'Chat';
    return 'Chat';
  });
</script>

<div class="chat-panel">
  <!-- Header -->
  <div class="chat-header">
    <div class="chat-header-left">
      <span class="chat-title">{headerLabel}</span>
      <ProviderSelector bind:authStatus onProviderChange={handleProviderChange} />
    </div>
    <div class="chat-header-actions">
      <button
        class="header-btn"
        onclick={onCollapse}
        title="Collapse chat panel"
        aria-label="Collapse chat panel"
      >
        <!-- Chevron-right icon to collapse -->
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <polyline points="9 18 15 12 9 6"/>
        </svg>
      </button>
      <button
        class="header-btn"
        onclick={onClose}
        title="Close chat panel"
        aria-label="Close chat panel"
      >
        <!-- X icon to close -->
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <line x1="18" y1="6" x2="6" y2="18"/>
          <line x1="6" y1="6" x2="18" y2="18"/>
        </svg>
      </button>
    </div>
  </div>

  <!-- Message list -->
  <div class="messages-container" bind:this={messagesContainerEl}>
    {#if messages.length === 0}
      <div class="empty-state">
        <div class="empty-icon">
          <svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
            <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
          </svg>
        </div>
        <div class="empty-title">Ask about your codebase</div>
        <div class="empty-hint">Try: "Where is authentication handled?" or "What calls the main function?"</div>
      </div>
    {/if}

    {#each messages as message}
      <ChatMessage {message} onCitationClick={onCitationClick} />
    {/each}

    {#if isThinking}
      <div class="thinking-indicator">
        <div class="thinking-dots">
          <span></span>
          <span></span>
          <span></span>
        </div>
        <span class="thinking-text">Thinking...</span>
      </div>
    {/if}

    <div bind:this={messagesEndEl}></div>
  </div>

  <!-- Input at bottom -->
  <ChatInput {isThinking} disabled={!authStatus?.configured} onSend={sendMessage} />
</div>

<style>
  .chat-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
    background: var(--color-bg-surface);
  }

  /* Header */
  .chat-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 10px;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
    min-height: 42px;
  }

  .chat-header-left {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    flex: 1;
  }

  .chat-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    flex-shrink: 0;
  }

  .chat-header-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }

  .header-btn {
    width: 26px;
    height: 26px;
    border-radius: 4px;
    border: none;
    background: transparent;
    cursor: pointer;
    color: var(--color-text-muted);
    display: flex;
    align-items: center;
    justify-content: center;
    transition: color 100ms ease, background 100ms ease;
  }

  .header-btn:hover {
    color: var(--color-text-primary);
    background: rgba(255, 255, 255, 0.06);
  }

  /* Messages */
  .messages-container {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  /* Empty state */
  .empty-state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 32px 24px;
    gap: 10px;
    text-align: center;
  }

  .empty-icon {
    color: var(--color-text-muted);
    opacity: 0.4;
  }

  .empty-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--color-text-primary);
    opacity: 0.7;
  }

  .empty-hint {
    font-size: 12px;
    color: var(--color-text-muted);
    line-height: 1.5;
    max-width: 240px;
  }

  /* Thinking indicator */
  .thinking-indicator {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 16px;
    color: var(--color-text-muted);
    font-size: 12px;
    font-style: italic;
  }

  .thinking-dots {
    display: flex;
    gap: 4px;
    align-items: center;
  }

  .thinking-dots span {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--color-text-muted);
    animation: pulse 1.2s ease-in-out infinite;
  }

  .thinking-dots span:nth-child(2) {
    animation-delay: 0.2s;
  }

  .thinking-dots span:nth-child(3) {
    animation-delay: 0.4s;
  }

  @keyframes pulse {
    0%, 80%, 100% {
      opacity: 0.3;
      transform: scale(0.8);
    }
    40% {
      opacity: 1;
      transform: scale(1);
    }
  }
</style>
