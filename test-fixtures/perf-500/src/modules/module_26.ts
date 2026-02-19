import { Service_26 } from "../services/service_26";

export class Module_26 {
  private service: Service_26;

  constructor() {
    this.service = new Service_26();
  }

  run(): number {
    return this.service.process(26);
  }

  describe(): string {
    return this.service.format("module_26");
  }
}
