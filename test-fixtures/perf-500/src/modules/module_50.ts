import { Service_50 } from "../services/service_50";

export class Module_50 {
  private service: Service_50;

  constructor() {
    this.service = new Service_50();
  }

  run(): number {
    return this.service.process(50);
  }

  describe(): string {
    return this.service.format("module_50");
  }
}
