export function specialThing(): number {
  return 42;
}

export function unusedSpecialFn(): void {
  // This should be detected as a dead export
}
