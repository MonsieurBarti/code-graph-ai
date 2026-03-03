export interface WsClient {
  close: () => void;
}

export function createWsClient(onGraphUpdate: () => void): WsClient {
  let ws: WebSocket | null = null;
  let reconnectDelay = 1000;
  const maxDelay = 30000;
  let closed = false;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

  function connect() {
    if (closed) return;

    const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
    ws = new WebSocket(`${protocol}//${location.host}/ws`);

    ws.addEventListener('open', () => {
      reconnectDelay = 1000;
    });

    ws.addEventListener('message', (event: MessageEvent) => {
      try {
        const msg = JSON.parse(event.data as string) as { type?: string };
        if (msg.type === 'graph_updated') {
          onGraphUpdate();
        }
      } catch {
        // Ignore non-JSON messages
      }
    });

    ws.addEventListener('close', () => {
      if (closed) return;
      reconnectTimer = setTimeout(() => {
        reconnectDelay = Math.min(reconnectDelay * 2, maxDelay);
        connect();
      }, reconnectDelay);
    });

    ws.addEventListener('error', () => {
      ws?.close();
    });
  }

  connect();

  return {
    close() {
      closed = true;
      if (reconnectTimer !== null) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      ws?.close();
    },
  };
}
