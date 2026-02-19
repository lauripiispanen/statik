import { Service_83 } from "../services/service_83";

export class Module_83 {
  private service: Service_83;

  constructor() {
    this.service = new Service_83();
  }

  run(): number {
    return this.service.process(83);
  }

  describe(): string {
    return this.service.format("module_83");
  }
}
