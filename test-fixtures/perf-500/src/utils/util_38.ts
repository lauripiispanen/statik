export function utilFunc_38_a(x: number): number {
  return x * 38;
}

export function utilFunc_38_b(s: string): string {
  return s + "_38";
}

export function utilFunc_38_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 38;
}

// Dead function
export function utilFunc_38_dead(): void {
  console.log("dead code in util_38");
}
