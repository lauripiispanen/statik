import { Service_38 } from "../services/service_38";

export class Module_38 {
  private service: Service_38;

  constructor() {
    this.service = new Service_38();
  }

  run(): number {
    return this.service.process(38);
  }

  describe(): string {
    return this.service.format("module_38");
  }
}
