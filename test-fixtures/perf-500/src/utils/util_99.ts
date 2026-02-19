export function utilFunc_99_a(x: number): number {
  return x * 99;
}

export function utilFunc_99_b(s: string): string {
  return s + "_99";
}

export function utilFunc_99_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 99;
}

// Dead function
export function utilFunc_99_dead(): void {
  console.log("dead code in util_99");
}
