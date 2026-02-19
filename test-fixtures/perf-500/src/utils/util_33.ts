export function utilFunc_33_a(x: number): number {
  return x * 33;
}

export function utilFunc_33_b(s: string): string {
  return s + "_33";
}

export function utilFunc_33_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 33;
}

// Dead function
export function utilFunc_33_dead(): void {
  console.log("dead code in util_33");
}
