import { Service_84 } from "../services/service_84";

export class Module_84 {
  private service: Service_84;

  constructor() {
    this.service = new Service_84();
  }

  run(): number {
    return this.service.process(84);
  }

  describe(): string {
    return this.service.format("module_84");
  }
}
