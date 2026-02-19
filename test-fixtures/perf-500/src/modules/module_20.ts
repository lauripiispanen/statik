import { Service_20 } from "../services/service_20";

export class Module_20 {
  private service: Service_20;

  constructor() {
    this.service = new Service_20();
  }

  run(): number {
    return this.service.process(20);
  }

  describe(): string {
    return this.service.format("module_20");
  }
}
