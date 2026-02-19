export function utilFunc_12_a(x: number): number {
  return x * 12;
}

export function utilFunc_12_b(s: string): string {
  return s + "_12";
}

export function utilFunc_12_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 12;
}

// Dead function
export function utilFunc_12_dead(): void {
  console.log("dead code in util_12");
}
