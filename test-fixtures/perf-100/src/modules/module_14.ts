import { Service_14 } from "../services/service_14";

export class Module_14 {
  private service: Service_14;

  constructor() {
    this.service = new Service_14();
  }

  run(): number {
    return this.service.process(14);
  }

  describe(): string {
    return this.service.format("module_14");
  }
}
