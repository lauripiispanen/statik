export function utilFunc_23_a(x: number): number {
  return x * 23;
}

export function utilFunc_23_b(s: string): string {
  return s + "_23";
}

export function utilFunc_23_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 23;
}

// Dead function
export function utilFunc_23_dead(): void {
  console.log("dead code in util_23");
}
