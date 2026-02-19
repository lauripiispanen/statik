import { Result, ok, err } from "./result";

export function validateEmail(email: string): Result<string, string> {
  if (/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
    return ok(email);
  }
  return err("Invalid email format");
}

export function validateName(name: string): Result<string, string> {
  if (name.length >= 2 && name.length <= 50) {
    return ok(name);
  }
  return err("Name must be between 2 and 50 characters");
}

export function validatePassword(password: string): Result<string, string> {
  if (password.length >= 8) {
    return ok(password);
  }
  return err("Password must be at least 8 characters");
}
