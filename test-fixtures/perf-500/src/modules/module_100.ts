import { Service_100 } from "../services/service_100";

export class Module_100 {
  private service: Service_100;

  constructor() {
    this.service = new Service_100();
  }

  run(): number {
    return this.service.process(100);
  }

  describe(): string {
    return this.service.format("module_100");
  }
}
