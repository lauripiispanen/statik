import { User } from '../models/user';
import { formatName } from '../utils/format';

export class UserService {
  getUser(): User {
    return { name: formatName("test"), id: 1 };
  }
}

export function unusedHelper() {
  return "unused";
}
