import { Service_93 } from "../services/service_93";

export class Module_93 {
  private service: Service_93;

  constructor() {
    this.service = new Service_93();
  }

  run(): number {
    return this.service.process(93);
  }

  describe(): string {
    return this.service.format("module_93");
  }
}
