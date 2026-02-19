import { Logger, LogLevel } from "./logger";

export class Calculator {
  private logger: Logger;

  constructor(logger: Logger) {
    this.logger = logger;
  }

  add(a: number, b: number): number {
    this.logger.log(LogLevel.Debug, `add(${a}, ${b})`);
    return a + b;
  }

  multiply(a: number, b: number): number {
    this.logger.log(LogLevel.Debug, `multiply(${a}, ${b})`);
    return a * b;
  }

  divide(a: number, b: number): number {
    if (b === 0) {
      this.logger.log(LogLevel.Error, "Division by zero");
      throw new Error("Division by zero");
    }
    this.logger.log(LogLevel.Debug, `divide(${a}, ${b})`);
    return a / b;
  }
}
