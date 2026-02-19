type EventHandler = (...args: unknown[]) => void;

export class EventEmitter {
  private handlers: Map<string, EventHandler[]> = new Map();

  on(event: string, handler: EventHandler): void {
    const existing = this.handlers.get(event) || [];
    existing.push(handler);
    this.handlers.set(event, existing);
  }

  off(event: string, handler: EventHandler): void {
    const existing = this.handlers.get(event) || [];
    this.handlers.set(
      event,
      existing.filter((h) => h !== handler)
    );
  }

  emit(event: string, ...args: unknown[]): void {
    const handlers = this.handlers.get(event) || [];
    for (const handler of handlers) {
      handler(...args);
    }
  }

  once(event: string, handler: EventHandler): void {
    const wrapped: EventHandler = (...args) => {
      handler(...args);
      this.off(event, wrapped);
    };
    this.on(event, wrapped);
  }

  removeAllListeners(event?: string): void {
    if (event) {
      this.handlers.delete(event);
    } else {
      this.handlers.clear();
    }
  }
}
