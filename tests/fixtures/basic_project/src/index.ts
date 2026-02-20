import { UserService } from './services/userService';
import { formatName } from './utils/format';
// This directly imports from auth internals (containment violation)
import { login } from './auth/service';
// Import through barrel file
import { barrelHelper, specialThing } from './barrel';

const service = new UserService();
console.log(formatName("test"));
console.log(login("admin"));
console.log(barrelHelper());
console.log(specialThing());

// Dynamic import
async function loadLazy() {
  const mod = await import("./lazy");
  return mod.lazyLoadedFn();
}
loadLazy();
