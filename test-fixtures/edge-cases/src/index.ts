// Entry point exercising various edge cases
import { default as MyClass } from "./defaultExport";
import * as ns from "./namespaceImport";
import { renamed as originalName } from "./renameExport";
import { Config } from "./typeOnlyExport";
import { processItems } from "./overloads";
import { Color } from "./constEnum";
import { createFactory } from "./higherOrder";
import { MixedClass } from "./mixedExports";
import "./sideEffects"; // side-effect only import

// Use the imports
const obj = new MyClass("test");
obj.greet();

ns.helperA();
ns.helperB();

console.log(originalName);

const config: Config = { host: "localhost", port: 3000 };
console.log(config);

processItems([1, 2, 3]);
processItems(["a", "b"]);

console.log(Color.Red, Color.Green, Color.Blue);

const factory = createFactory((x: number) => x * 2);
console.log(factory(5));

const mixed = new MixedClass();
mixed.doSomething();
