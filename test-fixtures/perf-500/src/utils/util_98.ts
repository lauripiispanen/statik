export function utilFunc_98_a(x: number): number {
  return x * 98;
}

export function utilFunc_98_b(s: string): string {
  return s + "_98";
}

export function utilFunc_98_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 98;
}

// Dead function
export function utilFunc_98_dead(): void {
  console.log("dead code in util_98");
}
