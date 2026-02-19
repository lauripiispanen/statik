import { Service_7 } from "../services/service_7";

export class Module_7 {
  private service: Service_7;

  constructor() {
    this.service = new Service_7();
  }

  run(): number {
    return this.service.process(7);
  }

  describe(): string {
    return this.service.format("module_7");
  }
}
