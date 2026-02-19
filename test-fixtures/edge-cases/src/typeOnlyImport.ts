import type { Config } from "./typeOnlyExport";
import { DEFAULT_CONFIG } from "./typeOnlyExport";

function printConfig(config: Config): void {
  console.log(`Host: ${config.host}, Port: ${config.port}`);
}

printConfig(DEFAULT_CONFIG);
