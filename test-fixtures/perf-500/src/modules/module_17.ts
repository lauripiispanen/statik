import { Service_17 } from "../services/service_17";

export class Module_17 {
  private service: Service_17;

  constructor() {
    this.service = new Service_17();
  }

  run(): number {
    return this.service.process(17);
  }

  describe(): string {
    return this.service.format("module_17");
  }
}
