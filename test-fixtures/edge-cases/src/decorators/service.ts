import { Injectable, Log } from "./decorators";

@Injectable()
export class UserService {
  private users: string[] = [];

  @Log()
  addUser(name: string): void {
    this.users.push(name);
  }

  @Log()
  getUsers(): string[] {
    return [...this.users];
  }

  removeUser(name: string): boolean {
    const index = this.users.indexOf(name);
    if (index >= 0) {
      this.users.splice(index, 1);
      return true;
    }
    return false;
  }
}
