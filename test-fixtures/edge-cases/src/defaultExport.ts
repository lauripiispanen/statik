export default class Greeter {
  private name: string;

  constructor(name: string) {
    this.name = name;
  }

  greet(): string {
    return `Hello, ${this.name}!`;
  }

  farewell(): string {
    return `Goodbye, ${this.name}!`;
  }
}

export function createGreeter(name: string): Greeter {
  return new Greeter(name);
}

export const DEFAULT_NAME = "World";
