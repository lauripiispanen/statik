export function utilFunc_5_a(x: number): number {
  return x * 5;
}

export function utilFunc_5_b(s: string): string {
  return s + "_5";
}

export function utilFunc_5_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 5;
}

// Dead function
export function utilFunc_5_dead(): void {
  console.log("dead code in util_5");
}
