import { Service_70 } from "../services/service_70";

export class Module_70 {
  private service: Service_70;

  constructor() {
    this.service = new Service_70();
  }

  run(): number {
    return this.service.process(70);
  }

  describe(): string {
    return this.service.format("module_70");
  }
}
