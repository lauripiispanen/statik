import { User, UserRole, createUser } from "../models/user";
import { Logger } from "../utils/logger";

export class UserService {
  private logger: Logger;
  private users: User[] = [];

  constructor(logger: Logger) {
    this.logger = logger;
    this.seedData();
  }

  private seedData(): void {
    this.users = [
      createUser("Alice", "alice@example.com", UserRole.Admin),
      createUser("Bob", "bob@example.com", UserRole.Member),
      createUser("Charlie", "charlie@example.com", UserRole.Guest),
    ];
  }

  async getAllUsers(): Promise<User[]> {
    this.logger.info("Fetching all users");
    return this.users;
  }

  async getUserById(id: string): Promise<User | undefined> {
    this.logger.info(`Fetching user ${id}`);
    return this.users.find((u) => u.id === id);
  }

  async addUser(name: string, email: string, role: UserRole): Promise<User> {
    this.logger.info(`Adding user ${name}`);
    const user = createUser(name, email, role);
    this.users.push(user);
    return user;
  }

  async removeUser(id: string): Promise<boolean> {
    this.logger.warn(`Removing user ${id}`);
    const index = this.users.findIndex((u) => u.id === id);
    if (index >= 0) {
      this.users.splice(index, 1);
      return true;
    }
    return false;
  }

  async getUsersByRole(role: UserRole): Promise<User[]> {
    this.logger.info(`Fetching users with role ${role}`);
    return this.users.filter((u) => u.role === role);
  }
}
