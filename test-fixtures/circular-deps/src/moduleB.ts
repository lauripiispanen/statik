// Circular: B imports A (creating A <-> B cycle)
import { getModuleAVersion } from "./moduleA";

export class ModuleB {
  getName(): string {
    return `ModuleB (partner version: ${getModuleAVersion()})`;
  }
}
