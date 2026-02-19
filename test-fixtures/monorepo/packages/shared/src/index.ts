// Shared package barrel - re-exports types and utilities used across packages
export { Result, ok, err, isOk, isErr } from "./result";
export type { User, UserRole } from "./types";
export { validateEmail, validateName } from "./validation";
export { EventEmitter } from "./events";
