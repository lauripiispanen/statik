export function utilFunc_13_a(x: number): number {
  return x * 13;
}

export function utilFunc_13_b(s: string): string {
  return s + "_13";
}

export function utilFunc_13_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 13;
}

// Dead function
export function utilFunc_13_dead(): void {
  console.log("dead code in util_13");
}
