// Circular: A imports B, B imports A
import { ModuleB } from "./moduleB";

export class ModuleA {
  private partner: ModuleB;

  constructor() {
    this.partner = new ModuleB();
  }

  getName(): string {
    return "ModuleA";
  }

  getPartnerName(): string {
    return this.partner.getName();
  }
}

// Used by ModuleB
export function getModuleAVersion(): string {
  return "1.0.0";
}
