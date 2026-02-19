import { Service_81 } from "../services/service_81";

export class Module_81 {
  private service: Service_81;

  constructor() {
    this.service = new Service_81();
  }

  run(): number {
    return this.service.process(81);
  }

  describe(): string {
    return this.service.format("module_81");
  }
}
