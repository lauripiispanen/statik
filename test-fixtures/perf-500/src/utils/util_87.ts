export function utilFunc_87_a(x: number): number {
  return x * 87;
}

export function utilFunc_87_b(s: string): string {
  return s + "_87";
}

export function utilFunc_87_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 87;
}

// Dead function
export function utilFunc_87_dead(): void {
  console.log("dead code in util_87");
}
