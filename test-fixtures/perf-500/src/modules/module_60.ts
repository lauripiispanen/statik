import { Service_60 } from "../services/service_60";

export class Module_60 {
  private service: Service_60;

  constructor() {
    this.service = new Service_60();
  }

  run(): number {
    return this.service.process(60);
  }

  describe(): string {
    return this.service.format("module_60");
  }
}
