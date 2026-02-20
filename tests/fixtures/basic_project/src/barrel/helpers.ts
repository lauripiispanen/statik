export function barrelHelper(): string {
  return "from barrel helper";
}

export function unusedBarrelFn(): void {
  // This should be detected as a dead export
}
