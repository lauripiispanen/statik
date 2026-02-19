import { EventEmitter } from "../../shared/src";

export class NotificationService {
  private emitter: EventEmitter;

  constructor(emitter: EventEmitter) {
    this.emitter = emitter;
  }

  notify(userId: string, message: string): void {
    this.emitter.emit("notification", { userId, message });
  }

  broadcast(message: string): void {
    this.emitter.emit("broadcast", { message });
  }
}
