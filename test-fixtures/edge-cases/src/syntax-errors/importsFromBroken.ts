import { validFunction } from "./validFile";
import { almostValid } from "./brokenSyntax";

export function useBoth(): string {
  return validFunction() + almostValid();
}
