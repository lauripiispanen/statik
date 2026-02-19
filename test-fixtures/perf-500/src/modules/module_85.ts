import { Service_85 } from "../services/service_85";

export class Module_85 {
  private service: Service_85;

  constructor() {
    this.service = new Service_85();
  }

  run(): number {
    return this.service.process(85);
  }

  describe(): string {
    return this.service.format("module_85");
  }
}
