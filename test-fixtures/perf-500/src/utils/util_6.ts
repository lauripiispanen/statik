export function utilFunc_6_a(x: number): number {
  return x * 6;
}

export function utilFunc_6_b(s: string): string {
  return s + "_6";
}

export function utilFunc_6_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 6;
}

// Dead function
export function utilFunc_6_dead(): void {
  console.log("dead code in util_6");
}
