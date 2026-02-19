export function utilFunc_11_a(x: number): number {
  return x * 11;
}

export function utilFunc_11_b(s: string): string {
  return s + "_11";
}

export function utilFunc_11_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 11;
}

// Dead function
export function utilFunc_11_dead(): void {
  console.log("dead code in util_11");
}
