import { Service_98 } from "../services/service_98";

export class Module_98 {
  private service: Service_98;

  constructor() {
    this.service = new Service_98();
  }

  run(): number {
    return this.service.process(98);
  }

  describe(): string {
    return this.service.format("module_98");
  }
}
