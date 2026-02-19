import { Service_71 } from "../services/service_71";

export class Module_71 {
  private service: Service_71;

  constructor() {
    this.service = new Service_71();
  }

  run(): number {
    return this.service.process(71);
  }

  describe(): string {
    return this.service.format("module_71");
  }
}
