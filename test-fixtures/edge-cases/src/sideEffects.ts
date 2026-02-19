console.log("Side effects module loaded");

if (!Array.prototype.flat) {
  // @ts-ignore
  Array.prototype.flat = function (depth = 1) {
    return this.reduce(
      (acc: unknown[], val: unknown) =>
        acc.concat(
          depth > 0 && Array.isArray(val) ? (val as unknown[]).flat(depth - 1) : val
        ),
      []
    );
  };
}

const registry = new Map<string, unknown>();

export function register(name: string, value: unknown): void {
  registry.set(name, value);
}

export function get(name: string): unknown {
  return registry.get(name);
}
