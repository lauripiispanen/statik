import { LogLevel } from "./logger";

export interface Config {
  logLevel: LogLevel;
  precision: number;
}

export function loadConfig(): Config {
  return {
    logLevel: LogLevel.Debug,
    precision: 2,
  };
}
