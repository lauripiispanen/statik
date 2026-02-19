import { Service_16 } from "../services/service_16";

export class Module_16 {
  private service: Service_16;

  constructor() {
    this.service = new Service_16();
  }

  run(): number {
    return this.service.process(16);
  }

  describe(): string {
    return this.service.format("module_16");
  }
}
