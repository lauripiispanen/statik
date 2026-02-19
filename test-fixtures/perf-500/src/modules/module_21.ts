import { Service_21 } from "../services/service_21";

export class Module_21 {
  private service: Service_21;

  constructor() {
    this.service = new Service_21();
  }

  run(): number {
    return this.service.process(21);
  }

  describe(): string {
    return this.service.format("module_21");
  }
}
