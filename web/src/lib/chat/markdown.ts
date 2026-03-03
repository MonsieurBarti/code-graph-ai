import { marked } from 'marked';
import DOMPurify from 'dompurify';
import { highlightCode } from '../code/syntax';

// Configure marked to use shiki placeholders for code blocks
const renderer = new marked.Renderer();
renderer.code = function ({ text, lang }: { text: string; lang?: string | null }) {
  const escapedText = text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
  return `<pre class="shiki-code" data-lang="${lang || 'text'}"><code>${escapedText}</code></pre>`;
};

marked.use({ renderer });

/**
 * Render markdown to sanitized HTML.
 * Code blocks are left as plain-text placeholders for async shiki post-processing.
 */
export function renderMarkdown(text: string): string {
  const rawHtml = marked.parse(text) as string;
  // Sanitize HTML to prevent XSS — allow class/data attributes needed for shiki
  return DOMPurify.sanitize(rawHtml, {
    ADD_ATTR: ['data-lang', 'class'],
    ADD_TAGS: ['pre', 'code'],
  });
}

/**
 * Post-process: replace shiki-code placeholder blocks with syntax-highlighted HTML.
 * Must be called after the rendered HTML is mounted in the DOM.
 */
export async function highlightCodeBlocks(container: HTMLElement): Promise<void> {
  const blocks = container.querySelectorAll('pre.shiki-code code');
  for (const block of blocks) {
    const lang = block.parentElement?.getAttribute('data-lang') || 'text';
    const code = block.textContent || '';
    try {
      const highlighted = await highlightCode(code, lang);
      const sanitized = DOMPurify.sanitize(highlighted, {
        ADD_ATTR: ['style', 'class'],
      });
      block.parentElement!.outerHTML = sanitized;
    } catch {
      // Keep plain text on error — silently ignore highlighting failures
    }
  }
}
