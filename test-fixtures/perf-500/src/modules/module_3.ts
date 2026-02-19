import { Service_3 } from "../services/service_3";

export class Module_3 {
  private service: Service_3;

  constructor() {
    this.service = new Service_3();
  }

  run(): number {
    return this.service.process(3);
  }

  describe(): string {
    return this.service.format("module_3");
  }
}
