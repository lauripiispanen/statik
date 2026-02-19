export function utilFunc_1_a(x: number): number {
  return x * 1;
}

export function utilFunc_1_b(s: string): string {
  return s + "_1";
}

export function utilFunc_1_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 1;
}

// Dead function
export function utilFunc_1_dead(): void {
  console.log("dead code in util_1");
}
