export function utilFunc_7_a(x: number): number {
  return x * 7;
}

export function utilFunc_7_b(s: string): string {
  return s + "_7";
}

export function utilFunc_7_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 7;
}

// Dead function
export function utilFunc_7_dead(): void {
  console.log("dead code in util_7");
}
