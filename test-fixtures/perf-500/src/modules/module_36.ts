import { Service_36 } from "../services/service_36";

export class Module_36 {
  private service: Service_36;

  constructor() {
    this.service = new Service_36();
  }

  run(): number {
    return this.service.process(36);
  }

  describe(): string {
    return this.service.format("module_36");
  }
}
