export function utilFunc_64_a(x: number): number {
  return x * 64;
}

export function utilFunc_64_b(s: string): string {
  return s + "_64";
}

export function utilFunc_64_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 64;
}

// Dead function
export function utilFunc_64_dead(): void {
  console.log("dead code in util_64");
}
