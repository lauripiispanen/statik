import { Service_1 } from "../services/service_1";

export class Module_1 {
  private service: Service_1;

  constructor() {
    this.service = new Service_1();
  }

  run(): number {
    return this.service.process(1);
  }

  describe(): string {
    return this.service.format("module_1");
  }
}
