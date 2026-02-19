export function utilFunc_25_a(x: number): number {
  return x * 25;
}

export function utilFunc_25_b(s: string): string {
  return s + "_25";
}

export function utilFunc_25_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 25;
}

// Dead function
export function utilFunc_25_dead(): void {
  console.log("dead code in util_25");
}
