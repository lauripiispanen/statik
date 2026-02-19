export function utilFunc_2_a(x: number): number {
  return x * 2;
}

export function utilFunc_2_b(s: string): string {
  return s + "_2";
}

export function utilFunc_2_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 2;
}

// Dead function
export function utilFunc_2_dead(): void {
  console.log("dead code in util_2");
}
