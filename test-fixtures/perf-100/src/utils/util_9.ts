export function utilFunc_9_a(x: number): number {
  return x * 9;
}

export function utilFunc_9_b(s: string): string {
  return s + "_9";
}

export function utilFunc_9_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 9;
}

// Dead function
export function utilFunc_9_dead(): void {
  console.log("dead code in util_9");
}
