export function utilFunc_37_a(x: number): number {
  return x * 37;
}

export function utilFunc_37_b(s: string): string {
  return s + "_37";
}

export function utilFunc_37_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 37;
}

// Dead function
export function utilFunc_37_dead(): void {
  console.log("dead code in util_37");
}
