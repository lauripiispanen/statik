export function utilFunc_26_a(x: number): number {
  return x * 26;
}

export function utilFunc_26_b(s: string): string {
  return s + "_26";
}

export function utilFunc_26_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 26;
}

// Dead function
export function utilFunc_26_dead(): void {
  console.log("dead code in util_26");
}
