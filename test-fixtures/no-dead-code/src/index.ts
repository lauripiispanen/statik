import { Calculator } from "./calculator";
import { Logger, LogLevel } from "./logger";
import { formatResult, formatError } from "./formatter";
import { Config, loadConfig } from "./config";

const config: Config = loadConfig();
const logger = new Logger(config.logLevel);
const calc = new Calculator(logger);

function main() {
  logger.log(LogLevel.Info, "Starting calculator");

  const sum = calc.add(10, 20);
  console.log(formatResult("add", sum));

  const product = calc.multiply(5, 6);
  console.log(formatResult("multiply", product));

  try {
    calc.divide(10, 0);
  } catch (e) {
    console.log(formatError(e as Error));
  }

  logger.log(LogLevel.Info, "Done");
}

main();
