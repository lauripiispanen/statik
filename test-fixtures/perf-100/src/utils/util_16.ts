export function utilFunc_16_a(x: number): number {
  return x * 16;
}

export function utilFunc_16_b(s: string): string {
  return s + "_16";
}

export function utilFunc_16_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 16;
}

// Dead function
export function utilFunc_16_dead(): void {
  console.log("dead code in util_16");
}
