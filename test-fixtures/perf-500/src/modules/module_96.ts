import { Service_96 } from "../services/service_96";

export class Module_96 {
  private service: Service_96;

  constructor() {
    this.service = new Service_96();
  }

  run(): number {
    return this.service.process(96);
  }

  describe(): string {
    return this.service.format("module_96");
  }
}
