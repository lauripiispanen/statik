import { Service_9 } from "../services/service_9";

export class Module_9 {
  private service: Service_9;

  constructor() {
    this.service = new Service_9();
  }

  run(): number {
    return this.service.process(9);
  }

  describe(): string {
    return this.service.format("module_9");
  }
}
