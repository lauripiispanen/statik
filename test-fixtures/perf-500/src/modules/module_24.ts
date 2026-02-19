import { Service_24 } from "../services/service_24";

export class Module_24 {
  private service: Service_24;

  constructor() {
    this.service = new Service_24();
  }

  run(): number {
    return this.service.process(24);
  }

  describe(): string {
    return this.service.format("module_24");
  }
}
