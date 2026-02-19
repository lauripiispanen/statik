import { Service_5 } from "../services/service_5";

export class Module_5 {
  private service: Service_5;

  constructor() {
    this.service = new Service_5();
  }

  run(): number {
    return this.service.process(5);
  }

  describe(): string {
    return this.service.format("module_5");
  }
}
