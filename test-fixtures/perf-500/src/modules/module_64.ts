import { Service_64 } from "../services/service_64";

export class Module_64 {
  private service: Service_64;

  constructor() {
    this.service = new Service_64();
  }

  run(): number {
    return this.service.process(64);
  }

  describe(): string {
    return this.service.format("module_64");
  }
}
