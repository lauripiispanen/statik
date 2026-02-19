export function processItems(items: number[]): number;
export function processItems(items: string[]): string;
export function processItems(items: (number | string)[]): number | string {
  if (typeof items[0] === "number") {
    return (items as number[]).reduce((a, b) => a + b, 0);
  }
  return (items as string[]).join(", ");
}

export class Parser {
  parse(input: string): object;
  parse(input: Buffer): object;
  parse(input: string | Buffer): object {
    const str = typeof input === "string" ? input : input.toString();
    return JSON.parse(str);
  }
}
