import { Service_30 } from "../services/service_30";

export class Module_30 {
  private service: Service_30;

  constructor() {
    this.service = new Service_30();
  }

  run(): number {
    return this.service.process(30);
  }

  describe(): string {
    return this.service.format("module_30");
  }
}
