import { Service_13 } from "../services/service_13";

export class Module_13 {
  private service: Service_13;

  constructor() {
    this.service = new Service_13();
  }

  run(): number {
    return this.service.process(13);
  }

  describe(): string {
    return this.service.format("module_13");
  }
}
