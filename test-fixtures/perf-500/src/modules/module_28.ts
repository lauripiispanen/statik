import { Service_28 } from "../services/service_28";

export class Module_28 {
  private service: Service_28;

  constructor() {
    this.service = new Service_28();
  }

  run(): number {
    return this.service.process(28);
  }

  describe(): string {
    return this.service.format("module_28");
  }
}
