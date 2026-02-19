import { Service_6 } from "../services/service_6";

export class Module_6 {
  private service: Service_6;

  constructor() {
    this.service = new Service_6();
  }

  run(): number {
    return this.service.process(6);
  }

  describe(): string {
    return this.service.format("module_6");
  }
}
