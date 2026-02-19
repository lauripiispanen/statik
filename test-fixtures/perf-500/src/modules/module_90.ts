import { Service_90 } from "../services/service_90";

export class Module_90 {
  private service: Service_90;

  constructor() {
    this.service = new Service_90();
  }

  run(): number {
    return this.service.process(90);
  }

  describe(): string {
    return this.service.format("module_90");
  }
}
