export function utilFunc_27_a(x: number): number {
  return x * 27;
}

export function utilFunc_27_b(s: string): string {
  return s + "_27";
}

export function utilFunc_27_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 27;
}

// Dead function
export function utilFunc_27_dead(): void {
  console.log("dead code in util_27");
}
