export type UserRole = "admin" | "user" | "guest";

export interface User {
  id: string;
  name: string;
  email: string;
  role: UserRole;
}

export interface Session {
  userId: string;
  token: string;
  expiresAt: Date;
}

export interface Pagination {
  page: number;
  limit: number;
  total: number;
}
