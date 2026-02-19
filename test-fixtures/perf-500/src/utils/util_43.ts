export function utilFunc_43_a(x: number): number {
  return x * 43;
}

export function utilFunc_43_b(s: string): string {
  return s + "_43";
}

export function utilFunc_43_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 43;
}

// Dead function
export function utilFunc_43_dead(): void {
  console.log("dead code in util_43");
}
