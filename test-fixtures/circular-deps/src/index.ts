// Entry point that uses the circular dependency chain
import { ModuleA } from "./moduleA";

const a = new ModuleA();
console.log(a.getName());
console.log(a.getPartnerName());
