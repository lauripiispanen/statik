import { Service_2 } from "../services/service_2";

export class Module_2 {
  private service: Service_2;

  constructor() {
    this.service = new Service_2();
  }

  run(): number {
    return this.service.process(2);
  }

  describe(): string {
    return this.service.format("module_2");
  }
}
