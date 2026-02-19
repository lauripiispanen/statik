export function utilFunc_28_a(x: number): number {
  return x * 28;
}

export function utilFunc_28_b(s: string): string {
  return s + "_28";
}

export function utilFunc_28_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 28;
}

// Dead function
export function utilFunc_28_dead(): void {
  console.log("dead code in util_28");
}
