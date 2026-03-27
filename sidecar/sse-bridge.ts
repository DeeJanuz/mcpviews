#!/usr/bin/env node
/**
 * SSE Bridge Sidecar for MCPViews
 *
 * Connects to a remote app's /api/companion/stream via SSE,
 * and forwards push events to the local MCPViews HTTP server
 * (POST localhost:4200/api/push).
 *
 * Usage: node sse-bridge.mjs --app-host https://app.example.com --key lf_companion_xxx
 */

const PUSH_URL = 'http://localhost:4200/api/push';

interface Config {
  appHost: string;
  companionKey: string;
}

function parseArgs(): Config | null {
  const args = process.argv.slice(2);
  let appHost = '';
  let companionKey = '';

  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--app-host' && args[i + 1]) {
      appHost = args[i + 1];
      i++;
    } else if (args[i] === '--key' && args[i + 1]) {
      companionKey = args[i + 1];
      i++;
    }
  }

  if (!appHost || !companionKey) {
    console.error('Usage: sse-bridge --app-host <url> --key <companion_key>');
    return null;
  }

  return { appHost, companionKey };
}

class SSEBridge {
  private abortController: AbortController | null = null;
  private reconnectDelay = 5000;
  private readonly maxReconnectDelay = 60000;
  private keepaliveTimeout: ReturnType<typeof setTimeout> | null = null;
  private readonly keepaliveDeadline = 45000;
  private running = false;

  constructor(private readonly config: Config) {}

  async start(): Promise<void> {
    this.running = true;
    console.log(`[sse-bridge] Starting bridge to ${this.config.appHost}`);
    this.connect();
  }

  stop(): void {
    this.running = false;
    this.abortController?.abort();
    this.abortController = null;
    this.clearKeepaliveTimeout();
  }

  private async connect(): Promise<void> {
    if (!this.running) return;

    this.abortController = new AbortController();
    const url = `${this.config.appHost}/api/companion/stream`;

    try {
      console.log(`[sse-bridge] Connecting to ${url}`);

      const response = await fetch(url, {
        headers: {
          'Authorization': `Bearer ${this.config.companionKey}`,
          'Accept': 'text/event-stream',
        },
        signal: this.abortController.signal,
      });

      if (!response.ok) {
        throw new Error(`SSE connection failed: ${response.status} ${response.statusText}`);
      }

      if (!response.body) {
        throw new Error('SSE response has no body');
      }

      console.log('[sse-bridge] Connected');
      this.reconnectDelay = 5000;
      this.resetKeepaliveTimeout();

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';

      while (this.running) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        this.resetKeepaliveTimeout();

        const lines = buffer.split('\n');
        buffer = lines.pop() ?? '';

        for (const line of lines) {
          if (line.startsWith('data: ')) {
            const jsonStr = line.slice(6);
            try {
              const event = JSON.parse(jsonStr);
              await this.forwardToLocal(event);
            } catch (err) {
              console.error('[sse-bridge] Failed to parse SSE event:', err);
            }
          }
        }
      }
    } catch (error: unknown) {
      const isAborted = error instanceof Error && error.name === 'AbortError';
      if (isAborted && !this.running) {
        console.log('[sse-bridge] Stopped');
        return;
      }
      console.error('[sse-bridge] Connection error:', error);
    }

    this.clearKeepaliveTimeout();

    if (this.running) {
      console.log(`[sse-bridge] Reconnecting in ${this.reconnectDelay / 1000}s`);
      await new Promise(resolve => setTimeout(resolve, this.reconnectDelay));
      this.reconnectDelay = Math.min(this.reconnectDelay * 2, this.maxReconnectDelay);
      this.connect();
    }
  }

  private async forwardToLocal(pushReq: unknown): Promise<void> {
    try {
      const res = await fetch(PUSH_URL, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(pushReq),
      });

      if (!res.ok) {
        console.error(`[sse-bridge] Push failed: ${res.status}`);
      }
    } catch (err) {
      console.error('[sse-bridge] Push to local failed:', err);
    }
  }

  private resetKeepaliveTimeout(): void {
    this.clearKeepaliveTimeout();
    this.keepaliveTimeout = setTimeout(() => {
      console.warn('[sse-bridge] Keepalive timeout — forcing reconnect');
      this.abortController?.abort();
    }, this.keepaliveDeadline);
  }

  private clearKeepaliveTimeout(): void {
    if (this.keepaliveTimeout) {
      clearTimeout(this.keepaliveTimeout);
      this.keepaliveTimeout = null;
    }
  }
}

// --- Main ---
const config = parseArgs();
if (!config) {
  process.exit(1);
}

const bridge = new SSEBridge(config);

process.on('SIGINT', () => {
  bridge.stop();
  process.exit(0);
});

process.on('SIGTERM', () => {
  bridge.stop();
  process.exit(0);
});

bridge.start().catch((err) => {
  console.error('[sse-bridge] Fatal error:', err);
  process.exit(1);
});
