import { Service_97 } from "../services/service_97";

export class Module_97 {
  private service: Service_97;

  constructor() {
    this.service = new Service_97();
  }

  run(): number {
    return this.service.process(97);
  }

  describe(): string {
    return this.service.format("module_97");
  }
}
