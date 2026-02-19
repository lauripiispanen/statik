import { Service_25 } from "../services/service_25";

export class Module_25 {
  private service: Service_25;

  constructor() {
    this.service = new Service_25();
  }

  run(): number {
    return this.service.process(25);
  }

  describe(): string {
    return this.service.format("module_25");
  }
}
