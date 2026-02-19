export function utilFunc_10_a(x: number): number {
  return x * 10;
}

export function utilFunc_10_b(s: string): string {
  return s + "_10";
}

export function utilFunc_10_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 10;
}

// Dead function
export function utilFunc_10_dead(): void {
  console.log("dead code in util_10");
}
