export function utilFunc_41_a(x: number): number {
  return x * 41;
}

export function utilFunc_41_b(s: string): string {
  return s + "_41";
}

export function utilFunc_41_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 41;
}

// Dead function
export function utilFunc_41_dead(): void {
  console.log("dead code in util_41");
}
