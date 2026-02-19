import { Service_88 } from "../services/service_88";

export class Module_88 {
  private service: Service_88;

  constructor() {
    this.service = new Service_88();
  }

  run(): number {
    return this.service.process(88);
  }

  describe(): string {
    return this.service.format("module_88");
  }
}
