import { AuthToken } from './types';

export function login(username: string): AuthToken {
  return { token: `token-${username}`, expiry: Date.now() + 3600000 };
}
