import { esmHelper } from "./esmFile";

const path = require("path");
const fs = require("fs");

export function loadConfig(dir: string): object {
  const configPath = path.join(dir, "config.json");
  if (fs.existsSync(configPath)) {
    return JSON.parse(fs.readFileSync(configPath, "utf8"));
  }
  return { fallback: esmHelper() };
}

module.exports = { loadConfig };
