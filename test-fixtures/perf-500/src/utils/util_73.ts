export function utilFunc_73_a(x: number): number {
  return x * 73;
}

export function utilFunc_73_b(s: string): string {
  return s + "_73";
}

export function utilFunc_73_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 73;
}

// Dead function
export function utilFunc_73_dead(): void {
  console.log("dead code in util_73");
}
