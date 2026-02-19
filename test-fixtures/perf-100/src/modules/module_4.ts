import { Service_4 } from "../services/service_4";

export class Module_4 {
  private service: Service_4;

  constructor() {
    this.service = new Service_4();
  }

  run(): number {
    return this.service.process(4);
  }

  describe(): string {
    return this.service.format("module_4");
  }
}
