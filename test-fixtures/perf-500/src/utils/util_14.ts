export function utilFunc_14_a(x: number): number {
  return x * 14;
}

export function utilFunc_14_b(s: string): string {
  return s + "_14";
}

export function utilFunc_14_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 14;
}

// Dead function
export function utilFunc_14_dead(): void {
  console.log("dead code in util_14");
}
