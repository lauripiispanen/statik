import { UserManager } from "../../core/src";
import { EventEmitter } from "../../shared/src";
import type { User } from "../../shared/src";
import { isOk } from "../../shared/src";

export class UserListComponent {
  private userManager: UserManager;

  constructor(events: EventEmitter) {
    this.userManager = new UserManager(events);
  }

  render(): string {
    const users = this.userManager.listUsers();
    return users.map((u: User) => `<li>${u.name} (${u.email})</li>`).join("\n");
  }

  addUser(name: string, email: string): boolean {
    const result = this.userManager.createUser(name, email, "user");
    return isOk(result);
  }
}

export class UserDetailComponent {
  private userManager: UserManager;

  constructor(events: EventEmitter) {
    this.userManager = new UserManager(events);
  }

  render(userId: string): string {
    const result = this.userManager.getUser(userId);
    if (isOk(result)) {
      const user = result.value;
      return `<div><h2>${user.name}</h2><p>${user.email}</p></div>`;
    }
    return `<div>User not found</div>`;
  }
}
