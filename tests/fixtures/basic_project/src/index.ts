import { UserService } from './services/userService';
import { formatName } from './utils/format';

const service = new UserService();
console.log(formatName("test"));
