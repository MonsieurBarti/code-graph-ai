export interface NavigationState {
  selectedNode: string | null;
  granularity: string;
  cameraX: number;
  cameraY: number;
  cameraRatio: number;
}

export class NavigationHistory {
  private stack: NavigationState[] = [];
  private index: number = -1;

  push(state: NavigationState): void {
    // Truncate forward history when pushing a new state
    this.stack = this.stack.slice(0, this.index + 1);
    this.stack.push(state);
    this.index = this.stack.length - 1;
  }

  back(): NavigationState | null {
    if (!this.canGoBack()) return null;
    this.index--;
    return this.stack[this.index];
  }

  forward(): NavigationState | null {
    if (!this.canGoForward()) return null;
    this.index++;
    return this.stack[this.index];
  }

  canGoBack(): boolean {
    return this.index > 0;
  }

  canGoForward(): boolean {
    return this.index < this.stack.length - 1;
  }

  current(): NavigationState | null {
    if (this.index < 0 || this.index >= this.stack.length) return null;
    return this.stack[this.index];
  }
}

export function syncToUrl(state: NavigationState): void {
  const params = new URLSearchParams();
  if (state.selectedNode) params.set('node', state.selectedNode);
  params.set('g', state.granularity);
  params.set('x', state.cameraX.toFixed(2));
  params.set('y', state.cameraY.toFixed(2));
  params.set('r', state.cameraRatio.toFixed(2));
  history.pushState(null, '', `?${params.toString()}`);
}

export function syncFromUrl(): Partial<NavigationState> {
  const params = new URLSearchParams(location.search);
  const result: Partial<NavigationState> = {
    granularity: params.get('g') || 'file',
    cameraX: parseFloat(params.get('x') || '0'),
    cameraY: parseFloat(params.get('y') || '0'),
    cameraRatio: parseFloat(params.get('r') || '1'),
  };
  const node = params.get('node');
  if (node) {
    result.selectedNode = node;
  } else {
    result.selectedNode = null;
  }
  return result;
}
