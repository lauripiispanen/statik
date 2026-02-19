import { Service_86 } from "../services/service_86";

export class Module_86 {
  private service: Service_86;

  constructor() {
    this.service = new Service_86();
  }

  run(): number {
    return this.service.process(86);
  }

  describe(): string {
    return this.service.format("module_86");
  }
}
