import { Service_73 } from "../services/service_73";

export class Module_73 {
  private service: Service_73;

  constructor() {
    this.service = new Service_73();
  }

  run(): number {
    return this.service.process(73);
  }

  describe(): string {
    return this.service.format("module_73");
  }
}
