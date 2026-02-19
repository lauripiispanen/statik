export enum LogLevel {
  Debug = "DEBUG",
  Info = "INFO",
  Warn = "WARN",
  Error = "ERROR",
}

export class Logger {
  private minLevel: LogLevel;

  constructor(minLevel: LogLevel = LogLevel.Info) {
    this.minLevel = minLevel;
  }

  log(level: LogLevel, message: string): void {
    const levels = [LogLevel.Debug, LogLevel.Info, LogLevel.Warn, LogLevel.Error];
    if (levels.indexOf(level) >= levels.indexOf(this.minLevel)) {
      console.log(`[${level}] ${message}`);
    }
  }
}
