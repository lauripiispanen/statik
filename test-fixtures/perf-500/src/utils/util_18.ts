export function utilFunc_18_a(x: number): number {
  return x * 18;
}

export function utilFunc_18_b(s: string): string {
  return s + "_18";
}

export function utilFunc_18_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 18;
}

// Dead function
export function utilFunc_18_dead(): void {
  console.log("dead code in util_18");
}
