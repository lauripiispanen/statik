export function utilFunc_72_a(x: number): number {
  return x * 72;
}

export function utilFunc_72_b(s: string): string {
  return s + "_72";
}

export function utilFunc_72_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 72;
}

// Dead function
export function utilFunc_72_dead(): void {
  console.log("dead code in util_72");
}
