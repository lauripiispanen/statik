export function utilFunc_78_a(x: number): number {
  return x * 78;
}

export function utilFunc_78_b(s: string): string {
  return s + "_78";
}

export function utilFunc_78_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 78;
}

// Dead function
export function utilFunc_78_dead(): void {
  console.log("dead code in util_78");
}
