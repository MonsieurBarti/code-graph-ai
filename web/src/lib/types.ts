export interface NodeAttributes {
  label: string;
  kind: string;
  language: string;
  path: string;
  size: number;
  x: number;
  y: number;
  color: string;
  isCircular: boolean;
  decorators: string[];
  line: number;
  lineEnd: number;
  /** FA2 layout mass — structural nodes get higher mass to act as layout anchors. */
  mass: number;
}

export interface EdgeAttributes {
  edgeType: string;
  color: string;
  isCircular: boolean;
  /** Sigma edge type identifier — 'curved' for EdgeCurveProgram rendering. */
  type: string;
  /** Bezier curvature amount for @sigma/edge-curve (0.12 to 0.20). */
  curvature: number;
}

export interface GraphNode {
  key: string;
  attributes: NodeAttributes;
}

export interface GraphEdge {
  key: string;
  source: string;
  target: string;
  attributes: EdgeAttributes;
}

export interface GraphResponse {
  attributes: Record<string, unknown>;
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface LanguageStats {
  language: string;
  files: number;
  symbols: number;
}

export interface StatsResponse {
  project_root: string;
  total_files: number;
  total_symbols: number;
  languages: LanguageStats[];
  cache_version: number;
}

export interface SearchResult {
  symbol: string;
  kind: string;
  file: string;
  line: number;
}

export interface CitationData {
  index: number;
  file: string;
  line: number;
  symbol: string;
}

export interface ChatMessageData {
  role: 'user' | 'assistant';
  content: string;
  citations?: CitationData[];
  toolsUsed?: string[];
}

export interface AuthStatus {
  provider: 'claude' | 'ollama';
  configured: boolean;
  model: string;
}

export interface OllamaModel {
  name: string;
  size: number;
  modified_at: string;
}
