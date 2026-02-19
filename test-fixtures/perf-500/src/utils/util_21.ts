export function utilFunc_21_a(x: number): number {
  return x * 21;
}

export function utilFunc_21_b(s: string): string {
  return s + "_21";
}

export function utilFunc_21_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 21;
}

// Dead function
export function utilFunc_21_dead(): void {
  console.log("dead code in util_21");
}
