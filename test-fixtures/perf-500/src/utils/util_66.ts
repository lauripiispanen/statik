export function utilFunc_66_a(x: number): number {
  return x * 66;
}

export function utilFunc_66_b(s: string): string {
  return s + "_66";
}

export function utilFunc_66_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 66;
}

// Dead function
export function utilFunc_66_dead(): void {
  console.log("dead code in util_66");
}
