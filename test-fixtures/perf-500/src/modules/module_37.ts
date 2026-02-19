import { Service_37 } from "../services/service_37";

export class Module_37 {
  private service: Service_37;

  constructor() {
    this.service = new Service_37();
  }

  run(): number {
    return this.service.process(37);
  }

  describe(): string {
    return this.service.format("module_37");
  }
}
