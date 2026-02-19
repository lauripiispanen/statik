import { ESM_CONSTANT } from "./esmFile";

const lodash = require("lodash");

export function processData(items: unknown[]): unknown[] {
  return lodash.uniqBy(items, "id").slice(0, ESM_CONSTANT);
}

export = { processData };
