import { Service_8 } from "../services/service_8";

export class Module_8 {
  private service: Service_8;

  constructor() {
    this.service = new Service_8();
  }

  run(): number {
    return this.service.process(8);
  }

  describe(): string {
    return this.service.format("module_8");
  }
}
