<script lang="ts">
  interface Props {
    position: 'left' | 'right';
    onResize: (delta: number) => void;
    onResizeEnd?: () => void;
  }

  let { position, onResize, onResizeEnd }: Props = $props();

  let isDragging = $state(false);

  function handleMouseDown(e: MouseEvent) {
    e.preventDefault();
    isDragging = true;
    let startX = e.clientX;

    const onMove = (me: MouseEvent) => {
      const deltaX = me.clientX - startX;
      startX = me.clientX;
      onResize(deltaX);
    };

    const onUp = () => {
      isDragging = false;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      document.body.style.userSelect = '';
      onResizeEnd?.();
    };

    document.body.style.userSelect = 'none';
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="divider {isDragging ? 'divider-active' : ''}"
  role="separator"
  aria-label="{position === 'left' ? 'Resize sidebar' : 'Resize code panel'}"
  onmousedown={handleMouseDown}
></div>

<style>
  .divider {
    width: 4px;
    flex-shrink: 0;
    background: transparent;
    cursor: col-resize;
    position: relative;
    z-index: 10;
    transition: background 150ms ease;
  }

  .divider:hover,
  .divider-active {
    background: var(--color-accent, #3B82F6);
  }
</style>
