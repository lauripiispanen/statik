export function utilFunc_90_a(x: number): number {
  return x * 90;
}

export function utilFunc_90_b(s: string): string {
  return s + "_90";
}

export function utilFunc_90_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 90;
}

// Dead function
export function utilFunc_90_dead(): void {
  console.log("dead code in util_90");
}
