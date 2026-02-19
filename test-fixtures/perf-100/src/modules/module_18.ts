import { Service_18 } from "../services/service_18";

export class Module_18 {
  private service: Service_18;

  constructor() {
    this.service = new Service_18();
  }

  run(): number {
    return this.service.process(18);
  }

  describe(): string {
    return this.service.format("module_18");
  }
}
