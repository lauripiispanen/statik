export function utilFunc_50_a(x: number): number {
  return x * 50;
}

export function utilFunc_50_b(s: string): string {
  return s + "_50";
}

export function utilFunc_50_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 50;
}

// Dead function
export function utilFunc_50_dead(): void {
  console.log("dead code in util_50");
}
