export function utilFunc_34_a(x: number): number {
  return x * 34;
}

export function utilFunc_34_b(s: string): string {
  return s + "_34";
}

export function utilFunc_34_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 34;
}

// Dead function
export function utilFunc_34_dead(): void {
  console.log("dead code in util_34");
}
