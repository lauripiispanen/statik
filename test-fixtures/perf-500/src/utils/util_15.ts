export function utilFunc_15_a(x: number): number {
  return x * 15;
}

export function utilFunc_15_b(s: string): string {
  return s + "_15";
}

export function utilFunc_15_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 15;
}

// Dead function
export function utilFunc_15_dead(): void {
  console.log("dead code in util_15");
}
