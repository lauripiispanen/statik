export function utilFunc_70_a(x: number): number {
  return x * 70;
}

export function utilFunc_70_b(s: string): string {
  return s + "_70";
}

export function utilFunc_70_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 70;
}

// Dead function
export function utilFunc_70_dead(): void {
  console.log("dead code in util_70");
}
