export function utilFunc_80_a(x: number): number {
  return x * 80;
}

export function utilFunc_80_b(s: string): string {
  return s + "_80";
}

export function utilFunc_80_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 80;
}

// Dead function
export function utilFunc_80_dead(): void {
  console.log("dead code in util_80");
}
