import { Service_10 } from "../services/service_10";

export class Module_10 {
  private service: Service_10;

  constructor() {
    this.service = new Service_10();
  }

  run(): number {
    return this.service.process(10);
  }

  describe(): string {
    return this.service.format("module_10");
  }
}
