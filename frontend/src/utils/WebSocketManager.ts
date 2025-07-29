export interface WebSocketConfig {
  enableReconnect: boolean;
  reconnectInterval: number; // in milliseconds
  maxReconnectAttempts: number;
  mode: 'single' | 'broad'; // WebSocket connection mode
}

export class WebSocketManager {
  private ws: WebSocket | null = null;
  private url: string;
  private config: WebSocketConfig;
  private reconnectAttempts = 0;
  private reconnectTimer: number | null = null;
  private isManualClose = false;

  // Event handlers
  private onOpenHandler?: () => void;
  private onMessageHandler?: (event: MessageEvent) => void;
  private onCloseHandler?: (event: CloseEvent) => void;
  private onErrorHandler?: (error: Event) => void;

  constructor(url: string, config: WebSocketConfig) {
    this.url = url;
    this.config = config;
  }

  setConfig(config: WebSocketConfig) {
    this.config = config
  }


  connect(): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      return;
    }

    this.isManualClose = false;
    this.ws = new WebSocket(this.url);

    this.ws.onopen = () => {
      this.reconnectAttempts = 0;
      this.onOpenHandler?.();
    };

    this.ws.onmessage = (event) => {
      this.onMessageHandler?.(event);
    };

    this.ws.onclose = (event) => {
      this.onCloseHandler?.(event);
      if (!this.isManualClose && this.config.enableReconnect && this.reconnectAttempts < this.config.maxReconnectAttempts) {
        this.scheduleReconnect();
      }
    };

    this.ws.onerror = (error) => {
      this.onErrorHandler?.(error);
    };
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
    }

    this.reconnectTimer = setTimeout(() => {
      this.reconnectAttempts++;
      console.debug(`WebSocket reconnect attempt ${this.reconnectAttempts}/${this.config.maxReconnectAttempts}`);
      this.connect();
    }, this.config.reconnectInterval);
  }

  send(message: string | ArrayBufferLike | Blob | ArrayBufferView): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(message);
    } else {
      console.error("WebSocket is not connected.");
    }
  }

  close(): void {
    this.isManualClose = true;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.ws?.close();
  }

  get readyState(): number | undefined {
    return this.ws?.readyState;
  }

  // Event handler setters
  onOpen(handler: () => void): void {
    this.onOpenHandler = handler;
  }

  onMessage(handler: (event: MessageEvent) => void): void {
    this.onMessageHandler = handler;
  }

  onClose(handler: (event: CloseEvent) => void): void {
    this.onCloseHandler = handler;
  }

  onError(handler: (error: Event) => void): void {
    this.onErrorHandler = handler;
  }
}