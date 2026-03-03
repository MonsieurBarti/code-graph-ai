import type { GraphResponse, SearchResult, StatsResponse, CitationData, AuthStatus, OllamaModel } from './types';

export async function fetchGraph(granularity: string = 'file'): Promise<GraphResponse> {
  const res = await fetch(`/api/graph?granularity=${encodeURIComponent(granularity)}`);
  if (!res.ok) throw new Error(`fetchGraph failed: ${res.status} ${res.statusText}`);
  return res.json() as Promise<GraphResponse>;
}

export async function fetchFile(path: string): Promise<string> {
  const res = await fetch(`/api/file?path=${encodeURIComponent(path)}`);
  if (!res.ok) throw new Error(`fetchFile failed: ${res.status} ${res.statusText}`);
  return res.text();
}

export async function searchSymbols(query: string, limit: number = 20): Promise<SearchResult[]> {
  const res = await fetch(`/api/search?q=${encodeURIComponent(query)}&limit=${limit}`);
  if (!res.ok) throw new Error(`searchSymbols failed: ${res.status} ${res.statusText}`);
  return res.json() as Promise<SearchResult[]>;
}

export async function fetchStats(): Promise<StatsResponse> {
  const res = await fetch('/api/stats');
  if (!res.ok) throw new Error(`fetchStats failed: ${res.status} ${res.statusText}`);
  return res.json() as Promise<StatsResponse>;
}

export async function sendChatMessage(
  message: string,
  sessionId?: string,
  provider?: string
): Promise<{
  session_id: string;
  answer: string;
  citations: CitationData[];
  tools_used: string[];
  provider: string;
}> {
  const res = await fetch('/api/chat', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ session_id: sessionId, message, provider }),
  });
  if (!res.ok) throw new Error(await res.text());
  return res.json() as Promise<{ session_id: string; answer: string; citations: CitationData[]; tools_used: string[]; provider: string }>;
}

export async function getAuthStatus(): Promise<AuthStatus> {
  const res = await fetch('/api/auth/status');
  if (!res.ok) throw new Error(`getAuthStatus failed: ${res.status} ${res.statusText}`);
  return res.json() as Promise<AuthStatus>;
}

export async function setApiKey(apiKey: string): Promise<void> {
  const res = await fetch('/api/auth/key', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ api_key: apiKey }),
  });
  if (!res.ok) throw new Error(`setApiKey failed: ${res.status} ${res.statusText}`);
}

export async function setProvider(provider: string, model?: string): Promise<void> {
  const res = await fetch('/api/auth/provider', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ provider, model }),
  });
  if (!res.ok) throw new Error(`setProvider failed: ${res.status} ${res.statusText}`);
}

/** Fetch locally available Ollama models (proxied through our backend to avoid CORS). */
export async function fetchOllamaModels(): Promise<OllamaModel[]> {
  const res = await fetch('/api/ollama/models');
  if (!res.ok) return [];
  const data = await res.json() as { models?: OllamaModel[] };
  return data.models ?? [];
}
