import { UserService } from "./services/userService";
import { formatDate } from "./utils/format";
import { Logger } from "./utils/logger";

const logger = new Logger("main");
const userService = new UserService(logger);

async function main() {
  logger.info("Starting application");
  const users = await userService.getAllUsers();
  for (const user of users) {
    console.log(`${user.name} - joined ${formatDate(user.createdAt)}`);
  }
}

main();
