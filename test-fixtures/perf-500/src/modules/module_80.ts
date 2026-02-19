import { Service_80 } from "../services/service_80";

export class Module_80 {
  private service: Service_80;

  constructor() {
    this.service = new Service_80();
  }

  run(): number {
    return this.service.process(80);
  }

  describe(): string {
    return this.service.format("module_80");
  }
}
