import { Service_15 } from "../services/service_15";

export class Module_15 {
  private service: Service_15;

  constructor() {
    this.service = new Service_15();
  }

  run(): number {
    return this.service.process(15);
  }

  describe(): string {
    return this.service.format("module_15");
  }
}
