import { UserService } from "../services/userService";
import { UserRole } from "../models/user";
import { Logger } from "../utils/logger";

export class UserController {
  private userService: UserService;

  constructor() {
    const logger = new Logger("UserController");
    this.userService = new UserService(logger);
  }

  async handleGetUsers(): Promise<string> {
    const users = await this.userService.getAllUsers();
    return JSON.stringify(users);
  }

  async handleGetUser(id: string): Promise<string> {
    const user = await this.userService.getUserById(id);
    if (!user) {
      return JSON.stringify({ error: "Not found" });
    }
    return JSON.stringify(user);
  }

  async handleCreateUser(
    name: string,
    email: string,
    role: string
  ): Promise<string> {
    const userRole = role as UserRole;
    const user = await this.userService.addUser(name, email, userRole);
    return JSON.stringify(user);
  }
}
