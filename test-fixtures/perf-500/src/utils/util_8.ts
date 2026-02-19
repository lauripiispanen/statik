export function utilFunc_8_a(x: number): number {
  return x * 8;
}

export function utilFunc_8_b(s: string): string {
  return s + "_8";
}

export function utilFunc_8_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 8;
}

// Dead function
export function utilFunc_8_dead(): void {
  console.log("dead code in util_8");
}
