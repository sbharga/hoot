export type ConnectionStatus = 'connecting' | 'connected' | 'disconnected';

export function connect(
  role: 'host' | 'player',
  token: string,
  onState: (state: any) => void,
  onError: (message: string, authenticationFailed?: boolean) => void,
  onStatus: (status: ConnectionStatus) => void
) {
  let socket: WebSocket | undefined;
  let stopped = false;
  let attempts = 0;
  let reconnectTimer: number | undefined;
  let authenticated = false;
  let queued: Record<string, unknown> | undefined;

  const open = () => {
    if (stopped) return;
    onStatus('connecting');
    authenticated = false;
    const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
    socket = new WebSocket(`${protocol}//${location.host}/api/ws`);
    socket.onopen = () => {
      attempts = 0;
      socket?.send(JSON.stringify({ type: 'authenticate', role, token }));
    };
    socket.onmessage = (event) => {
      const message = JSON.parse(event.data);
      if (message.type === 'snapshot') {
        onStatus('connected');
        onState(message.state);
        if (!authenticated) {
          authenticated = true;
          if (queued) {
            socket?.send(JSON.stringify(queued));
            queued = undefined;
          }
        }
      } else if (message.type === 'error') {
        onError(message.message, message.code === 'authentication_failed');
      }
    };
    socket.onclose = () => {
      if (stopped) return;
      onStatus('disconnected');
      const delay = Math.min(8_000, 500 * 2 ** attempts++);
      reconnectTimer = window.setTimeout(open, delay);
    };
    socket.onerror = () => socket?.close();
  };
  open();

  return {
    send(command: Record<string, unknown>) {
      if (socket?.readyState === WebSocket.OPEN && authenticated) {
        socket.send(JSON.stringify(command));
      } else {
        queued = command;
        onError('Still reconnecting—your last action will be sent once reconnected.');
      }
    },
    close() {
      stopped = true;
      queued = undefined;
      window.clearTimeout(reconnectTimer);
      socket?.close();
    }
  };
}

