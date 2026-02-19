import { formatLogMessage } from "./format";

export type LogLevel = "debug" | "info" | "warn" | "error";

export class Logger {
  private module: string;
  private level: LogLevel;

  constructor(module: string, level: LogLevel = "info") {
    this.module = module;
    this.level = level;
  }

  debug(message: string): void {
    if (this.shouldLog("debug")) {
      console.log(formatLogMessage("debug", this.module, message));
    }
  }

  info(message: string): void {
    if (this.shouldLog("info")) {
      console.log(formatLogMessage("info", this.module, message));
    }
  }

  warn(message: string): void {
    if (this.shouldLog("warn")) {
      console.warn(formatLogMessage("warn", this.module, message));
    }
  }

  error(message: string, error?: Error): void {
    if (this.shouldLog("error")) {
      console.error(formatLogMessage("error", this.module, message));
      if (error) {
        console.error(error.stack);
      }
    }
  }

  private shouldLog(level: LogLevel): boolean {
    const levels: LogLevel[] = ["debug", "info", "warn", "error"];
    return levels.indexOf(level) >= levels.indexOf(this.level);
  }
}

export function createChildLogger(parent: Logger, childModule: string): Logger {
  return new Logger(childModule);
}
