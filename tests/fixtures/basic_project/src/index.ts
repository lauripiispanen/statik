import { UserService } from './services/userService';
import { formatName } from './utils/format';
// This directly imports from auth internals (containment violation)
import { login } from './auth/service';

const service = new UserService();
console.log(formatName("test"));
console.log(login("admin"));
