export function utilFunc_96_a(x: number): number {
  return x * 96;
}

export function utilFunc_96_b(s: string): string {
  return s + "_96";
}

export function utilFunc_96_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + 96;
}

// Dead function
export function utilFunc_96_dead(): void {
  console.log("dead code in util_96");
}
