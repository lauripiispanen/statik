// This import from db violates boundary rules
import { getConnection } from '../db/connection';
import { User } from '../models/user';

export function UserForm(user: User) {
  const conn = getConnection();
  return `<form>${user.name}</form>`;
}
