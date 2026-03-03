<script lang="ts">
  interface Props {
    onSend: (text: string) => void;
    isThinking?: boolean;
    disabled?: boolean;
  }

  let { onSend, isThinking = false, disabled = false }: Props = $props();

  let inputText = $state('');
  let textareaEl: HTMLTextAreaElement | null = $state(null);

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  }

  function submit() {
    const text = inputText.trim();
    if (!text || isThinking || disabled) return;
    onSend(text);
    inputText = '';
    // Reset textarea height after send
    if (textareaEl) {
      textareaEl.style.height = 'auto';
    }
  }

  function handleInput() {
    if (!textareaEl) return;
    // Auto-resize: reset to auto first, then set scrollHeight
    textareaEl.style.height = 'auto';
    textareaEl.style.height = Math.min(textareaEl.scrollHeight, 160) + 'px';
  }
</script>

<div class="chat-input-bar">
  <textarea
    bind:this={textareaEl}
    bind:value={inputText}
    onkeydown={handleKeydown}
    oninput={handleInput}
    disabled={isThinking || disabled}
    placeholder={disabled ? 'Configure a provider to start chatting...' : 'Ask about your codebase...'}
    rows="1"
    class="chat-textarea"
  ></textarea>
  <button
    class="send-btn"
    onclick={submit}
    disabled={isThinking || disabled || !inputText.trim()}
    title="Send message"
    aria-label="Send message"
  >
    <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <line x1="22" y1="2" x2="11" y2="13"/>
      <polygon points="22 2 15 22 11 13 2 9 22 2"/>
    </svg>
  </button>
</div>

<style>
  .chat-input-bar {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    padding: 10px 12px;
    border-top: 1px solid var(--color-border);
    background: var(--color-bg-secondary);
    flex-shrink: 0;
  }

  .chat-textarea {
    flex: 1;
    resize: none;
    background: var(--color-bg-elevated);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    color: var(--color-text-primary);
    font-size: 13px;
    font-family: inherit;
    line-height: 1.5;
    padding: 8px 10px;
    outline: none;
    min-height: 36px;
    max-height: 160px;
    overflow-y: auto;
    transition: border-color 150ms ease;
  }

  .chat-textarea::placeholder {
    color: var(--color-text-muted);
  }

  .chat-textarea:focus {
    border-color: var(--color-accent, #7c5cfc);
  }

  .chat-textarea:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .send-btn {
    width: 36px;
    height: 36px;
    flex-shrink: 0;
    border-radius: 8px;
    border: none;
    background: var(--color-accent, #7c5cfc);
    color: white;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: opacity 150ms ease, background 150ms ease;
  }

  .send-btn:hover:not(:disabled) {
    opacity: 0.85;
  }

  .send-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }
</style>
