import { createHighlighter } from 'shiki';

let highlighter: Awaited<ReturnType<typeof createHighlighter>> | null = null;

export async function getHighlighter() {
  if (!highlighter) {
    highlighter = await createHighlighter({
      themes: ['monokai'],
      langs: ['typescript', 'javascript', 'rust', 'python', 'go', 'json', 'toml', 'yaml', 'css', 'html'],
    });
  }
  return highlighter;
}

export async function highlightCode(code: string, lang: string): Promise<string> {
  const hl = await getHighlighter();
  // Fall back to 'text' if language not loaded
  const supportedLangs = hl.getLoadedLanguages();
  const actualLang = supportedLangs.includes(lang as Parameters<typeof hl.codeToHtml>[1]['lang']) ? lang : 'text';
  return hl.codeToHtml(code, { lang: actualLang as Parameters<typeof hl.codeToHtml>[1]['lang'], theme: 'monokai' });
}

/**
 * Map a file path extension to a Shiki language name.
 */
export function langFromPath(path: string): string {
  const ext = path.split('.').pop()?.toLowerCase() ?? '';
  const map: Record<string, string> = {
    ts: 'typescript',
    tsx: 'typescript',
    js: 'javascript',
    jsx: 'javascript',
    rs: 'rust',
    py: 'python',
    go: 'go',
    json: 'json',
    toml: 'toml',
    yaml: 'yaml',
    yml: 'yaml',
    css: 'css',
    html: 'html',
    svelte: 'html',
  };
  return map[ext] ?? 'text';
}
