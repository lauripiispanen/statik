import { Service_58 } from "../services/service_58";

export class Module_58 {
  private service: Service_58;

  constructor() {
    this.service = new Service_58();
  }

  run(): number {
    return this.service.process(58);
  }

  describe(): string {
    return this.service.format("module_58");
  }
}
