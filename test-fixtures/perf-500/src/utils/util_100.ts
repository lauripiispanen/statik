export function utilFunc_100_a(x: number): number {
  return x * 100;
}

export function utilFunc_100_b(s: string): string {
  return s + "_100";
}

export function utilFunc_100_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 100;
}

// Dead function
export function utilFunc_100_dead(): void {
  console.log("dead code in util_100");
}
