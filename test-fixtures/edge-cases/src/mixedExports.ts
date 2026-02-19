export const CONSTANT_A = "a";

export const CONSTANT_B = "b";

export class MixedClass {
  doSomething(): void {
    console.log("doing something");
    this.privateHelper();
  }

  private privateHelper(): void {
    console.log("private helper");
  }
}

export class AnotherClass {
  value: string = "test";
}

function internalFunc(): string {
  return "internal";
}

export { internalFunc };

export default function defaultFunc(): void {
  console.log("default function");
}

export type MixedType = {
  key: string;
  value: number;
};

export type UnusedType = {
  id: number;
};
