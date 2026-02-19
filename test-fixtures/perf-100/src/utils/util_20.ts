export function utilFunc_20_a(x: number): number {
  return x * 20;
}

export function utilFunc_20_b(s: string): string {
  return s + "_20";
}

export function utilFunc_20_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 20;
}

// Dead function
export function utilFunc_20_dead(): void {
  console.log("dead code in util_20");
}
