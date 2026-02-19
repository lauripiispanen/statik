import { UserManager } from "../../core/src";
import { EventEmitter } from "../../shared/src";
import { isOk } from "../../shared/src";

const events = new EventEmitter();
const userManager = new UserManager(events);

events.on("user:created", (user) => {
  console.log("[API] User created:", user);
});

export function handleCreateUser(name: string, email: string): object {
  const result = userManager.createUser(name, email, "user");
  if (isOk(result)) {
    return { status: 200, data: result.value };
  }
  return { status: 400, error: result.error };
}

export function handleListUsers(): object {
  return { status: 200, data: userManager.listUsers() };
}

export function handleDeleteUser(id: string): object {
  return { status: 501, error: "Not implemented" };
}
