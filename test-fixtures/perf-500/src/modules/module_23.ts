import { Service_23 } from "../services/service_23";

export class Module_23 {
  private service: Service_23;

  constructor() {
    this.service = new Service_23();
  }

  run(): number {
    return this.service.process(23);
  }

  describe(): string {
    return this.service.format("module_23");
  }
}
