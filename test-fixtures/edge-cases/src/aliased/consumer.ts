import {
  originalName as renamedFunc,
  anotherExport as getNumber,
  OriginalClass as RenamedClass,
} from "./source";

const result = renamedFunc();
const num = getNumber();
const instance = new RenamedClass();

console.log(result, num, instance.getValue());
