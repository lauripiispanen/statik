export interface Config {
  host: string;
  port: number;
  debug?: boolean;
}

export type ConfigKey = keyof Config;

export interface DatabaseConfig extends Config {
  connectionString: string;
  pool: number;
}

export const DEFAULT_CONFIG: Config = {
  host: "localhost",
  port: 8080,
};

export function mergeConfigs(base: Config, override: Partial<Config>): Config {
  return { ...base, ...override };
}
