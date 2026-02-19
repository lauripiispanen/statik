export function utilFunc_22_a(x: number): number {
  return x * 22;
}

export function utilFunc_22_b(s: string): string {
  return s + "_22";
}

export function utilFunc_22_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 22;
}

// Dead function
export function utilFunc_22_dead(): void {
  console.log("dead code in util_22");
}
