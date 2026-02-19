import { Service_27 } from "../services/service_27";

export class Module_27 {
  private service: Service_27;

  constructor() {
    this.service = new Service_27();
  }

  run(): number {
    return this.service.process(27);
  }

  describe(): string {
    return this.service.format("module_27");
  }
}
