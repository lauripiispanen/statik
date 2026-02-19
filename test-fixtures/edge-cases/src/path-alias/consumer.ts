import { User, createUser } from "@models/user";
import { formatUser } from "@utils/format";

const user: User = createUser("Alice", "alice@example.com");
console.log(formatUser(user.name, user.email));
