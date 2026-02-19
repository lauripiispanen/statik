import { Result, ok, err, isOk } from "../../shared/src";
import type { User, UserRole } from "../../shared/src";
import { validateEmail, validateName } from "../../shared/src";
import { EventEmitter } from "../../shared/src";

export class UserManager {
  private users: Map<string, User> = new Map();
  private events: EventEmitter;

  constructor(events: EventEmitter) {
    this.events = events;
  }

  createUser(name: string, email: string, role: UserRole): Result<User, string> {
    const nameResult = validateName(name);
    if (!isOk(nameResult)) return nameResult;

    const emailResult = validateEmail(email);
    if (!isOk(emailResult)) return emailResult;

    const user: User = {
      id: Math.random().toString(36).substring(7),
      name,
      email,
      role,
    };

    this.users.set(user.id, user);
    this.events.emit("user:created", user);
    return ok(user);
  }

  getUser(id: string): Result<User, string> {
    const user = this.users.get(id);
    if (!user) return err(`User ${id} not found`);
    return ok(user);
  }

  deleteUser(id: string): Result<void, string> {
    if (!this.users.has(id)) return err(`User ${id} not found`);
    this.users.delete(id);
    this.events.emit("user:deleted", id);
    return ok(undefined);
  }

  listUsers(): User[] {
    return Array.from(this.users.values());
  }
}
