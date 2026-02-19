export function utilFunc_17_a(x: number): number {
  return x * 17;
}

export function utilFunc_17_b(s: string): string {
  return s + "_17";
}

export function utilFunc_17_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 17;
}

// Dead function
export function utilFunc_17_dead(): void {
  console.log("dead code in util_17");
}
