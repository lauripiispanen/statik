export interface User {
  id: string;
  name: string;
  email: string;
  createdAt: Date;
  role: UserRole;
}

export enum UserRole {
  Admin = "admin",
  Member = "member",
  Guest = "guest",
}

export interface UserPreferences {
  theme: "light" | "dark";
  language: string;
  notifications: boolean;
}

export type UserWithoutId = Omit<User, "id">;

export function createUser(name: string, email: string, role: UserRole): User {
  return {
    id: Math.random().toString(36).substring(7),
    name,
    email,
    createdAt: new Date(),
    role,
  };
}

export function deleteUser(id: string): boolean {
  console.log(`Deleting user ${id}`);
  return true;
}

export class UserValidator {
  validate(user: User): boolean {
    return !!user.name && !!user.email;
  }

  validateEmail(email: string): boolean {
    return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email);
  }
}
