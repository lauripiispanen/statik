import { Service_87 } from "../services/service_87";

export class Module_87 {
  private service: Service_87;

  constructor() {
    this.service = new Service_87();
  }

  run(): number {
    return this.service.process(87);
  }

  describe(): string {
    return this.service.format("module_87");
  }
}
