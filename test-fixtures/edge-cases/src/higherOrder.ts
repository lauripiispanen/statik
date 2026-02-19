type Transform<T> = (input: T) => T;

export function createFactory<T>(transform: Transform<T>): (input: T) => T {
  return (input: T) => {
    console.log("Factory processing:", input);
    return transform(input);
  };
}

export function compose<T>(...fns: Transform<T>[]): Transform<T> {
  return (input: T) => fns.reduceRight((acc, fn) => fn(acc), input);
}

export function pipe<T>(...fns: Transform<T>[]): Transform<T> {
  return (input: T) => fns.reduce((acc, fn) => fn(acc), input);
}

export function memoize<T extends (...args: unknown[]) => unknown>(fn: T): T {
  const cache = new Map<string, ReturnType<T>>();
  return ((...args: unknown[]) => {
    const key = JSON.stringify(args);
    if (cache.has(key)) return cache.get(key)!;
    const result = fn(...args) as ReturnType<T>;
    cache.set(key, result);
    return result;
  }) as T;
}
