import { Service_91 } from "../services/service_91";

export class Module_91 {
  private service: Service_91;

  constructor() {
    this.service = new Service_91();
  }

  run(): number {
    return this.service.process(91);
  }

  describe(): string {
    return this.service.format("module_91");
  }
}
