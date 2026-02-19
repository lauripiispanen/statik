export function utilFunc_42_a(x: number): number {
  return x * 42;
}

export function utilFunc_42_b(s: string): string {
  return s + "_42";
}

export function utilFunc_42_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 42;
}

// Dead function
export function utilFunc_42_dead(): void {
  console.log("dead code in util_42");
}
