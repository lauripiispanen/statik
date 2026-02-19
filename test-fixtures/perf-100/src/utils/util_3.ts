export function utilFunc_3_a(x: number): number {
  return x * 3;
}

export function utilFunc_3_b(s: string): string {
  return s + "_3";
}

export function utilFunc_3_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 3;
}

// Dead function
export function utilFunc_3_dead(): void {
  console.log("dead code in util_3");
}
